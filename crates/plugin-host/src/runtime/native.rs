#![allow(unsafe_code)]

use std::panic::{AssertUnwindSafe, catch_unwind};
use std::path::Path;
use std::time::Duration;

use craft_plugin::{ABI, Outcome, Pixel};
use libloading::Library;

use super::serve::{Bulk, HostCx, serve};
use crate::plugin::PluginError;

type AbiFn = unsafe extern "C" fn() -> u32;
type AllocFn = unsafe extern "C" fn(usize) -> *mut u8;
type FreeFn = unsafe extern "C" fn(*mut u8, usize);
type CallFn = unsafe extern "C" fn(usize, *const u8, usize, *mut u8, usize, *mut [usize; 2]);
type HostFn = unsafe extern "C" fn(usize, *const u8, usize, *const u8, usize, *mut [usize; 4]);
type InitFn = unsafe extern "C" fn(HostFn);

pub(crate) fn guarded(cx: &mut HostCx, request: &[u8], bulk: &[u8]) -> (Vec<u8>, Bulk) {
    catch_unwind(AssertUnwindSafe(|| serve(cx, request, bulk))).unwrap_or_else(|_| refusal("the app failed to answer"))
}

fn refusal(why: &str) -> (Vec<u8>, Bulk) {
    (serde_json::to_vec(&Outcome::<()>::Err(why.into())).unwrap_or_default(), Bulk::None)
}

pub(crate) fn erase(cx: &mut HostCx) -> usize {
    std::ptr::from_mut(cx) as usize
}

pub(crate) fn serve_erased(ctx: usize, request: &[u8], bulk: &[u8]) -> (Vec<u8>, Bulk, Duration) {
    if ctx == 0 {
        let (reply, bulk) = refusal("host calls are only available during a call from the app");
        return (reply, bulk, Duration::ZERO);
    }
    // SAFETY: `ctx` comes from `erase` on a `HostCx` that the caller of the plugin keeps alive and
    // exclusively borrowed for the whole plugin call; the plugin runs on that same thread.
    let cx = unsafe { &mut *(ctx as *mut HostCx) };
    let (reply, bulk) = guarded(cx, request, bulk);
    (reply, bulk, std::mem::take(&mut cx.paused))
}

pub(crate) fn compiled_module(engine: &wasmtime::Engine, verified: &[u8]) -> Option<wasmtime::Module> {
    // SAFETY: `verified` is a compile-cache entry whose HMAC-SHA-256 under this install's owner-only key matched, so it
    // was written by `wasm::cached` from `Engine::precompile_module`; the entry is named by the SHA-256 of the module
    // bytes and this engine's version, target and settings, in a directory that `serve::resolve` refuses to every plugin
    // file access, and wasmtime rejects bytes whose header does not match this engine's version and settings.
    unsafe { wasmtime::Module::deserialize(engine, verified) }.ok()
}

pub(crate) fn fresh_module(engine: &wasmtime::Engine, compiled: &[u8]) -> wasmtime::Result<wasmtime::Module> {
    // SAFETY: `compiled` was just produced by `Engine::precompile_module` on this same engine.
    unsafe { wasmtime::Module::deserialize(engine, compiled) }
}

pub(crate) struct NativeGuest {
    call: CallFn,
    alloc: AllocFn,
    free: FreeFn,
    _library: Library,
}

struct NativeCall<'a, 'b> {
    cx: &'a mut HostCx<'b>,
    alloc: AllocFn,
}

impl NativeGuest {
    pub fn load(path: &Path) -> Result<NativeGuest, PluginError> {
        let failed = |e: libloading::Error| PluginError::Load(format!("{}: {e}", path.display()));
        // SAFETY: loading a native plugin runs its code with the app's privileges; the user accepted
        // that for this plugin before the registry allowed the load. The symbols are declared by the SDK
        // `export!` macro with exactly these signatures.
        unsafe {
            let library = Library::new(path).map_err(failed)?;
            let abi = *library.get::<AbiFn>(b"craft_plugin_abi\0").map_err(failed)?;
            let found = abi();
            if found != ABI {
                return Err(PluginError::Load(format!("plugin speaks ABI {found}, this app speaks ABI {ABI}")));
            }
            let init = *library.get::<InitFn>(b"craft_plugin_init\0").map_err(failed)?;
            let call = *library.get::<CallFn>(b"craft_plugin_call\0").map_err(failed)?;
            let alloc = *library.get::<AllocFn>(b"craft_plugin_alloc\0").map_err(failed)?;
            let free = *library.get::<FreeFn>(b"craft_plugin_free\0").map_err(failed)?;
            init(host_call);
            Ok(NativeGuest { call, alloc, free, _library: library })
        }
    }

    pub fn call(&self, request: &[u8], pixels: Option<&mut [Pixel]>, cx: &mut HostCx) -> Vec<u8> {
        let (pixels_at, pixels_len) = match pixels {
            Some(pixels) => (pixels.as_mut_ptr().cast::<u8>(), size_of_val(pixels)),
            None => (std::ptr::null_mut(), 0),
        };
        let mut frame = NativeCall { cx, alloc: self.alloc };
        let ctx = std::ptr::from_mut(&mut frame) as usize;
        let mut out = [0usize; 2];
        // SAFETY: the request, pixel buffer and `frame` outlive the call; the pixel buffer is exclusively
        // borrowed and f32 aligned; the reply is owned by the plugin until handed back to its `free`.
        unsafe {
            (self.call)(ctx, request.as_ptr(), request.len(), pixels_at, pixels_len, &mut out);
            if out[0] == 0 {
                return Vec::new();
            }
            let reply = std::slice::from_raw_parts(out[0] as *const u8, out[1]).to_vec();
            (self.free)(out[0] as *mut u8, out[1]);
            reply
        }
    }

    pub fn render_band(&self, request: &[u8], input: &[Pixel], output: &mut [Pixel], skip: usize, cx: &mut HostCx) -> Vec<u8> {
        if skip == 0 && input.len() == output.len() {
            output.copy_from_slice(input);
            return self.call(request, Some(output), cx);
        }
        let mut band = input.to_vec();
        let reply = self.call(request, Some(&mut band), cx);
        if let Some(core) = band.get(skip..skip + output.len()) {
            output.copy_from_slice(core);
        }
        reply
    }
}

unsafe extern "C" fn host_call(ctx: usize, request: *const u8, request_len: usize, bulk: *const u8, bulk_len: usize, out: *mut [usize; 4]) {
    let view = |at: *const u8, len: usize| -> &[u8] {
        if at.is_null() || len == 0 {
            return &[];
        }
        // SAFETY: the SDK passes slices it owns for the duration of the host call.
        unsafe { std::slice::from_raw_parts(at, len) }
    };
    let (request, bulk) = (view(request, request_len), view(bulk, bulk_len));
    let (alloc, (reply, bulk_out)) = if ctx == 0 {
        (None, refusal("host calls are only available during a call from the app"))
    } else {
        // SAFETY: `ctx` is the `NativeCall` that `NativeGuest::call` keeps on its stack, exclusively borrowed,
        // for the whole plugin call on this thread.
        let frame = unsafe { &mut *(ctx as *mut NativeCall) };
        (Some(frame.alloc), guarded(frame.cx, request, bulk))
    };
    let place = |bytes: &[u8]| -> [usize; 2] {
        let Some(alloc) = alloc.filter(|_| !bytes.is_empty()) else { return [0, 0] };
        // SAFETY: `alloc` is the calling plugin's allocator; the SDK frees the block with the same length.
        unsafe {
            let at = alloc(bytes.len());
            if at.is_null() {
                return [0, 0];
            }
            std::ptr::copy_nonoverlapping(bytes.as_ptr(), at, bytes.len());
            [at as usize, bytes.len()]
        }
    };
    let [reply_at, reply_len] = place(&reply);
    let [bulk_at, bulk_len] = place(bulk_out.bytes());
    // SAFETY: the SDK passes a writable four word array.
    unsafe { *out = [reply_at, reply_len, bulk_at, bulk_len] };
}
