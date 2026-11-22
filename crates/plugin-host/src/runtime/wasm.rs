use std::hash::{DefaultHasher, Hash, Hasher};
use std::path::Path;
use std::sync::OnceLock;
use std::time::{Duration, Instant};

use craft_plugin::{ABI, Outcome, Pixel};
use std::sync::Arc;

use wasmtime::{Caller, Config, Engine, Extern, Linker, Memory, Module, Store, StoreLimits, StoreLimitsBuilder, TypedFunc, UpdateDeadline};

use rayon::prelude::*;

use super::native;
use super::serve::HostCx;
use super::tasks::Slot;
use crate::plugin::{Io, PluginError};

const TICK: Duration = Duration::from_millis(50);
const MEMORY_LIMIT: usize = 3 << 30;

type CallFn = TypedFunc<(u32, u32, u32, u32, u32, u32), ()>;
type AllocFn = TypedFunc<u32, u32>;
type FreeFn = TypedFunc<(u32, u32), ()>;

struct State {
    limits: StoreLimits,
    ctx: usize,
    alloc: Option<AllocFn>,
    deadline: Option<Instant>,
}

fn engine() -> Result<&'static Engine, PluginError> {
    static ENGINE: OnceLock<Result<Engine, String>> = OnceLock::new();
    ENGINE
        .get_or_init(|| {
            let mut config = Config::new();
            config.epoch_interruption(true);
            let engine = Engine::new(&config).map_err(|e| e.to_string())?;
            let clock = engine.weak();
            std::thread::Builder::new()
                .name("craft-plugin-clock".into())
                .spawn(move || {
                    while let Some(engine) = clock.upgrade() {
                        engine.increment_epoch();
                        std::thread::sleep(TICK);
                    }
                })
                .map_err(|e| e.to_string())?;
            Ok(engine)
        })
        .as_ref()
        .map_err(|e| PluginError::Load(e.clone()))
}

pub(crate) struct WasmModule {
    module: Module,
    linker: Linker<State>,
}

impl WasmModule {
    pub fn compile(bytes: &[u8], cache: &Path) -> Result<WasmModule, PluginError> {
        let engine = engine()?;
        let module = cached(engine, bytes, cache)?;
        let mut linker = Linker::new(engine);
        linker.func_wrap("craft", "host_call", host_call).map_err(load_error)?;
        Ok(WasmModule { module, linker })
    }

    pub fn instantiate(&self) -> Result<WasmGuest, PluginError> {
        let limits = StoreLimitsBuilder::new().memory_size(MEMORY_LIMIT).memories(1).instances(1).build();
        let mut store = Store::new(self.module.engine(), State { limits, ctx: 0, alloc: None, deadline: None });
        store.limiter(|state| &mut state.limits);
        store.set_epoch_deadline(ticks(Duration::from_secs(5)));
        let instance = self.linker.instantiate(&mut store, &self.module).map_err(load_error)?;
        let memory = instance.get_memory(&mut store, "memory").ok_or_else(|| missing("memory"))?;
        let abi: TypedFunc<(), u32> = instance.get_typed_func(&mut store, "craft_plugin_abi").map_err(load_error)?;
        let found = abi.call(&mut store, ()).map_err(load_error)?;
        if found != ABI {
            return Err(PluginError::Load(format!("plugin speaks ABI {found}, this app speaks ABI {ABI}")));
        }
        let call: CallFn = instance.get_typed_func(&mut store, "craft_plugin_call").map_err(load_error)?;
        let alloc: AllocFn = instance.get_typed_func(&mut store, "craft_plugin_alloc").map_err(load_error)?;
        let free: FreeFn = instance.get_typed_func(&mut store, "craft_plugin_free").map_err(load_error)?;
        store.data_mut().alloc = Some(alloc.clone());
        let out = alloc.call(&mut store, 8).map_err(load_error)?;
        Ok(WasmGuest { store, memory, call, alloc, free, out, pixels: (0, 0) })
    }
}

pub(crate) struct WasmGuest {
    store: Store<State>,
    memory: Memory,
    call: CallFn,
    alloc: AllocFn,
    free: FreeFn,
    out: u32,
    pixels: (u32, u32),
}

impl WasmGuest {
    pub fn call(&mut self, request: &[u8], io: Io, cx: &mut HostCx, budget: Duration) -> Result<Vec<u8>, PluginError> {
        self.store.set_epoch_deadline(ticks(budget));
        self.run(request, io, cx, Instant::now().checked_add(budget))
    }

    pub fn run_task(&mut self, request: &[u8], cx: &mut HostCx, slot: Arc<Slot>) -> Result<Vec<u8>, PluginError> {
        self.store.epoch_deadline_callback(move |_| {
            if slot.overdue() {
                return Err(wasmtime::format_err!("the task was cancelled"));
            }
            Ok(UpdateDeadline::Continue(1))
        });
        self.store.set_epoch_deadline(1);
        self.run(request, Io::None, cx, None)
    }

    fn run(&mut self, request: &[u8], io: Io, cx: &mut HostCx, deadline: Option<Instant>) -> Result<Vec<u8>, PluginError> {
        self.store.data_mut().ctx = native::erase(cx);
        self.store.data_mut().deadline = deadline;
        let reply = self.exchange(request, io);
        self.store.data_mut().ctx = 0;
        self.store.data_mut().deadline = None;
        reply.map_err(|e| PluginError::Trap(format!("{e:#}")))
    }

    fn exchange(&mut self, request: &[u8], io: Io) -> wasmtime::Result<Vec<u8>> {
        let input: &[Pixel] = match &io {
            Io::None => &[],
            Io::InPlace(pixels) => pixels,
            Io::Region { input, .. } => input,
        };
        let pixel_bytes: &[u8] = bytemuck::cast_slice(input);
        let pixels = self.pixel_buffer(pixel_bytes.len())?;
        let place = self.memory.data_mut(&mut self.store).get_mut(pixels as usize..).and_then(|tail| tail.get_mut(..pixel_bytes.len()));
        copy(place.ok_or_else(|| wasmtime::format_err!("the frame does not fit in plugin memory"))?, pixel_bytes);

        let request_at = self.alloc.call(&mut self.store, len(request)?)?;
        if request_at == 0 {
            wasmtime::bail!("the plugin could not allocate the request");
        }
        self.memory.write(&mut self.store, request_at as usize, request)?;
        self.memory.write(&mut self.store, self.out as usize, &[0; 8])?;
        let args = (0, request_at, len(request)?, pixels, len(pixel_bytes)?, self.out);
        self.call.call(&mut self.store, args)?;
        self.free.call(&mut self.store, (request_at, len(request)?))?;

        let mut out = [0u32; 2];
        self.memory.read(&self.store, self.out as usize, bytemuck::cast_slice_mut(&mut out))?;
        let reply = (out[0] as usize)
            .checked_add(out[1] as usize)
            .and_then(|end| self.memory.data(&self.store).get(out[0] as usize..end))
            .filter(|_| out[0] != 0)
            .map(<[u8]>::to_vec)
            .ok_or_else(|| wasmtime::format_err!("the plugin returned no reply"))?;
        self.free.call(&mut self.store, (out[0], out[1]))?;
        if !succeeded(&reply) {
            return Ok(reply);
        }

        let (output, skip): (&mut [Pixel], usize) = match io {
            Io::None => return Ok(reply),
            Io::InPlace(output) => (output, 0),
            Io::Region { output, skip, .. } => (output, skip),
        };
        let output: &mut [u8] = bytemuck::cast_slice_mut(output);
        let from = pixels as usize + skip * size_of::<Pixel>();
        let rendered = self.memory.data(&self.store).get(from..).and_then(|tail| tail.get(..output.len()));
        copy(output, rendered.ok_or_else(|| wasmtime::format_err!("the frame does not fit in plugin memory"))?);
        Ok(reply)
    }

    fn pixel_buffer(&mut self, bytes: usize) -> wasmtime::Result<u32> {
        let wanted = u32::try_from(bytes)?;
        if self.pixels.1 < wanted {
            if self.pixels.1 > 0 {
                self.free.call(&mut self.store, self.pixels)?;
            }
            self.pixels = (0, 0);
            let at = self.alloc.call(&mut self.store, wanted)?;
            if at == 0 {
                wasmtime::bail!("the plugin could not allocate a {bytes} byte frame");
            }
            self.pixels = (at, wanted);
        }
        Ok(self.pixels.0)
    }
}

fn host_call(mut caller: Caller<'_, State>, _ctx: u32, request: u32, request_len: u32, bulk: u32, bulk_len: u32, out: u32) -> wasmtime::Result<()> {
    let memory = caller.get_export("memory").and_then(Extern::into_memory).ok_or_else(|| wasmtime::format_err!("no memory"))?;
    let slice = |at: u32, len: u32| -> wasmtime::Result<Vec<u8>> {
        let (start, end) = (at as usize, at as usize + len as usize);
        memory.data(&caller).get(start..end).map(<[u8]>::to_vec).ok_or_else(|| wasmtime::format_err!("host call reads outside memory"))
    };
    let request = slice(request, request_len)?;
    let bulk_in = slice(bulk, bulk_len)?;
    let (reply, bulk_out) = native::serve_erased(caller.data().ctx, &request, &bulk_in);
    if caller.data().deadline.is_some_and(|deadline| Instant::now() > deadline) {
        wasmtime::bail!("the plugin passed its time limit while the host served a call");
    }
    let alloc = caller.data().alloc.clone().ok_or_else(|| wasmtime::format_err!("host call before start"))?;
    let mut place = |bytes: &[u8]| -> wasmtime::Result<[u32; 2]> {
        if bytes.is_empty() {
            return Ok([0, 0]);
        }
        let at = alloc.call(&mut caller, len(bytes)?)?;
        if at == 0 {
            wasmtime::bail!("the plugin could not allocate {} bytes", bytes.len());
        }
        memory.write(&mut caller, at as usize, bytes)?;
        Ok([at, len(bytes)?])
    };
    let [reply_at, reply_len] = place(&reply)?;
    let [bulk_at, bulk_len] = place(bulk_out.bytes())?;
    memory.write(&mut caller, out as usize, bytemuck::cast_slice(&[reply_at, reply_len, bulk_at, bulk_len]))?;
    Ok(())
}

fn cached(engine: &Engine, bytes: &[u8], cache: &Path) -> Result<Module, PluginError> {
    let mut hasher = DefaultHasher::new();
    bytes.hash(&mut hasher);
    engine.precompile_compatibility_hash().hash(&mut hasher);
    let path = cache.join(format!("{:016x}.cwasm", hasher.finish()));
    if let Some(module) = native::compiled_module(engine, &path) {
        return Ok(module);
    }
    let compiled = engine.precompile_module(bytes).map_err(load_error)?;
    let module = native::fresh_module(engine, &compiled).map_err(load_error)?;
    let staged = path.with_extension(format!("{}.part", std::process::id()));
    let stored = std::fs::create_dir_all(cache).and_then(|()| std::fs::write(&staged, &compiled)).and_then(|()| std::fs::rename(&staged, &path));
    if let Err(error) = stored {
        log::warn!("{}: the compiled plugin was not cached: {error}", path.display());
        let _ = std::fs::remove_file(&staged);
    }
    Ok(module)
}

fn copy(to: &mut [u8], from: &[u8]) {
    const CHUNK: usize = 1 << 20;
    to.par_chunks_mut(CHUNK).zip(from.par_chunks(CHUNK)).for_each(|(to, from)| to.copy_from_slice(from));
}

fn succeeded(reply: &[u8]) -> bool {
    matches!(serde_json::from_slice::<Outcome<serde::de::IgnoredAny>>(reply), Ok(Outcome::Ok(_)))
}

fn len(bytes: &[u8]) -> wasmtime::Result<u32> {
    Ok(u32::try_from(bytes.len())?)
}

fn ticks(budget: Duration) -> u64 {
    (budget.as_millis() / TICK.as_millis()).max(1) as u64
}

fn load_error(error: wasmtime::Error) -> PluginError {
    PluginError::Load(format!("{error:#}"))
}

fn missing(export: &str) -> PluginError {
    PluginError::Load(format!("the module does not export `{export}`"))
}
