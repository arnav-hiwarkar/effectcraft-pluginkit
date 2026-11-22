#[cfg(not(target_family = "wasm"))]
mod cache;
mod host;
mod images;
pub mod kernels;
mod package;
mod panel;
mod plugin;
mod registry;
#[cfg(not(target_family = "wasm"))]
mod runtime;
#[cfg(target_family = "wasm")]
#[path = "unavailable.rs"]
mod runtime;

pub use craft_plugin::{
    AudioRequest, Blend, Call, Camera, CommandSpec, Contributions, ControlsReply, Cull, Draw, EffectSpec, Entry, FrameDesc, FrameRequest, GpuData, GpuImage,
    GpuPass, GpuPlan, Layout, Light, LightKind, Manifest, MaskPath, PanelReply, PanelSpec, Param, ParamKind, ParamsAt, Permission, PickFile, Pixel, RenderCx,
    RunMode, Scene, TaskSpec, TaskState, TaskStatus, UiEvent, UiEventKind, UiInput, Value, Widget,
};
pub use host::{Access, Host, Origin, Pixels, Samples};
pub use images::{MAX_IMAGE_SIDE, PluginImage};
pub use panel::Retained;
pub use package::{EXTENSION as PACKAGE_EXTENSION, MAX_ENTRIES as PACKAGE_MAX_ENTRIES, MAX_UNPACKED_BYTES as PACKAGE_MAX_BYTES, PackageError, unpack};
pub use plugin::{Limits, Plugin, PluginError};
pub use registry::{Bundle, BundleError, Consent, Registry};

pub mod ui {
    pub use craft_plugin::{
        AppEvent, ControlsReply, Drag, Dropped, Key, Modifiers, Paint, PanelReply, Pointer, PointerPhase, Root, Shape, TextStyle, Topic, UiEvent, UiEventKind,
        UiInput, Widget,
    };
}

pub mod viewer {
    pub use craft_plugin::{Bind, Cursor, Handle, Mark, Overlay, Space, ToolEvent, ToolEventKind, ToolSpec};
}
