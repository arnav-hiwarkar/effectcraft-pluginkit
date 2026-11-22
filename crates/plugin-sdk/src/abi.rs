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
    static IN_CALL: Cell<bool> = const { Cell::new(false) };
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

pub(crate) struct Pixels<'p> {
    pixels: &'p mut [Pixel],
    addresses: bool,
}

type Spread<'p> = (&'p [Pixel], &'p [Pixel], &'p mut [Pixel]);

impl<'p> Pixels<'p> {
    pub(crate) fn packed(pixels: &'p mut [Pixel]) -> Pixels<'p> {
        Pixels { pixels, addresses: false }
    }

    pub(crate) fn into_packed(self) -> &'p mut [Pixel] {
        self.pixels
    }

    pub(crate) fn spread(self) -> Result<Spread<'p>, &'static str> {
        if !self.addresses {
            return Err("this call names its pixels by address, which only the app's native entry point may do; pass the pixels themselves");
        }
        let table = self.pixels;
        if table.len() != 3 {
            return Err("the frame addresses are missing");
        }
        // SAFETY: the table holds 48 initialised bytes, read as six unaligned words.
        let [src, src_len, aux, aux_len, out, out_len] = unsafe { table.as_ptr().cast::<[u64; 6]>().read_unaligned() };
        if src == 0 || aux == 0 || out == 0 {
            return Err("the frame addresses are missing");
        }
        // SAFETY: `addresses` is set only by `entry`, whose caller guarantees that a call marked `spread` comes with
        // a table of the addresses and pixel counts of three live, f32-aligned pixel slices that stay borrowed for
        // the whole call, the last one exclusively and apart from the other two.
        unsafe {
            Ok((
                std::slice::from_raw_parts(src as *const Pixel, src_len as usize),
                std::slice::from_raw_parts(aux as *const Pixel, aux_len as usize),
                std::slice::from_raw_parts_mut(out as *mut Pixel, out_len as usize),
            ))
        }
    }
}

struct InPlace {
    band: (usize, usize),
    copies: Vec<Vec<Pixel>>,
}

thread_local! {
    static IN_PLACE: std::cell::RefCell<Vec<InPlace>> = const { std::cell::RefCell::new(Vec::new()) };
    static SPARE: std::cell::RefCell<Vec<Pixel>> = const { std::cell::RefCell::new(Vec::new()) };
}

struct Ended;

impl Drop for Ended {
    fn drop(&mut self) {
        let copies = IN_PLACE.with_borrow_mut(Vec::pop).map(|scope| scope.copies).unwrap_or_default();
        if let Some(largest) = copies.into_iter().max_by_key(Vec::capacity) {
            SPARE.set(largest);
        }
    }
}

pub(crate) fn in_place<R>(band: &mut [Pixel], run: impl for<'a> FnOnce(&'a mut [Pixel]) -> R) -> R {
    IN_PLACE.with_borrow_mut(|scopes| scopes.push(InPlace { band: (band.as_ptr() as usize, band.len()), copies: Vec::new() }));
    let _ended = Ended;
    run(band)
}

pub(crate) fn snapshot(band: &mut [Pixel]) -> (&[Pixel], &mut [Pixel]) {
    let key = (band.as_ptr() as usize, band.len());
    if band.is_empty() || !IN_PLACE.with_borrow(|scopes| scopes.last().is_some_and(|scope| scope.band == key)) {
        return (&[], band);
    }
    let mut copy = SPARE.take();
    copy.clear();
    copy.extend_from_slice(band);
    let (at, len) = (copy.as_ptr(), copy.len());
    IN_PLACE.with_borrow_mut(|scopes| scopes.last_mut().map(|scope| scope.copies.push(copy)));
    // SAFETY: `copy` now belongs to the innermost `in_place` scope, which neither changes nor frees it before that
    // scope ends. `band` is that scope's band: the only way to hold it mutably is through the borrow `in_place`
    // passed to its closure, which cannot outlive the closure, so the returned slice cannot outlive the scope.
    (unsafe { std::slice::from_raw_parts(at, len) }, band)
}

thread_local! {
    static PANIC: std::cell::RefCell<Option<String>> = const { std::cell::RefCell::new(None) };
}

fn report_panics() {
    static HOOK: std::sync::Once = std::sync::Once::new();
    HOOK.call_once(|| {
        let outside = std::panic::take_hook();
        std::panic::set_hook(Box::new(move |info| {
            if !IN_CALL.get() {
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
/// for `f32` (or be null), and `out` must be writable. Outside WebAssembly, when the request is a render or pass
/// marked `spread`, `pixels` must hold the addresses and pixel counts of three live, f32-aligned pixel slices
/// that stay borrowed for the whole call, the last one exclusively and apart from the other two.
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
    let (outer, outer_call) = (CTX.replace(ctx), IN_CALL.replace(true));
    let pixels = Pixels { pixels, addresses: cfg!(not(target_family = "wasm")) };
    let reply = catch_unwind(AssertUnwindSafe(|| plugin().serve(request, pixels))).unwrap_or_else(|_| {
        let report = PANIC.take().unwrap_or_else(|| "the plugin panicked".into());
        serde_json::to_vec(&Outcome::<()>::Err(report)).unwrap_or_default()
    });
    CTX.set(outer);
    IN_CALL.set(outer_call);
    // SAFETY: the caller guarantees `out` is writable.
    unsafe { *out = Buffer::copy_of(&reply).into_raw() };
}
