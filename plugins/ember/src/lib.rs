mod draw;
mod noise;
mod random;
mod raster;
mod settings;
mod sim;
mod spec;

use craft_plugin::prelude::*;

use draw::{DrawList, View};
use settings::{Hash, Settings};
use sim::{SUBSTEPS, Simulator, State};

pub const CHECKPOINT_EVERY: u32 = 10;
pub const LAYOUT_WORK: u64 = 12_000_000;

struct Ember;

pub trait Memo {
    fn recall(&self, key: &str) -> Option<Vec<u8>>;
    fn remember(&self, key: &str, bytes: &[u8]);
}

impl Memo for Cx {
    fn recall(&self, key: &str) -> Option<Vec<u8>> {
        Cx::recall(self, key)
    }

    fn remember(&self, key: &str, bytes: &[u8]) {
        let _ = Cx::remember(self, key, bytes);
    }
}

fn checkpoint_key(hash: u64, frame: u32) -> String {
    format!("ck/{hash:016x}/{frame}")
}

fn last_key(hash: u64) -> String {
    format!("last/{hash:016x}")
}

fn nearest(memo: &dyn Memo, hash: u64, frame: u32) -> Option<State> {
    let last = memo.recall(&last_key(hash)).and_then(|b| State::from_bytes(&b)).filter(|s| s.frame <= frame);
    let mut at = frame - frame % CHECKPOINT_EVERY;
    loop {
        if last.as_ref().is_some_and(|s| s.frame >= at) {
            return last;
        }
        if let Some(state) = memo.recall(&checkpoint_key(hash, at)).and_then(|b| State::from_bytes(&b)).filter(|s| s.frame == at) {
            return Some(state);
        }
        if at == 0 {
            return last;
        }
        at -= CHECKPOINT_EVERY;
    }
}

pub fn simulate(memo: &dyn Memo, settings: &Settings, work_limit: u64) -> Option<State> {
    let hash = settings.sim.hash();
    let simulator = Simulator::new(&settings.sim);
    let target = settings.frame;
    if target == u32::MAX {
        return Some(State { frame: target, particles: Vec::new() });
    }
    let mut state = nearest(memo, hash, target).unwrap_or_else(|| {
        let state = simulator.start();
        memo.remember(&checkpoint_key(hash, 0), &state.to_bytes());
        state
    });
    let mut work = 0u64;
    while state.frame < target {
        if work > work_limit {
            memo.remember(&last_key(hash), &state.to_bytes());
            return None;
        }
        simulator.step_frame(&mut state);
        work += (state.particles.len() as u64 + 1) * u64::from(SUBSTEPS);
        if state.frame.is_multiple_of(CHECKPOINT_EVERY) {
            memo.remember(&checkpoint_key(hash, state.frame), &state.to_bytes());
        }
    }
    memo.remember(&last_key(hash), &state.to_bytes());
    Some(state)
}

fn masks(cx: &Cx) -> Vec<Vec<[f64; 2]>> {
    if cx.index(spec::EMITTER_TYPE) == 3 { cx.masks().into_iter().map(|m| closed(m.points, m.closed)).collect() } else { Vec::new() }
}

fn closed(mut points: Vec<[f64; 2]>, is_closed: bool) -> Vec<[f64; 2]> {
    if is_closed && let Some(&first) = points.first() {
        points.push(first);
    }
    points
}

fn view(cx: &Cx, settings: &Settings) -> View {
    let projection = if settings.look.camera { cx.scene().filter(|s| s.camera_layer).and_then(|s| s.projection) } else { None };
    View {
        projection,
        scale: cx.scale.max(1e-6) as f32,
        offset: [cx.offset[0] as f32, cx.offset[1] as f32],
        size: [cx.size[0] as usize, cx.size[1] as usize],
        frame_rate: settings.sim.frame_rate,
    }
}

fn draw_key(settings: &Settings, view: &View) -> String {
    let mut h = Hash::new();
    h.u32((settings.sim.hash() >> 32) as u32).u32(settings.sim.hash() as u32);
    let look = format!("{:?}", settings.look);
    look.bytes().for_each(|b| {
        h.u32(u32::from(b));
    });
    h.f32(view.scale).f32s(&view.offset).u32(view.size[0] as u32).u32(view.size[1] as u32);
    view.projection.iter().flatten().flatten().for_each(|v| {
        h.u32((v.to_bits() >> 32) as u32).u32(v.to_bits() as u32);
    });
    format!("draw/{:016x}/{}", h.finish(), settings.frame)
}

pub fn prepare(memo: &dyn Memo, settings: &Settings, view: &View, work_limit: u64) -> Option<DrawList> {
    let key = draw_key(settings, view);
    if let Some(list) = memo.recall(&key).and_then(|b| DrawList::from_bytes(&b)) {
        return Some(list);
    }
    let state = simulate(memo, settings, work_limit)?;
    let list = draw::build(&settings.look, view, &state.particles);
    memo.remember(&key, &list.to_bytes());
    Some(list)
}

fn read(cx: &Cx) -> (Settings, View) {
    let settings = Settings::read(cx, masks(cx));
    let view = view(cx, &settings);
    (settings, view)
}

impl Effect for Ember {
    fn spec(&self) -> EffectSpec {
        spec::effect()
    }

    fn layout(&self, cx: &Cx) -> Layout {
        let (settings, view) = read(cx);
        prepare(cx, &settings, &view, LAYOUT_WORK);
        Layout::pointwise()
    }

    fn render(&self, cx: &Cx, frame: &mut Frame) -> Result<()> {
        let (settings, view) = read(cx);
        if let Some(list) = prepare(cx, &settings, &view, u64::MAX) {
            raster::render(&list, frame);
        }
        Ok(())
    }
}

fn plugin() -> Plugin {
    Plugin::new().effect(Ember)
}

craft_plugin::export!(plugin);

#[cfg(test)]
mod tests {
    use super::*;
    use settings::Emitter;
    use std::cell::RefCell;
    use std::collections::HashMap;

    #[derive(Default)]
    struct Shelf(RefCell<HashMap<String, Vec<u8>>>);

    impl Memo for Shelf {
        fn recall(&self, key: &str) -> Option<Vec<u8>> {
            self.0.borrow().get(key).cloned()
        }

        fn remember(&self, key: &str, bytes: &[u8]) {
            self.0.borrow_mut().insert(key.into(), bytes.to_vec());
        }
    }

    struct Forgetful;

    impl Memo for Forgetful {
        fn recall(&self, _key: &str) -> Option<Vec<u8>> {
            None
        }

        fn remember(&self, _key: &str, _bytes: &[u8]) {}
    }

    fn settings(frame: u32) -> Settings {
        let cx = craft_plugin::RenderCx {
            params: spec::effect()
                .params
                .into_iter()
                .filter_map(|p| {
                    use craft_plugin::{ParamKind as K, Value as V};
                    Some((
                        p.id,
                        match p.kind {
                            K::Slider { default, .. } | K::Percent { default } | K::Angle { default } => V::Number(default),
                            K::Checkbox { default } => V::Bool(default),
                            K::Popup { default, .. } => V::Number(f64::from(default)),
                            K::Point3 { default } => V::List(vec![default[0] * 320.0, default[1] * 180.0, default[2]]),
                            K::Color { default } => V::List(default.to_vec()),
                            _ => return None,
                        },
                    ))
                })
                .chain([("Physics/turbulence".to_string(), craft_plugin::Value::Number(300.0))])
                .collect(),
            time: f64::from(frame) / 30.0,
            frame_rate: 30.0,
            scale: 1.0,
            size: [320, 180],
            ..Default::default()
        };
        Settings::read(&Cx::new(cx), Vec::new())
    }

    #[test]
    fn a_frame_is_the_same_from_any_checkpoint_or_none() {
        let cold = simulate(&Forgetful, &settings(37), u64::MAX).expect("finishes");
        let shelf = Shelf::default();
        let stepped: Vec<State> = (0..=37).map(|f| simulate(&shelf, &settings(f), u64::MAX).expect("finishes")).collect();
        assert_eq!(stepped[37], cold);
        assert_eq!(simulate(&shelf, &settings(37), u64::MAX), Some(cold.clone()));
        assert_eq!(simulate(&shelf, &settings(23), u64::MAX).map(|s| s.frame), Some(23));
        assert_eq!(cold.frame, 37);
        assert!(cold.particles.len() > 300);
    }

    #[test]
    fn a_bounded_catch_up_resumes_where_it_stopped() {
        let shelf = Shelf::default();
        let mut attempts = 0;
        let state = loop {
            attempts += 1;
            if let Some(state) = simulate(&shelf, &settings(45), 20_000) {
                break state;
            }
        };
        assert!(attempts > 1);
        assert_eq!(state, simulate(&Forgetful, &settings(45), u64::MAX).expect("finishes"));
    }

    #[test]
    fn changing_a_simulation_parameter_changes_the_cache_key() {
        let a = settings(10);
        let mut b = a.clone();
        b.sim.gravity += 1.0;
        assert_ne!(a.sim.hash(), b.sim.hash());
        let mut c = a.clone();
        c.look.size += 1.0;
        assert_eq!(a.sim.hash(), c.sim.hash());
        assert!(matches!(a.sim.emitter, Emitter::Point));
    }
}
