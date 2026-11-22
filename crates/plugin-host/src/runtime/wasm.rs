use std::hash::{Hash, Hasher};
use std::path::Path;
use std::sync::OnceLock;
use std::time::{Duration, Instant};

use craft_plugin::{ABI, Outcome, Pixel};
use std::sync::Arc;

use wasmtime::{AsContextMut, Caller, Config, Engine, Extern, Linker, Memory, Module, Store, StoreLimits, StoreLimitsBuilder, TypedFunc, UpdateDeadline};

use sha2::{Digest, Sha256};

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
        let (inputs, out_bytes): ([&[Pixel]; 2], usize) = match &io {
            Io::None => ([&[], &[]], 0),
            Io::InPlace(pixels) => ([pixels, &[]], 0),
            Io::Region { input, .. } => ([input, &[]], 0),
            Io::Pass { src, aux, out } => ([src, aux], size_of_val::<[Pixel]>(&**out)),
        };
        let [first, second] = inputs.map(bytemuck::cast_slice::<Pixel, u8>);
        let pixel_bytes = first.len() + second.len() + out_bytes;
        let pixels = self.pixel_buffer(pixel_bytes)?;
        let place = self.memory.data_mut(&mut self.store).get_mut(pixels as usize..).and_then(|tail| tail.get_mut(..pixel_bytes));
        let place = place.ok_or_else(|| wasmtime::format_err!("the frame does not fit in plugin memory"))?;
        let (first_place, rest) = place.split_at_mut(first.len());
        first_place.copy_from_slice(first);
        let (second_place, out_place) = rest.split_at_mut(second.len());
        second_place.copy_from_slice(second);
        out_place.fill(0);

        let request_at = self.alloc.call(&mut self.store, len(request)?)?;
        if request_at == 0 {
            wasmtime::bail!("the plugin could not allocate the request");
        }
        self.memory.write(&mut self.store, request_at as usize, request)?;
        self.memory.write(&mut self.store, self.out as usize, &[0; 8])?;
        let args = (0, request_at, len(request)?, pixels, u32::try_from(pixel_bytes)?, self.out);
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
            Io::Pass { src, aux, out } => (out, src.len() + aux.len()),
        };
        let output: &mut [u8] = bytemuck::cast_slice_mut(output);
        let from = pixels as usize + skip * size_of::<Pixel>();
        let rendered = self.memory.data(&self.store).get(from..).and_then(|tail| tail.get(..output.len()));
        output.copy_from_slice(rendered.ok_or_else(|| wasmtime::format_err!("the frame does not fit in plugin memory"))?);
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
    let (reply, bulk_out, paused) = native::serve_erased(caller.data().ctx, &request, &bulk_in, caller.data().deadline);
    if !paused.is_zero()
        && let Some(deadline) = caller.data().deadline.and_then(|deadline| deadline.checked_add(paused))
    {
        caller.data_mut().deadline = Some(deadline);
        caller.as_context_mut().set_epoch_deadline(ticks(deadline.saturating_duration_since(Instant::now())));
    }
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

const KEY_FILE: &str = "key";
const MAC_LEN: usize = 32;

fn cached(engine: &Engine, bytes: &[u8], cache: &Path) -> Result<Module, PluginError> {
    let path = cache.join(format!("{}.cwasm", cache_name(engine, bytes)));
    let key = match install_key(cache) {
        Ok(key) => key,
        Err(error) => {
            log::warn!("{}: the compiled plugin cache is off: {error}", cache.display());
            return engine.precompile_module(bytes).and_then(|compiled| native::fresh_module(engine, &compiled)).map_err(load_error);
        }
    };
    if let Some(module) = std::fs::read(&path).ok().as_deref().and_then(|entry| signed_body(&key, entry)).and_then(|body| native::compiled_module(engine, body))
    {
        return Ok(module);
    }
    let mut compiled = engine.precompile_module(bytes).map_err(load_error)?;
    let module = native::fresh_module(engine, &compiled).map_err(load_error)?;
    compiled.extend_from_slice(&hmac(&key, &compiled));
    let staged = path.with_extension(format!("{}.part", std::process::id()));
    let stored = std::fs::write(&staged, &compiled).and_then(|()| std::fs::rename(&staged, &path));
    if let Err(error) = stored {
        log::warn!("{}: the compiled plugin was not cached: {error}", path.display());
        let _ = std::fs::remove_file(&staged);
    }
    Ok(module)
}

fn signed_body<'a>(key: &[u8; 32], entry: &'a [u8]) -> Option<&'a [u8]> {
    let (body, mac) = entry.split_at_checked(entry.len().checked_sub(MAC_LEN)?)?;
    let expected = hmac(key, body);
    (mac.iter().zip(expected).fold(0, |differ, (a, b)| differ | (a ^ b)) == 0).then_some(body)
}

fn hmac(key: &[u8], message: &[u8]) -> [u8; 32] {
    let mut padded = [0u8; 64];
    padded[..key.len()].copy_from_slice(key);
    let pad = |byte: u8| padded.map(|k| k ^ byte);
    let inner = Sha256::new().chain_update(pad(0x36)).chain_update(message).finalize();
    Sha256::new().chain_update(pad(0x5c)).chain_update(inner).finalize().into()
}

fn install_key(cache: &Path) -> std::io::Result<[u8; 32]> {
    let path = cache.join(KEY_FILE);
    let owner_only = |meta: &std::fs::Metadata| {
        #[cfg(unix)]
        return std::os::unix::fs::PermissionsExt::mode(&meta.permissions()) & 0o077 == 0;
        #[cfg(not(unix))]
        return meta.is_file();
    };
    if let Ok(meta) = std::fs::symlink_metadata(&path)
        && meta.is_file()
        && owner_only(&meta)
        && let Ok(Ok(key)) = std::fs::read(&path).map(<[u8; 32]>::try_from)
    {
        return Ok(key);
    }
    let mut key = [0u8; 32];
    getrandom::fill(&mut key).map_err(|e| std::io::Error::other(e.to_string()))?;
    std::fs::create_dir_all(cache)?;
    let staged = path.with_extension(format!("{}.part", std::process::id()));
    let mut options = std::fs::OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    std::os::unix::fs::OpenOptionsExt::mode(&mut options, 0o600);
    let _ = std::fs::remove_file(&staged);
    std::io::Write::write_all(&mut options.open(&staged)?, &key)?;
    std::fs::rename(&staged, &path)?;
    Ok(key)
}

struct DigestHasher(Sha256);

impl Hasher for DigestHasher {
    fn write(&mut self, bytes: &[u8]) {
        self.0.update(bytes);
    }

    fn finish(&self) -> u64 {
        0
    }
}

fn cache_name(engine: &Engine, bytes: &[u8]) -> String {
    let mut hasher = DigestHasher(Sha256::new());
    hasher.write(bytes);
    hasher.write(std::env::consts::ARCH.as_bytes());
    hasher.write(std::env::consts::OS.as_bytes());
    engine.precompile_compatibility_hash().hash(&mut hasher);
    hasher.0.finalize().iter().map(|byte| format!("{byte:02x}")).collect()
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

#[cfg(test)]
mod tests {
    use super::{KEY_FILE, cache_name, cached, engine, hmac};

    const EMPTY: &[u8] = &[0, 0x61, 0x73, 0x6d, 1, 0, 0, 0];
    const ONE_IMPORT: &[u8] = &[0, 0x61, 0x73, 0x6d, 1, 0, 0, 0, 1, 4, 1, 0x60, 0, 0, 2, 7, 1, 1, b'a', 1, b'b', 0, 0];

    #[test]
    fn hmac_sha256_matches_rfc_4231_test_case_2() {
        let mac = hmac(b"Jefe", b"what do ya want for nothing?");
        let hex: String = mac.iter().map(|byte| format!("{byte:02x}")).collect();
        assert_eq!(hex, "5bdcc146bf60754e6a042426089575c75a003f089d2739839dec58b964ec3843");
    }

    #[test]
    fn a_planted_compiled_module_is_recompiled_and_a_signed_one_is_reused() {
        let engine = engine().unwrap();
        let dir = tempfile::tempdir().unwrap();
        let entry = dir.path().join(format!("{}.cwasm", cache_name(engine, EMPTY)));
        std::fs::write(&entry, engine.precompile_module(ONE_IMPORT).unwrap()).unwrap();
        assert_eq!(cached(engine, EMPTY, dir.path()).unwrap().imports().len(), 0);
        #[cfg(unix)]
        assert_eq!(std::os::unix::fs::PermissionsExt::mode(&std::fs::metadata(dir.path().join(KEY_FILE)).unwrap().permissions()) & 0o777, 0o600);
        let signed = std::fs::read(&entry).unwrap();
        assert_eq!(cached(engine, EMPTY, dir.path()).unwrap().imports().len(), 0);
        assert_eq!(std::fs::read(&entry).unwrap(), signed);
    }
}
