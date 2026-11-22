#![allow(unsafe_code)]

use std::alloc::{Layout as MemLayout, alloc, dealloc};
use std::cell::Cell;
use std::panic::{AssertUnwindSafe, catch_unwind};

use crate::wire::{Outcome, PIXEL_BYTES, Pixel};
use crate::{LogLevel, Plugin};

pub type HostFn = unsafe extern "C" fn(ctx: usize, request: *const u8, request_len: usize, bulk: *const u8, bulk_len: usize, out: *mut [usize; 4]);

const ALIGN: usize = 16;

#[cfg(target_family = "wasm")]
#[link(wasm_import_module = "craft")]
unsafe extern "C" {
    fn host_call(ctx: usize, request: *const u8, request_len: usize, bulk: *const u8, bulk_len: usize, out: *mut [usize; 4]);
}

#[cfg(not(target_family = "wasm"))]
static HOST: std::sync::OnceLock<HostFn> = std::sync::OnceLock::new();

#[cfg(not(target_family = "wasm"))]
pub fn set_host(host: HostFn) {
    let _ = HOST.set(host);
}

thread_local! {
    static CTX: Cell<usize> = const { Cell::new(0) };
}

fn layout(len: usize) -> Option<MemLayout> {
    MemLayout::from_size_align(len.max(1), ALIGN).ok()
}

pub fn alloc_bytes(len: usize) -> *mut u8 {
    match layout(len) {
        // SAFETY: the layout has a non-zero size.
        Some(layout) => unsafe { alloc(layout) },
        None => std::ptr::null_mut(),
    }
}

/// # Safety
/// `ptr` and `len` must come from [`alloc_bytes`] and not be freed twice.
pub unsafe fn free_bytes(ptr: *mut u8, len: usize) {
    if let (false, Some(layout)) = (ptr.is_null(), layout(len)) {
        // SAFETY: the caller passes a pointer from `alloc_bytes(len)`, which used this layout.
        unsafe { dealloc(ptr, layout) }
    }
}

pub struct Buffer {
    ptr: *mut u8,
    len: usize,
}

// SAFETY: a Buffer exclusively owns its allocation.
unsafe impl Send for Buffer {}
// SAFETY: shared access only reads the allocation.
unsafe impl Sync for Buffer {}

impl Buffer {
    pub fn copy_of(bytes: &[u8]) -> Buffer {
        let ptr = alloc_bytes(bytes.len());
        if ptr.is_null() {
            return Buffer { ptr, len: 0 };
        }
        // SAFETY: `ptr` is a fresh allocation of at least `bytes.len()` bytes.
        unsafe { std::ptr::copy_nonoverlapping(bytes.as_ptr(), ptr, bytes.len()) };
        Buffer { ptr, len: bytes.len() }
    }

    pub fn bytes(&self) -> &[u8] {
        if self.ptr.is_null() {
            return &[];
        }
        // SAFETY: the buffer owns `len` initialised bytes at `ptr`.
        unsafe { std::slice::from_raw_parts(self.ptr, self.len) }
    }

    pub fn pixels(&self) -> &[Pixel] {
        if self.ptr.is_null() {
            return &[];
        }
        // SAFETY: allocations are 16-byte aligned and hold `len` initialised bytes; f32 has no invalid bit patterns.
        unsafe { std::slice::from_raw_parts(self.ptr.cast::<Pixel>(), self.len / PIXEL_BYTES) }
    }

    fn into_raw(self) -> [usize; 2] {
        let raw = [self.ptr as usize, self.len];
        std::mem::forget(self);
        raw
    }
}

impl Drop for Buffer {
    fn drop(&mut self) {
        // SAFETY: the buffer owns this allocation, made by `alloc_bytes(len)`.
        unsafe { free_bytes(self.ptr, self.len) }
    }
}

pub(crate) fn call_host(request: &[u8], bulk: &[u8]) -> (Buffer, Buffer) {
    let mut out = [0usize; 4];
    let ctx = CTX.get();
    #[cfg(target_family = "wasm")]
    // SAFETY: the host reads the two slices and writes four words to `out`.
    unsafe {
        host_call(ctx, request.as_ptr(), request.len(), bulk.as_ptr(), bulk.len(), &mut out)
    };
    #[cfg(not(target_family = "wasm"))]
    if let Some(host) = HOST.get() {
        // SAFETY: the host reads the two slices and writes four words to `out`.
        unsafe { host(ctx, request.as_ptr(), request.len(), bulk.as_ptr(), bulk.len(), &mut out) };
    }
    let buffer = |ptr: usize, len: usize| Buffer { ptr: ptr as *mut u8, len: if ptr == 0 { 0 } else { len } };
    (buffer(out[0], out[1]), buffer(out[2], out[3]))
}

pub(crate) fn spread(table: &mut [Pixel]) -> Option<(&[Pixel], &[Pixel], &mut [Pixel])> {
    if table.len() != 3 {
        return None;
    }
    // SAFETY: the table holds 48 initialised bytes, read as six unaligned words.
    let [src, src_len, aux, aux_len, out, out_len] = unsafe { table.as_ptr().cast::<[u64; 6]>().read_unaligned() };
    if src == 0 || aux == 0 || out == 0 {
        return None;
    }
    // SAFETY: the app sets `spread` only for a native plugin and fills the table with the addresses and pixel
    // counts of three live, f32-aligned pixel slices that it keeps borrowed for the whole call, the last one
    // exclusively and apart from the other two.
    unsafe {
        Some((
            std::slice::from_raw_parts(src as *const Pixel, src_len as usize),
            std::slice::from_raw_parts(aux as *const Pixel, aux_len as usize),
            std::slice::from_raw_parts_mut(out as *mut Pixel, out_len as usize),
        ))
    }
}

thread_local! {
    static PANIC: std::cell::RefCell<Option<String>> = const { std::cell::RefCell::new(None) };
}

fn report_panics() {
    static HOOK: std::sync::Once = std::sync::Once::new();
    HOOK.call_once(|| {
        let outside = std::panic::take_hook();
        std::panic::set_hook(Box::new(move |info| {
            if CTX.get() == 0 {
                return outside(info);
            }
            let message = info.payload_as_str().unwrap_or("no message");
            let report = match info.location() {
                Some(at) => format!("the plugin panicked at {}:{}:{}: {message}", at.file(), at.line(), at.column()),
                None => format!("the plugin panicked: {message}"),
            };
            crate::host::log(LogLevel::Error, &report);
            PANIC.set(Some(report));
        }));
    });
}

/// # Safety
/// `request` must point to `request_len` readable bytes, `pixels` to `pixels_len` writable bytes aligned
/// for `f32` (or be null), and `out` must be writable.
pub unsafe fn entry<'p>(
    plugin: impl FnOnce() -> &'p Plugin,
    ctx: usize,
    request: *const u8,
    request_len: usize,
    pixels: *mut u8,
    pixels_len: usize,
    out: *mut [usize; 2],
) {
    report_panics();
    // SAFETY: the caller guarantees `request` holds `request_len` readable bytes.
    let request = unsafe { std::slice::from_raw_parts(request, request_len) };
    let pixels: &mut [Pixel] = if pixels.is_null() {
        &mut []
    } else {
        // SAFETY: the caller guarantees an exclusive, f32-aligned buffer of `pixels_len` bytes.
        unsafe { std::slice::from_raw_parts_mut(pixels.cast::<Pixel>(), pixels_len / PIXEL_BYTES) }
    };
    let outer = CTX.replace(ctx);
    let reply = catch_unwind(AssertUnwindSafe(|| plugin().dispatch(request, pixels))).unwrap_or_else(|_| {
        let report = PANIC.take().unwrap_or_else(|| "the plugin panicked".into());
        serde_json::to_vec(&Outcome::<()>::Err(report)).unwrap_or_default()
    });
    CTX.set(outer);
    // SAFETY: the caller guarantees `out` is writable.
    unsafe { *out = Buffer::copy_of(&reply).into_raw() };
}
