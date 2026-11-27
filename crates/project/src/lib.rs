//! The EffectCraft document model.
//!
//! `Project` → items (folders, compositions, footage, solids) → `Comp` → `Layer` → property tree
//! ([`props`]). Compositions, and each layer within them ([`Layers`]), sit behind `Arc` so undo
//! snapshots share everything that did not change.

#![deny(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::unimplemented, clippy::todo, clippy::unreachable)]

pub mod ae_order;
pub mod build;
pub mod essential;
mod file_check;
pub mod limits;
pub mod plugin_data;
pub mod props;
pub mod render_queue;
pub mod render_templates;
pub mod sequence;
pub mod styles;
pub mod tracking;

use std::collections::BTreeMap;
use std::sync::Arc;

pub use effectcraft_color::ColorSpace;
use effectcraft_color::{BlendMode, Label};
use effectcraft_time::{FrameRate, Tick};
pub use props::{Expression, FeatherFalloff, GroupKind, MaskMode, MaskMotionBlur, Node, ParamUi, PropGroup, Property, Uid, parse_path};
use serde::{Deserialize, Serialize};

pub use effectcraft_keyframe as keyframe;
pub use effectcraft_keyframe::{Keyframe, Value};

mod comp;
mod dirty;
mod footage;
mod item;
mod layer;
mod layers;
mod reach;
mod settings;

pub use comp::{Comp, Guide, Renderer, Resolution};
pub use footage::{
    AlphaMode, CineonSettings, CineonUnits, FieldOrder, Footage, FootageKind, PCRAFT_MAX_INPUT, Proxy, Solid, SourceLayer, VIDEO_EXTENSIONS, cineon_log_to_lin,
};
pub use item::NO_LAYER;
pub use item::{Heard, Item, ItemId, ItemKind, LayerId};
pub use layer::{
    AutoOrient, FrameBlend, Layer, LayerSource, LightKind, MatteKind, PrimitiveKind, Quality, Sampling, Switches, TrackMatte, declare_reads_other_frames,
    is_audio_effect, reads_other_frames, text_layer_name,
};
pub use layers::Layers;
pub use reach::{ExprReach, PixelReach, expression_reach};
pub use settings::{
    APP_VERSION, BitDepth, ColorEngine, CuePoint, HdrMode, Marker, ProjectError, ProjectSettings, SCHEMA_EFFECT_SPACE, SCHEMA_VERSION, SavedView,
    TimeDisplayStyle, is_newer_version, saved_by, saved_view,
};

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Project {
    pub schema: u32,
    pub settings: ProjectSettings,
    pub items: BTreeMap<ItemId, Item>,
    pub next_id: u64,
    /// The Render Queue (Composition ▸ Add to Render Queue).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub render_queue: Vec<render_queue::RenderQueueItem>,
    /// Render Settings / Output Module templates and their defaults.
    #[serde(default, skip_serializing_if = "render_templates::RenderTemplates::is_default")]
    pub render_templates: render_templates::RenderTemplates,
    /// Render Queue preferences (Notify, storage overflow folders).
    #[serde(default, skip_serializing_if = "render_queue::RenderQueuePrefs::is_default")]
    pub render_prefs: render_queue::RenderQueuePrefs,
    /// The plug-ins whose effects the project uses, as they were when it was saved.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub plugins: Vec<PluginUse>,
    /// Each plugin effect the project uses, saved as `plugin/item@version`. A string this version
    /// cannot read is kept and saved back as it was.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub plugin_refs: Vec<String>,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty", with = "plugin_data")]
    pub plugin_data: plugin_data::PluginData,
}

/// One plug-in a saved project uses: its id and version, and its digest once plug-ins carry one.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct PluginUse {
    pub id: String,
    pub version: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub digest: Option<String>,
    /// The plug-in's display name when the project was saved, shown while it is missing.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
}

#[cfg(test)]
mod tests;
