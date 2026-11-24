pub mod abi;
mod effect;
mod gradient;
pub mod host;
mod kernels;
mod manifest;
pub mod math;
mod plugin;
pub mod typed;
mod ui;
mod viewer;
mod wire;

pub use effect::{Cx, Effect, Frame, Image, Pass, Rows};
pub use gradient::{Gradient, MAX_GRADIENT_STOPS};
pub use kernels::*;
pub use manifest::{Entry, MANIFEST_FILE, Manifest, ManifestError, Permission, RunMode, native_target};
pub use plugin::{Error, Plugin, Result, Task};
pub use typed::{FromParam, LayerParam};
pub use ui::{Canvas, Panel, Ui};
pub use viewer::{Bind, Cursor, Handle, Mark, Overlay, Space, Tool, ToolEvent, ToolEventKind, ToolSpec};
pub use wire::*;

pub mod prelude {
    pub use crate::host;
    pub use crate::{
        AppEvent, AudioRequest, Bind, Blend, BlendMode, BlurShape, Camera, Canvas, Cull, Cursor, Cx, Drag, Dropped, Effect, EffectSpec, Encoding, Error,
        FileErrorKind, Filter, Frame, FrameRequest, Gradient, Handle, HostKernel, Image, Key, LayerParam, LayerPlacement, Layout, Light, LightKind, LogLevel,
        Mark, MaskPath, Measure, Modifiers, Negatives, Overlay, Paint, Panel, Param, Pass, PickFile, Pixel, Plan, PlanData, PlanImage, PlanPass, Plugin,
        Pointer, PointerPhase, Rect, Result, Root, Rows, Scene, Space, Taps, Task, TaskState, TaskStatus, Tool, ToolEvent, ToolEventKind, Topic, Ui, Value,
    };
    pub use crate::{log, params};
    pub use serde_json::{self, json};
}

#[macro_export]
macro_rules! log {
    (error, $($message:tt)+) => {
        $crate::host::log($crate::LogLevel::Error, ::std::format_args!("{}:{}: {}", ::std::file!(), ::std::line!(), ::std::format_args!($($message)+)))
    };
    (warn, $($message:tt)+) => {
        $crate::host::log($crate::LogLevel::Warn, ::std::format_args!("{}:{}: {}", ::std::file!(), ::std::line!(), ::std::format_args!($($message)+)))
    };
    (info, $($message:tt)+) => {
        $crate::host::log($crate::LogLevel::Info, ::std::format_args!("{}:{}: {}", ::std::file!(), ::std::line!(), ::std::format_args!($($message)+)))
    };
    ($($message:tt)+) => {
        $crate::log!(info, $($message)+)
    };
}

#[macro_export]
macro_rules! export {
    ($make:expr) => {
        static CRAFT_PLUGIN: std::sync::OnceLock<$crate::Plugin> = std::sync::OnceLock::new();

        #[unsafe(no_mangle)]
        pub extern "C" fn craft_plugin_abi() -> u32 {
            $crate::ABI
        }

        #[unsafe(no_mangle)]
        pub extern "C" fn craft_plugin_alloc(len: usize) -> *mut u8 {
            $crate::abi::alloc_bytes(len)
        }

        /// # Safety
        /// `ptr` and `len` must come from `craft_plugin_alloc`.
        #[unsafe(no_mangle)]
        pub unsafe extern "C" fn craft_plugin_free(ptr: *mut u8, len: usize) {
            unsafe { $crate::abi::free_bytes(ptr, len) }
        }

        /// # Safety
        /// The pointers must describe live buffers of the given lengths; `out` must be writable.
        #[unsafe(no_mangle)]
        pub unsafe extern "C" fn craft_plugin_call(
            ctx: usize,
            request: *const u8,
            request_len: usize,
            pixels: *mut u8,
            pixels_len: usize,
            out: *mut [usize; 2],
        ) {
            let plugin = || CRAFT_PLUGIN.get_or_init($make);
            unsafe { $crate::abi::entry(plugin, ctx, request, request_len, pixels, pixels_len, out) }
        }

        #[cfg(not(target_family = "wasm"))]
        #[unsafe(no_mangle)]
        pub extern "C" fn craft_plugin_init(host: $crate::abi::HostFn) {
            $crate::abi::set_host(host)
        }
    };
}
