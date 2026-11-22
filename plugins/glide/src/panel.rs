use craft_plugin::{Canvas, Paint, Panel, Pointer, PointerPhase, Ui};

use crate::apply::{self, APPLY_LABEL};
use crate::bridge::Host;
use crate::curve::{Curve, Handle, Y_MAX, Y_MIN};
use crate::presets::{self, BUILTIN, PRESETS_FILE, Preset};
use crate::shared;

const EDITOR_PAD: f32 = 14.0;
const EDITOR_MIN: f32 = 180.0;
const EDITOR_MAX: f32 = 380.0;
const THUMB: f32 = 56.0;
const THUMB_PAD: f32 = 5.0;
const THUMB_GAP: f32 = 12.0;
const HANDLE_RADIUS: f32 = 5.5;
const HOT_HANDLE_RADIUS: f32 = 7.5;
const TIMING_DOTS: u32 = 12;
const IDLE_STATUS: &str = "Select keyframes, shape the curve, then Apply.";

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Plot {
    min: [f32; 2],
    max: [f32; 2],
}

impl Plot {
    pub fn inset(size: [f32; 2], pad: f32) -> Plot {
        Plot { min: [pad, pad], max: [size[0] - pad, size[1] - pad] }
    }

    fn span(&self) -> [f64; 2] {
        [f64::from(self.max[0] - self.min[0]), f64::from(self.max[1] - self.min[1]) / (Y_MAX - Y_MIN)]
    }

    pub fn at(&self, [x, y]: [f64; 2]) -> [f32; 2] {
        let [sx, sy] = self.span();
        [self.min[0] + (x * sx) as f32, self.max[1] - ((y - Y_MIN) * sy) as f32]
    }

    pub fn point(&self, [px, py]: [f32; 2]) -> [f64; 2] {
        let [sx, sy] = self.span();
        [f64::from(px - self.min[0]) / sx, Y_MIN + f64::from(self.max[1] - py) / sy]
    }

    fn bezier(&self, curve: Curve) -> [[f32; 2]; 4] {
        [self.at([0.0, 0.0]), self.at(curve.handle(Handle::Start)), self.at(curve.handle(Handle::End)), self.at([1.0, 1.0])]
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct HandleDrag {
    pub grabbed: Option<Handle>,
    pub hovered: Option<Handle>,
}

impl HandleDrag {
    pub fn follow(&mut self, curve: &mut Curve, pointer: Option<Pointer>, plot: &Plot) -> bool {
        let Some(Pointer { pos, phase, .. }) = pointer else {
            self.hovered = None;
            return false;
        };
        let point = plot.point(pos);
        let nearest = curve.nearest_handle(point, plot.span());
        match phase {
            PointerPhase::Hover => {
                self.hovered = Some(nearest);
                self.grabbed.take().is_some()
            }
            PointerPhase::Down | PointerPhase::Drag => {
                let handle = *self.grabbed.get_or_insert(nearest);
                *curve = curve.with_handle(handle, point);
                self.hovered = Some(handle);
                false
            }
            PointerPhase::Up => match self.grabbed.take() {
                Some(handle) => {
                    *curve = curve.with_handle(handle, point);
                    true
                }
                None => false,
            },
        }
    }

    fn hot(&self) -> Option<Handle> {
        self.grabbed.or(self.hovered)
    }
}

fn draw_editor(c: &mut Canvas, plot: &Plot, curve: Curve, hot: Option<Handle>) {
    c.rect([0.0, 0.0], c.size, Some(Paint::Surface), Some((Paint::Border, 1.0)), 4.0);
    for step in 0..=4 {
        let v = f64::from(step) / 4.0;
        c.line(&[plot.at([v, Y_MIN]), plot.at([v, Y_MAX])], Paint::Border, 1.0);
        c.line(&[plot.at([0.0, v]), plot.at([1.0, v])], Paint::Border, 1.0);
    }
    c.rect(plot.at([0.0, 1.0]), plot.at([1.0, 0.0]), None, Some((Paint::Weak, 1.0)), 0.0);
    c.line(&[plot.at([0.0, 0.0]), plot.at([1.0, 1.0])], Paint::Weak, 1.0);
    for handle in [Handle::Start, Handle::End] {
        c.line(&[plot.at(Curve::anchor(handle)), plot.at(curve.handle(handle))], Paint::Text, 1.0);
    }
    c.bezier(plot.bezier(curve), Paint::Accent, 2.5);
    for step in 1..TIMING_DOTS {
        let t = f64::from(step) / f64::from(TIMING_DOTS);
        c.circle(plot.at([t, curve.value_at(t)]), 2.0, Some(Paint::Text), None);
    }
    for handle in [Handle::Start, Handle::End] {
        c.circle(plot.at(Curve::anchor(handle)), 3.0, Some(Paint::Text), None);
        let is_hot = hot == Some(handle);
        let (radius, fill) = if is_hot { (HOT_HANDLE_RADIUS, Paint::Accent) } else { (HANDLE_RADIUS, Paint::Surface) };
        c.circle(plot.at(curve.handle(handle)), radius, Some(fill), Some((Paint::Accent, 2.0)));
    }
}

fn thumbnail(ui: &mut Ui, key: &str, curve: Curve, current: bool) -> bool {
    let mut clicked = false;
    ui.canvas(key, [THUMB, THUMB], |c| {
        let inside = |pos: [f32; 2]| (0.0..=THUMB).contains(&pos[0]) && (0.0..=THUMB).contains(&pos[1]);
        clicked = matches!(c.pointer, Some(Pointer { phase: PointerPhase::Up, pos, .. }) if inside(pos));
        let border = if current || c.pointer.is_some() { (Paint::Accent, 1.5) } else { (Paint::Border, 1.0) };
        let plot = Plot::inset(c.size, THUMB_PAD);
        c.rect([0.0, 0.0], c.size, Some(Paint::Surface), Some(border), 3.0);
        c.rect(plot.at([0.0, 1.0]), plot.at([1.0, 0.0]), None, Some((Paint::Border, 1.0)), 0.0);
        c.bezier(plot.bezier(curve), if current { Paint::Accent } else { Paint::Text }, 1.5);
    });
    clicked
}

fn editor_side(available: [f32; 2]) -> f32 {
    (available[0] - 2.0 * EDITOR_PAD).clamp(EDITOR_MIN, EDITOR_MAX)
}

fn grid_columns(available: [f32; 2]) -> usize {
    (((available[0] + THUMB_GAP) / (THUMB + THUMB_GAP)).floor() as usize).clamp(2, 8)
}

fn same_curve(a: Curve, b: Curve) -> bool {
    [(a.x1, b.x1), (a.y1, b.y1), (a.x2, b.x2), (a.y2, b.y2)].iter().all(|(p, q)| (p - q).abs() < 1e-6)
}

pub struct GlidePanel<H> {
    host: H,
    pub curve: Curve,
    pub drag: HandleDrag,
    pub auto_apply: bool,
    pub status: String,
    pub preset_name: String,
    pub user_presets: Vec<Preset>,
    presets_loaded: bool,
}

impl<H: Host> GlidePanel<H> {
    pub fn new(host: H) -> Self {
        GlidePanel {
            host,
            curve: Curve::DEFAULT,
            drag: HandleDrag::default(),
            auto_apply: false,
            status: IDLE_STATUS.into(),
            preset_name: String::new(),
            user_presets: Vec::new(),
            presets_loaded: false,
        }
    }

    fn load_presets_once(&mut self) {
        if std::mem::replace(&mut self.presets_loaded, true) {
            return;
        }
        if let Ok(bytes) = self.host.read_data(PRESETS_FILE) {
            match presets::decode(&bytes) {
                Ok(saved) => self.user_presets = saved,
                Err(message) => self.status = message,
            }
        }
    }

    fn apply(&mut self) {
        self.status = apply::apply(&self.host, self.curve, APPLY_LABEL).unwrap_or_else(|message| message);
    }

    fn read_selection(&mut self) {
        self.status = match apply::read(&self.host) {
            Ok((curve, property)) => {
                self.curve = curve;
                format!("Read {} from {property}", curve.css())
            }
            Err(message) => message,
        };
    }

    fn choose(&mut self, curve: Curve) {
        self.curve = curve.clamped();
        if self.auto_apply {
            self.apply();
        }
    }

    fn store_presets(&mut self, done: String) {
        self.status = match presets::encode(&self.user_presets).and_then(|bytes| self.host.write_data(PRESETS_FILE, &bytes)) {
            Ok(()) => done,
            Err(message) => format!("Presets could not be saved: {message}"),
        };
    }

    fn save_preset(&mut self) {
        let name = self.preset_name.trim().to_string();
        presets::save(&mut self.user_presets, &name, self.curve);
        self.preset_name.clear();
        self.store_presets(format!("Saved preset {name}"));
    }

    fn delete_preset(&mut self, index: usize) {
        if index < self.user_presets.len() {
            let removed = self.user_presets.remove(index);
            self.store_presets(format!("Deleted preset {}", removed.name));
        }
    }

    fn editor(&mut self, ui: &mut Ui) {
        let side = editor_side(ui.available());
        let plot = Plot::inset([side, side], EDITOR_PAD);
        let (curve, drag) = (&mut self.curve, &mut self.drag);
        let mut released = false;
        ui.canvas("curve", [side, side], |c| {
            released = drag.follow(curve, c.pointer, &plot);
            draw_editor(c, &plot, *curve, drag.hot());
        });
        if released && self.auto_apply {
            self.apply();
        }
    }

    fn fields(&mut self, ui: &mut Ui) {
        let mut changed = false;
        let Curve { x1, y1, x2, y2 } = &mut self.curve;
        ui.row(|ui| {
            changed |= ui.number("x1", x1, 0.01);
            changed |= ui.number("y1", y1, 0.01);
            changed |= ui.number("x2", x2, 0.01);
            changed |= ui.number("y2", y2, 0.01);
        });
        if changed {
            self.curve = self.curve.clamped();
        }
        ui.mono(&self.curve.css());
    }

    fn controls(&mut self, ui: &mut Ui) {
        let (mut apply, mut read) = (false, false);
        let auto_apply = &mut self.auto_apply;
        ui.row(|ui| {
            apply = ui.button("Apply");
            read = ui.button("Read Selection");
            ui.checkbox("Auto Apply", auto_apply);
        });
        if apply {
            self.apply();
        }
        if read {
            self.read_selection();
        }
        ui.label(&self.status);
    }

    fn builtin_grid(&mut self, ui: &mut Ui) {
        let current = self.curve;
        let mut chosen = None;
        ui.group("Presets", |ui| {
            for chunk in BUILTIN.chunks(grid_columns(ui.available())) {
                ui.row(|ui| {
                    for preset in chunk {
                        ui.column(|ui| {
                            if thumbnail(ui, preset.name, preset.curve, same_curve(preset.curve, current)) {
                                chosen = Some(preset.curve);
                            }
                            ui.weak(preset.name);
                        });
                    }
                });
            }
        });
        if let Some(curve) = chosen {
            self.choose(curve);
        }
    }

    fn user_section(&mut self, ui: &mut Ui) {
        let current = self.curve;
        let (mut save, mut chosen, mut delete) = (false, None, None);
        let (name, saved) = (&mut self.preset_name, &self.user_presets);
        ui.group("Saved Presets", |ui| {
            ui.row(|ui| {
                ui.text_edit("preset-name", name, "Preset name");
                save = ui.button_enabled("Save", !name.trim().is_empty());
            });
            if saved.is_empty() {
                ui.weak("Save the current curve to keep it here.");
            }
            for (index, preset) in saved.iter().enumerate() {
                ui.row(|ui| {
                    if thumbnail(ui, &preset.name, preset.curve, same_curve(preset.curve, current)) {
                        chosen = Some(preset.curve);
                    }
                    ui.label(&preset.name);
                    if ui.button("Delete") {
                        delete = Some(index);
                    }
                });
            }
        });
        if save {
            self.save_preset();
        }
        if let Some(curve) = chosen {
            self.choose(curve);
        }
        if let Some(index) = delete {
            self.delete_preset(index);
        }
    }
}

impl<H: Host + Send + 'static> Panel for GlidePanel<H> {
    fn ui(&mut self, ui: &mut Ui) {
        self.load_presets_once();
        self.editor(ui);
        self.fields(ui);
        self.controls(ui);
        ui.separator();
        self.builtin_grid(ui);
        self.user_section(ui);
        shared::publish(self.curve);
    }
}
