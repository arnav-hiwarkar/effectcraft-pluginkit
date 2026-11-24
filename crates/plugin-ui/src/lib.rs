use craft_plugin::{Drag, Dropped, Paint};
use egui::{Color32, FontFamily};

mod marks;
mod panel;
#[cfg(test)]
mod tests;

pub use marks::{draw_marks, filled_polygon};
pub use panel::draw;

pub type Triangulate = fn(&[[f64; 2]]) -> Vec<[usize; 3]>;

#[derive(Clone)]
pub struct Theme {
    pub text: Color32,
    pub weak: Color32,
    pub accent: Color32,
    pub background: Color32,
    pub surface: Color32,
    pub border: Color32,
    pub error: Color32,
    pub heading: FontFamily,
}

impl Theme {
    pub fn paint(&self, paint: Paint) -> Color32 {
        match paint {
            Paint::Text => self.text,
            Paint::Weak => self.weak,
            Paint::Accent => self.accent,
            Paint::Background => self.background,
            Paint::Surface => self.surface,
            Paint::Border => self.border,
            Paint::Error => self.error,
            Paint::Rgba([r, g, b, a]) => Color32::from_rgba_unmultiplied(r, g, b, a),
        }
    }
}

pub trait Host {
    type Payload: std::any::Any + Send + Sync;

    fn tr(&self, english: &str) -> String;
    fn register(&mut self, id: &str, response: &egui::Response, label: &str);
    fn texture(&mut self, ctx: &egui::Context, key: &str) -> Option<egui::TextureId>;
    fn triangulate(&self) -> Triangulate;
    fn drag(&self, drag: &Drag) -> Option<Self::Payload>;
    fn dropped(&self, payload: &Self::Payload) -> Option<Dropped>;
    fn accept_files(&mut self, paths: &[String]);
}
