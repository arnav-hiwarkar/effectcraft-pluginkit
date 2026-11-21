mod host;
mod plugin;
mod registry;
#[cfg(not(target_family = "wasm"))]
mod runtime;
#[cfg(target_family = "wasm")]
#[path = "unavailable.rs"]
mod runtime;

pub use craft_plugin::{
    Call, CommandSpec, Contributions, EffectSpec, Entry, FrameDesc, GpuImage, GpuPass, GpuPlan, Layout, Manifest, MaskPath, PanelSpec, Param, ParamKind,
    Permission, Pixel, RenderCx, Scene, UiEvent, UiEventKind, UiInput, Value, Widget,
};
pub use host::{Access, Host, Pixels};
pub use plugin::{Limits, Plugin, PluginError};
pub use registry::{Bundle, BundleError, Consent, Registry};

pub mod ui {
    pub use craft_plugin::{Paint, Pointer, PointerPhase, Shape, TextStyle, UiEvent, UiEventKind, UiInput, Widget};
}
