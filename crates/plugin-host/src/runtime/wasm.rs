use std::sync::OnceLock;
use std::time::Duration;

use craft_plugin::{ABI, Pixel};
use wasmtime::{Caller, Config, Engine, Extern, Linker, Memory, Module, Store, StoreLimits, StoreLimitsBuilder, TypedFunc};

use super::native;
use super::serve::HostCx;
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
    pub fn compile(bytes: &[u8]) -> Result<WasmModule, PluginError> {
        let engine = engine()?;
        let module = Module::new(engine, bytes).map_err(load_error)?;
        let mut linker = Linker::new(engine);
        linker.func_wrap("craft", "host_call", host_call).map_err(load_error)?;
        Ok(WasmModule { module, linker })
    }

    pub fn instantiate(&self) -> Result<WasmGuest, PluginError> {
        let limits = StoreLimitsBuilder::new().memory_size(MEMORY_LIMIT).build();
        let mut store = Store::new(self.module.engine(), State { limits, ctx: 0, alloc: None });
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
        self.store.data_mut().ctx = native::erase(cx);
        let reply = self.exchange(request, io);
        self.store.data_mut().ctx = 0;
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
        self.memory.write(&mut self.store, pixels as usize, pixel_bytes)?;

        let request_at = self.alloc.call(&mut self.store, len(request)?)?;
        self.memory.write(&mut self.store, request_at as usize, request)?;
        let args = (0, request_at, len(request)?, pixels, len(pixel_bytes)?, self.out);
        self.call.call(&mut self.store, args)?;
        self.free.call(&mut self.store, (request_at, len(request)?))?;

        let mut out = [0u32; 2];
        self.memory.read(&self.store, self.out as usize, bytemuck::cast_slice_mut(&mut out))?;
        let mut reply = vec![0u8; out[1] as usize];
        self.memory.read(&self.store, out[0] as usize, &mut reply)?;
        self.free.call(&mut self.store, (out[0], out[1]))?;

        match io {
            Io::None => {}
            Io::InPlace(output) => self.memory.read(&self.store, pixels as usize, bytemuck::cast_slice_mut(output))?,
            Io::Region { output, skip, .. } => {
                let from = pixels as usize + skip * size_of::<Pixel>();
                self.memory.read(&self.store, from, bytemuck::cast_slice_mut(output))?
            }
        }
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
