mod draw;
mod noise;
mod random;
mod raster;
mod settings;
mod sim;
mod spec;

use craft_plugin::prelude::*;

use draw::{DrawList, View};
use settings::{Hash, Settings, Sim};
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

pub trait Source {
    fn sim_at(&self, frame: u32) -> Option<Sim>;
    fn history(&self) -> Option<u64>;
}

struct Live<'a> {
    cx: &'a Cx,
    pre_run_frames: u32,
    frame_rate: f64,
}

impl Source for Live<'_> {
    fn sim_at(&self, frame: u32) -> Option<Sim> {
        let time = (f64::from(frame) - f64::from(self.pre_run_frames)) / self.frame_rate;
        self.cx.at(time).map(|cx| Settings::read(&cx, Vec::new()).sim)
    }

    fn history(&self) -> Option<u64> {
        self.cx.param_history()
    }
}

pub fn sim_key(settings: &Settings, source: &dyn Source) -> Option<u64> {
    match (source.sim_at(0), source.history()) {
        (None, _) => Some(settings.sim.hash()),
        (Some(_), Some(history)) => Some(settings.sim.fixed_hash(history)),
        (Some(_), None) => None,
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

struct Forget;

impl Memo for Forget {
    fn recall(&self, _key: &str) -> Option<Vec<u8>> {
        None
    }

    fn remember(&self, _key: &str, _bytes: &[u8]) {}
}

pub fn simulate(memo: &dyn Memo, settings: &Settings, source: &dyn Source, work_limit: u64) -> Option<State> {
    let target = settings.frame;
    if target == u32::MAX {
        return Some(State { frame: target, carry: 0.0, particles: Vec::new() });
    }
    let (memo, hash): (&dyn Memo, u64) = match sim_key(settings, source) {
        Some(hash) => (memo, hash),
        None => (&Forget, 0),
    };
    let simulator = Simulator::new(&settings.sim);
    let sample = |frame: u32| source.sim_at(frame).unwrap_or_else(|| settings.sim.clone());
    let mut state = nearest(memo, hash, target).unwrap_or_else(|| {
        let state = simulator.start(&sample(0), &sample(1));
        memo.remember(&checkpoint_key(hash, 0), &state.to_bytes());
        state
    });
    let mut work = 0u64;
    let mut after = sample(state.frame.wrapping_add(1));
    while state.frame < target {
        if work > work_limit {
            memo.remember(&last_key(hash), &state.to_bytes());
            return None;
        }
        let now = after;
        after = sample(state.frame.wrapping_add(2));
        simulator.step_frame(&mut state, &now, &after);
        work += (state.particles.len() as u64 + 1) * u64::from(SUBSTEPS);
        if state.frame.is_multiple_of(CHECKPOINT_EVERY) {
            memo.remember(&checkpoint_key(hash, state.frame), &state.to_bytes());
        }
    }
    memo.remember(&last_key(hash), &state.to_bytes());
    Some(state)
}

fn masks(cx: &Cx) -> Vec<Vec<[f64; 2]>> {
    if spec::Controls::read(cx).kind == 3 { cx.masks().into_iter().map(|m| closed(m.points, m.closed)).collect() } else { Vec::new() }
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

fn draw_key(sim_key: u64, settings: &Settings, view: &View) -> String {
    let mut h = Hash::new();
    h.u32((sim_key >> 32) as u32).u32(sim_key as u32);
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

pub fn prepare(memo: &dyn Memo, settings: &Settings, source: &dyn Source, view: &View, work_limit: u64) -> Option<DrawList> {
    let key = sim_key(settings, source).map(|sim_key| draw_key(sim_key, settings, view));
    if let Some(list) = key.as_ref().and_then(|key| memo.recall(key)).and_then(|b| DrawList::from_bytes(&b)) {
        return Some(list);
    }
    let state = simulate(memo, settings, source, work_limit)?;
    let list = draw::build(&settings.look, view, &state.particles);
    if let Some(key) = key {
        memo.remember(&key, &list.to_bytes());
    }
    Some(list)
}

fn read(cx: &Cx) -> (Settings, View) {
    let settings = Settings::read(cx, masks(cx));
    let view = view(cx, &settings);
    (settings, view)
}

fn live<'a>(cx: &'a Cx, settings: &Settings) -> Live<'a> {
    Live { cx, pre_run_frames: settings.sim.pre_run_frames, frame_rate: f64::from(settings.sim.frame_rate) }
}

impl Effect for Ember {
    fn spec(&self) -> EffectSpec {
        spec::effect()
    }

    fn layout(&self, cx: &Cx) -> Layout {
        let (settings, view) = read(cx);
        prepare(cx, &settings, &live(cx, &settings), &view, LAYOUT_WORK);
        Layout::pointwise()
    }

    fn render(&self, cx: &Cx, frame: &mut Frame) -> Result<()> {
        let (settings, view) = read(cx);
        if let Some(list) = prepare(cx, &settings, &live(cx, &settings), &view, u64::MAX) {
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

    use Forget as Forgetful;

    struct Still;

    impl Source for Still {
        fn sim_at(&self, _frame: u32) -> Option<Sim> {
            None
        }

        fn history(&self) -> Option<u64> {
            None
        }
    }

    struct Moving(Sim);

    impl Moving {
        fn position(frame: u32) -> [f32; 3] {
            [40.0 + 8.0 * frame as f32, 90.0, 0.0]
        }
    }

    impl Source for Moving {
        fn sim_at(&self, frame: u32) -> Option<Sim> {
            Some(Sim { position: Moving::position(frame), ..self.0.clone() })
        }

        fn history(&self) -> Option<u64> {
            Some(1)
        }
    }

    fn centroid_x(state: &State) -> f32 {
        state.particles.iter().map(|p| p.position[0]).sum::<f32>() / state.particles.len().max(1) as f32
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
                            K::Slider { default, .. } | K::Percent { default, .. } | K::Angle { default } => V::Number(default),
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
        let cold = simulate(&Forgetful, &settings(37), &Still, u64::MAX).expect("finishes");
        let shelf = Shelf::default();
        let stepped: Vec<State> = (0..=37).map(|f| simulate(&shelf, &settings(f), &Still, u64::MAX).expect("finishes")).collect();
        assert_eq!(stepped[37], cold);
        assert_eq!(simulate(&shelf, &settings(37), &Still, u64::MAX), Some(cold.clone()));
        assert_eq!(simulate(&shelf, &settings(23), &Still, u64::MAX).map(|s| s.frame), Some(23));
        assert_eq!(cold.frame, 37);
        assert!(cold.particles.len() > 300);
    }

    #[test]
    fn a_bounded_catch_up_resumes_where_it_stopped() {
        let shelf = Shelf::default();
        let mut attempts = 0;
        let state = loop {
            attempts += 1;
            if let Some(state) = simulate(&shelf, &settings(45), &Still, 20_000) {
                break state;
            }
        };
        assert!(attempts > 1);
        assert_eq!(state, simulate(&Forgetful, &settings(45), &Still, u64::MAX).expect("finishes"));
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

    #[test]
    fn a_moving_emitter_leaves_a_trail_behind_it_and_passes_on_its_real_motion() {
        let base = settings(30);
        let calm = Sim { gravity: 0.0, turbulence: 0.0, speed: 20.0, drag: 0.0, ..base.sim.clone() };
        let moving = Settings { sim: calm.clone(), ..base.clone() };
        let trail = simulate(&Forgetful, &moving, &Moving(calm.clone()), u64::MAX).expect("finishes");
        let (start, now) = (Moving::position(0)[0], Moving::position(30)[0]);
        let centre = centroid_x(&trail);
        assert!(start < centre && centre < now - 40.0, "the cloud centre {centre} lies behind the emitter at {now}");

        let carried = Sim { inherit: 1.0, ..calm };
        let carried_settings = Settings { sim: carried.clone(), ..base };
        let carried_trail = simulate(&Forgetful, &carried_settings, &Moving(carried), u64::MAX).expect("finishes");
        assert!(centroid_x(&carried_trail) > centre + 40.0, "inheriting the emitter's motion of 240 px/s carries particles along");

        let shelf = Shelf::default();
        let stepped: Vec<State> =
            (0..=30).map(|f| simulate(&shelf, &Settings { frame: f, ..moving.clone() }, &Moving(moving.sim.clone()), u64::MAX).expect("finishes")).collect();
        assert_eq!(stepped[30], trail, "checkpoints of a moving emitter give the same frame as a cold run");
    }

    fn block_means(frame: u32) -> Vec<u8> {
        let settings = settings(frame);
        let view = View { projection: None, scale: 1.0, offset: [0.0, 0.0], size: [320, 180], frame_rate: 30.0 };
        let list = prepare(&Forgetful, &settings, &Still, &view, u64::MAX).expect("finishes");
        let background = vec![[0.02, 0.02, 0.05, 1.0]; 320 * 180];
        let mut pixels = vec![[0.0; 4]; 320 * 180];
        raster::render(&list, &mut Frame::whole(320, 180, &background, &mut pixels));
        let mean = |bx: usize, by: usize, channel: usize| {
            let total: f32 = (0..30).flat_map(|y| (0..40).map(move |x| (by * 30 + y) * 320 + bx * 40 + x)).map(|at| pixels[at][channel].min(1.0)).sum();
            (total / 1200.0 * 255.0 + 0.5) as u8
        };
        (0..6).flat_map(|by| (0..8).flat_map(move |bx| (0..3).map(move |c| (bx, by, c)))).map(|(bx, by, c)| mean(bx, by, c)).collect()
    }

    const BLOCK_MEANS: [u8; 144] = [
        5, 5, 13, 5, 5, 13, 5, 5, 13, 5, 5, 13, 5, 5, 13, 18, 16, 20, 21, 18, 21, 5, 5, 13, 5, 5, 13, 5, 5, 13, 5, 5, 13, 5, 5, 13, 13, 12, 18, 46, 40, 36, 9,
        8, 15, 5, 5, 13, 5, 5, 13, 5, 5, 13, 5, 5, 13, 13, 12, 18, 38, 34, 34, 10, 9, 16, 5, 5, 13, 6, 5, 13, 5, 5, 13, 5, 5, 13, 5, 5, 13, 17, 16, 21, 8, 7,
        14, 5, 5, 13, 5, 5, 13, 5, 5, 13, 5, 5, 13, 5, 5, 13, 5, 5, 13, 5, 5, 13, 6, 5, 13, 5, 5, 13, 6, 5, 13, 5, 5, 13, 5, 5, 13, 5, 5, 13, 5, 5, 13, 5, 5,
        13, 5, 5, 13, 5, 5, 13, 5, 5, 13, 6, 5, 13,
    ];

    #[test]
    fn a_frame_matches_the_block_means_made_with_the_standard_library_within_one_level() {
        let worst = block_means(37).iter().zip(BLOCK_MEANS).map(|(a, b)| a.abs_diff(b)).max().unwrap();
        assert!(worst <= 1, "largest 8-bit difference {worst}");
    }

    #[test]
    fn the_default_fountain_matches_its_stored_frame() {
        let plugin = craft_plugin_host::testing::TestPlugin::of(plugin);
        craft_plugin_host::assert_frame!(plugin.render("ember").time(1.0).set("Physics/turbulence", 300.0), "fountain");
    }

    #[test]
    fn a_burst_of_sparks_from_a_box_matches_its_stored_frame() {
        let plugin = craft_plugin_host::testing::TestPlugin::of(plugin);
        let render =
            plugin.render("ember").time(0.8).set("Emitter/type", 1.0).set("Emitter/emission", 1.0).set("Emitter/rate", 4000.0).set("Particle/shape", 2.0);
        craft_plugin_host::assert_frame!(render, "sparks");
    }
}
