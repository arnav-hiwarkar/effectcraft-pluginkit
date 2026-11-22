#[cfg(not(target_family = "wasm"))]
mod cache;
mod host;
mod package;
mod plugin;
mod registry;
#[cfg(not(target_family = "wasm"))]
mod runtime;
#[cfg(target_family = "wasm")]
#[path = "unavailable.rs"]
mod runtime;

pub use craft_plugin::{
    Blend, Call, Camera, CommandSpec, Contributions, Cull, Draw, EffectSpec, Entry, FrameDesc, GpuData, GpuImage, GpuPass, GpuPlan, Layout, Light, LightKind,
    Manifest, MaskPath, PanelSpec, Param, ParamKind, Permission, Pixel, RenderCx, RunMode, Scene, UiEvent, UiEventKind, UiInput, Value, Widget,
};
pub use host::{Access, Host, Pixels};
pub use package::{EXTENSION as PACKAGE_EXTENSION, MAX_ENTRIES as PACKAGE_MAX_ENTRIES, MAX_UNPACKED_BYTES as PACKAGE_MAX_BYTES, PackageError, unpack};
pub use plugin::{Limits, Plugin, PluginError};
pub use registry::{Bundle, BundleError, Consent, Registry};

pub mod ui {
    pub use craft_plugin::{Paint, Pointer, PointerPhase, Shape, TextStyle, UiEvent, UiEventKind, UiInput, Widget};
}
