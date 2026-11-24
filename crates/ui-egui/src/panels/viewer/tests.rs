use super::*;
use effectcraft_engine::geom::{Mat4, vec3};

/// #1544: Shift-clicking with the Pen keeps the new segment horizontal, vertical or diagonal.
#[test]
fn pen_shift_click_keeps_the_segment_at_45_degrees() {
    let near = |a: [f64; 2], b: [f64; 2]| (a[0] - b[0]).abs() < 1e-9 && (a[1] - b[1]).abs() < 1e-9;
    let from = [100.0, 100.0];
    assert!(near(constrain_45(from, [104.0, 180.0]), [100.0, 180.0]), "straight down");
    assert!(near(constrain_45(from, [30.0, 97.0]), [30.0, 100.0]), "left");
    assert!(near(constrain_45(from, [150.0, 46.0]), [152.0, 48.0]), "up and right, on the diagonal");
    assert!(near(constrain_45(from, from), from), "on the vertex");
    assert!(constrain_45(from, [f64::NAN, 3.0])[0].is_nan(), "a non-finite point stays as it is");
}

/// The Rotation tool counts past a full turn in both directions, as in After Effects: the
/// pointer going round the anchor 1.5 turns clockwise gives 540°, then back 2 turns gives
/// −180°, with no jump where atan2 wraps from 180° to −180°.
#[test]
fn rotate_tool_accumulates_revolutions() {
    // Screen y points down, so a growing atan2 angle is clockwise on screen.
    let drag = |start: f64, steps: &[f64]| {
        let (mut last, mut turned, mut out) = (start, 0.0, vec![]);
        for &a in steps {
            // atan2's range: (−180°, 180°].
            let a = (a + 180.0).rem_euclid(360.0) - 180.0;
            turned += turn_delta(last, a);
            last = a;
            out.push(turned);
        }
        (last, turned, out)
    };
    let cw: Vec<f64> = (1..=54).map(|i| i as f64 * 10.0).collect();
    let (last, turned, seq) = drag(0.0, &cw);
    assert!((turned - 540.0).abs() < 1e-9, "{turned}");
    assert!(seq.windows(2).all(|w| (w[1] - w[0] - 10.0).abs() < 1e-9), "no jump: {seq:?}");
    assert!((rotate_value(0.0, turned, false) - 540.0).abs() < 1e-9);
    let ccw: Vec<f64> = (1..=72).map(|i| 540.0 - i as f64 * 10.0).collect();
    let (_, back, seq) = drag(last, &ccw);
    assert!(seq.windows(2).all(|w| (w[1] - w[0] + 10.0).abs() < 1e-9), "no jump: {seq:?}");
    let total = turned + back;
    assert!((total + 180.0).abs() < 1e-9, "{total}");
    assert!((rotate_value(30.0, total, false) + 150.0).abs() < 1e-9);
    // Shift snaps the accumulated value, not a wrapped one.
    assert_eq!(rotate_value(0.0, 400.0, true), 405.0);
    assert_eq!(rotate_value(0.0, -400.0, true), -405.0);
    // Across the seam: 170° → −170° is +20°, and back is −20°; exactly half a turn is +180°.
    assert!((turn_delta(170.0, -170.0) - 20.0).abs() < 1e-9);
    assert!((turn_delta(-170.0, 170.0) + 20.0).abs() < 1e-9);
    assert_eq!(turn_delta(0.0, 180.0), 180.0);
    assert_eq!(turn_delta(0.0, -180.0), 180.0);
    // Never crash or poison the value on non-finite input.
    assert_eq!(turn_delta(f64::NAN, 10.0), 0.0);
    assert_eq!(turn_delta(0.0, f64::INFINITY), 0.0);
    assert_eq!(rotate_value(15.0, f64::NAN, false), 15.0);
}

/// #1042: a path vertex lying under, or just beside, a bounding-box handle is grabbed
/// instead of the handle; away from every handle the reach is unchanged.
#[test]
fn vertex_reaches_as_far_as_a_box_handle_over_it() {
    let handle = pos2(100.0, 100.0);
    let vertex = pos2(100.0, 100.0);
    // The press lands 8 points off the vertex, still on the handle's square.
    let press = pos2(108.0, 100.0);
    let pick = |p: Pos2, handles: &[Pos2]| vertex.distance(p) < vertex_grab_radius(p, handles.iter().copied());
    assert!(handle.distance(press) < HANDLE + 2.0, "the handle would take this press");
    assert!(pick(press, &[handle]), "the vertex wins over the handle");
    assert!(!pick(press, &[]), "no handle there: the usual reach");
    assert!(pick(pos2(105.0, 100.0), &[]));
    assert!(!pick(pos2(120.0, 100.0), &[handle]), "past the handle: nothing");
}

/// #882: the frame's corner sits on a physical pixel at any display scaling, so 100 % maps
/// comp pixels 1:1 onto screen pixels.
#[test]
fn frame_origin_snaps_to_physical_pixels() {
    for ppp in [1.0f32, 1.25, 1.5, 1.75, 2.0] {
        for p in [pos2(-201.12, 72.4), pos2(689.8, 239.75), pos2(0.3, -0.37)] {
            let s = snap_to_pixels(p, ppp);
            for v in [s.x * ppp, s.y * ppp] {
                assert!((v - v.round()).abs() < 1e-3, "ppp {ppp}: {p:?} → {s:?}");
            }
            assert!((s - p).length() <= 0.71 / ppp, "moved by at most half a pixel: {p:?} → {s:?}");
        }
    }
    assert_eq!(snap_to_pixels(pos2(1.3, 2.0), 0.0), pos2(1.3, 2.0));
}

/// #882: a magnified frame shows its pixels as blocks in both Zoom Quality modes; More
/// Accurate only smooths when the frame is shown smaller.
#[test]
fn magnified_frames_are_never_smoothed() {
    use eframe::wgpu::FilterMode;
    for smooth in [true, false] {
        assert_eq!(zoom_texture_options(smooth).magnification, egui::TextureFilter::Nearest);
        assert_eq!(zoom_sampler(smooth).mag_filter, FilterMode::Nearest);
    }
    assert_eq!(zoom_texture_options(true).minification, egui::TextureFilter::Linear);
    assert_eq!(zoom_sampler(true).min_filter, FilterMode::Linear);
}

/// A perspective-like homography: w = 100 + y (the plane crosses the camera at y = −100).
fn tilted() -> Mat3 {
    Mat3([[100.0, 0.0, 0.0], [0.0, 100.0, 0.0], [0.0, 1.0, 100.0]])
}

/// #572: a box reaching behind the camera is clipped at the near plane instead of flipping
/// through infinity into long lines.
#[test]
fn outlines_clip_at_the_near_plane() {
    let m = tilted();
    let front = project_clipped(&m, &box_corners([-50.0, -50.0, 50.0, 50.0]));
    assert_eq!(front.len(), 4);
    let crossing = project_clipped(&m, &box_corners([-50.0, -150.0, 50.0, 50.0]));
    assert_eq!(crossing.len(), 4, "{crossing:?}");
    // Nothing flipped through infinity: a corner behind the camera (y = −150, w = −50) would
    // land at y = +300 if it were projected as is; clipped at the near plane it runs off far
    // above (negative y) instead, and the front corners stay where they were.
    assert!(crossing.iter().all(|p| p[0].is_finite() && p[1].is_finite() && p[1] < 34.0), "{crossing:?}");
    assert!(project_clipped(&m, &box_corners([-50.0, -400.0, 50.0, -200.0])).is_empty(), "wholly behind");
}

#[test]
fn axis_drag_follows_the_arrow_and_ignores_drag_across_it() {
    let (ax, ay) = axis_drag([0.0, 0.0, 60.0], [24.0, -24.0]);
    let at = |d: [f64; 2]| [0, 1, 2].map(|i| ax[i] * d[0] + ay[i] * d[1]);
    assert_eq!(at([24.0, -24.0]), [0.0, 0.0, 60.0]);
    assert_eq!(at([-12.0, 12.0]), [0.0, 0.0, -30.0]);
    assert_eq!(at([10.0, 10.0]), [0.0, 0.0, 0.0]);
}

#[test]
fn gizmo_segments_clip_at_the_near_plane() {
    // w = 100 + z.
    let pc = Mat4([[100.0, 0.0, 0.0, 0.0], [0.0, 100.0, 0.0, 0.0], [0.0, 0.0, 1.0, 0.0], [0.0, 0.0, 1.0, 100.0]]);
    let s = project_segment(&pc, vec3(10.0, 0.0, 0.0), vec3(10.0, 0.0, -300.0)).unwrap();
    assert_eq!(s[0], [10.0, 0.0]);
    assert!(s[1][0] > 10.0 && s[1][0].is_finite(), "toward the camera it grows, never flips: {s:?}");
    assert!(project_segment(&pc, vec3(0.0, 0.0, -200.0), vec3(0.0, 0.0, -300.0)).is_none());
}

/// A 640×360 comp whose Box (anchor 40,40) is a child of Parent (anchor 50,50) at (100, 60).
fn parented_session(parent: serde_json::Value) -> (effectcraft_engine::Session, u64) {
    let mut s = effectcraft_engine::Session::default();
    s.execute("comp.new", json!({"name": "C", "width": 640, "height": 360, "duration": 1})).unwrap();
    let id = |v: serde_json::Value| v["layer"].as_u64().unwrap();
    let p = id(s.execute("layer.newSolid", json!({"name": "Parent", "color": "#000000", "width": 100, "height": 100})).unwrap());
    let c = id(s.execute("layer.newSolid", json!({"name": "Box", "color": "#000000", "width": 80, "height": 80})).unwrap());
    for (path, value) in parent.as_object().unwrap() {
        s.execute("prop.set", json!({"layer": p, "path": format!("transform/{path}"), "value": value})).unwrap();
    }
    s.execute("layer.setParent", json!({"layers": [c], "parent": p, "compensate": false})).unwrap();
    s.execute("prop.set", json!({"layer": c, "path": "transform/position", "value": [100.0, 60.0]})).unwrap();
    (s, c)
}

fn with_ctx<T>(s: &effectcraft_engine::Session, f: impl FnOnce(&EvalCtx) -> T) -> T {
    let comp = s.active_comp().unwrap();
    f(&EvalCtx::new(&s.project, s.active_comp_id().unwrap(), comp, Tick::ZERO))
}

/// #2870: the pivot of a child follows its parent, through the camera for a 3D layer
/// and none behind it.
#[test]
fn anchor_in_comp_follows_the_parent_and_the_camera() {
    let (mut s, c) = parented_session(json!({"position": [400.0, 200.0]}));
    with_ctx(&s, |ctx| assert_eq!(anchor_in_comp(ctx, ctx.comp.layer(LayerId(c)).unwrap()), Some([450.0, 210.0])));
    s.execute("layer.setSwitch", json!({"layers": [c], "switch": "threeD", "value": true})).unwrap();
    with_ctx(&s, |ctx| {
        let a = anchor_in_comp(ctx, ctx.comp.layer(LayerId(c)).unwrap()).unwrap();
        assert!((a[0] - 450.0).abs() < 1e-6 && (a[1] - 210.0).abs() < 1e-6, "{a:?}");
    });
    s.execute("prop.set", json!({"layer": c, "path": "transform/position", "value": [100.0, 60.0, -1e6]})).unwrap();
    with_ctx(&s, |ctx| assert_eq!(anchor_in_comp(ctx, ctx.comp.layer(LayerId(c)).unwrap()), None));
}

/// #999: a vertex drag pans the view only near or past the edge, toward it, faster further out.
#[test]
fn edge_pan_speed_moves_toward_the_edge() {
    let area = Rect::from_min_max(pos2(100.0, 50.0), pos2(900.0, 650.0));
    assert_eq!(edge_pan_speed(area, pos2(500.0, 350.0)), egui::Vec2::ZERO);
    let left = edge_pan_speed(area, pos2(105.0, 350.0));
    assert!(left.x > 0.0 && left.y == 0.0, "near the left edge the view moves right to show more: {left:?}");
    assert!(edge_pan_speed(area, pos2(20.0, 350.0)).x > left.x, "faster past the edge");
    let corner = edge_pan_speed(area, pos2(950.0, 700.0));
    assert!(corner.x < 0.0 && corner.y < 0.0, "{corner:?}");
}

/// Every 3D view names its own `view.3d.*` command: a typo in that mapping would fall back to the
/// English label silently, which is what the viewer showed before (#2691).
#[test]
fn every_3d_view_finds_its_japanese_row() {
    crate::i18n::ui::set_current("ja");
    for v in View3D::ALL {
        assert_ne!(view3d_label(v), v.label(), "{v:?} has no Japanese row");
    }
    crate::i18n::ui::set_current("en");
}

struct UiThreadReads {
    inner: std::sync::Arc<dyn effectcraft_engine::render::FootageSource>,
    ui: std::thread::ThreadId,
    reads: std::sync::atomic::AtomicUsize,
}

impl effectcraft_engine::render::FootageSource for UiThreadReads {
    fn frame(
        &self,
        item: effectcraft_engine::project::ItemId,
        footage: &effectcraft_engine::project::Footage,
        t: effectcraft_engine::time::Tick,
    ) -> Option<std::sync::Arc<effectcraft_engine::render::Image>> {
        if std::thread::current().id() == self.ui {
            self.reads.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        }
        self.inner.frame(item, footage, t)
    }
}

type PaneHarness = egui_kittest::Harness<'static, crate::EffectcraftApp>;

/// A 64×36 comp with a still, the locked viewer and a 4-view layout; footage reads on the UI
/// thread counted and the layer cache gated.
fn panes_app() -> (tempfile::TempDir, PaneHarness, effectcraft_engine::project::ItemId, std::sync::Arc<UiThreadReads>) {
    panes_app_with(None)
}

/// As [`panes_app`], with `remote` installed before the first frame, so no local worker can
/// render a frame the test means to see queued.
fn panes_app_with(
    remote: Option<std::sync::Arc<dyn crate::frames::RemoteFrames>>,
) -> (tempfile::TempDir, PaneHarness, effectcraft_engine::project::ItemId, std::sync::Arc<UiThreadReads>) {
    let dir = tempfile::tempdir().unwrap();
    let png = dir.path().join("still.png");
    image::RgbaImage::from_fn(64, 36, |x, y| image::Rgba([(x * 4) as u8, (y * 7) as u8, 90, 255])).save(&png).unwrap();
    let mut session = effectcraft_host::session();
    session.execute("comp.new", serde_json::json!({"width": 64, "height": 36, "duration": 2})).unwrap();
    let cid = session.active_comp_id().unwrap();
    let item = session.execute("file.import", serde_json::json!({"paths": [png.to_string_lossy()]})).unwrap()["items"][0].clone();
    session.execute("layer.addItem", serde_json::json!({"item": item})).unwrap();
    session.execute("view.splitLockedViewer", serde_json::json!({})).unwrap();
    session.execute("view.layout", serde_json::json!({"views": 4})).unwrap();
    let counter = std::sync::Arc::new(UiThreadReads { inner: session.footage.clone(), ui: std::thread::current().id(), reads: Default::default() });
    session.footage = counter.clone();
    session.layer_cache.set_gate(Some(std::sync::Arc::new(std::sync::atomic::AtomicBool::new(true))));
    let harness = egui_kittest::Harness::builder().with_size(egui::vec2(1400.0, 900.0)).build_eframe(|_| {
        let mut app = crate::EffectcraftApp::new(session);
        app.frames.set_remote(remote);
        app
    });
    (dir, harness, cid, counter)
}

const PANES: [u32; 4] = [crate::panels::viewers::LOCKED_PANE, u32::MAX - 1, u32::MAX - 2, u32::MAX - 3];

fn panes_shown(h: &PaneHarness) -> usize {
    PANES.iter().filter(|p| h.state().passive_tex.contains_key(p)).count()
}

fn wait_for_panes(h: &mut PaneHarness) {
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(60);
    while panes_shown(h) != 4 && std::time::Instant::now() < deadline {
        std::thread::sleep(std::time::Duration::from_millis(5));
        h.step();
    }
    assert_eq!(panes_shown(h), 4, "every pane shows its frame once the workers finish");
}

/// Renders nothing, so every queued frame stays queued.
struct Idle;
impl crate::frames::RemoteFrames for Idle {
    fn slots(&self) -> usize {
        0
    }
    fn start(&self, _: crate::frames::RemoteJob, _: crate::frames::RemoteDone) {}
}

/// Study change #10: the locked viewer and the extra views of a 4-view layout render on the frame
/// workers, not on the UI thread, and show the same pixels the UI thread rendered before.
#[test]
fn side_and_locked_panes_render_on_the_frame_workers() {
    use crate::panels::viewers::{LOCKED_PANE, aux_pane};
    let (_dir, mut harness, cid, counter) = panes_app();
    harness.step();
    assert_eq!(counter.reads.load(std::sync::atomic::Ordering::SeqCst), 0, "a pane rendered on the UI thread");
    wait_for_panes(&mut harness);
    assert_eq!(counter.reads.load(std::sync::atomic::Ordering::SeqCst), 0, "a pane rendered on the UI thread");
    for (pane, draft) in [(LOCKED_PANE, false), (aux_pane(0), true)] {
        let app = harness.state();
        let key = app.passive_tex[&pane].0;
        let Some(crate::frames::FrameImage::Cpu(img)) = app.frames.get(&key) else { panic!("no CPU frame for pane {pane}") };
        let opts = effectcraft_engine::render::RenderOpts { scale: f64::from(key.scale) / 1000.0, draft, ..Default::default() };
        let before = crate::frames::to_color_image(&app.session.render(cid, app.session.time_of(cid), opts));
        assert_eq!(img.size, before.size);
        assert!(img.pixels == before.pixels, "pane {pane} differs from the UI thread's render");
    }
}

/// The side panes' frames queue behind the frame the active viewer shows.
#[test]
fn the_viewer_frame_queues_before_the_side_panes() {
    let (_dir, mut harness, _, _) = panes_app_with(Some(std::sync::Arc::new(Idle)));
    harness.step();
    let queued = harness.state().frames.queued();
    assert!(queued.len() > 1, "the panes queue frames: {queued:?}");
    assert!(queued[0].1, "the active viewer's frame queues first: {queued:?}");
}

/// A pane switched to another view shows nothing until that view's frame is in, not the old
/// view's picture under the new view's label.
#[test]
fn a_pane_never_shows_another_view_s_frame() {
    use crate::panels::viewers::{aux_pane, pane};
    let (_dir, mut harness, cid, _) = panes_app();
    wait_for_panes(&mut harness);
    let ctx = harness.ctx.clone();
    let app = harness.state_mut();
    app.frames.set_remote(Some(std::sync::Arc::new(Idle)));
    let key = app.passive_tex[&aux_pane(0)].0;
    let top = app.session.state.views3d.get(&cid).cloned().unwrap_or_default().cam(View3D::Top, 64.0, 36.0).state();
    let opts = effectcraft_engine::render::RenderOpts { scale: f64::from(key.scale) / 1000.0, view: Some(top), draft: true, ..Default::default() };
    assert!(pane(app, &ctx, aux_pane(0), cid, opts, None).is_none());
}

/// Closing the last comp gives the panes' textures back.
#[test]
fn closing_the_last_comp_frees_the_pane_textures() {
    let (_dir, mut harness, _, _) = panes_app();
    wait_for_panes(&mut harness);
    harness.state_mut().session.activate_comp_none();
    harness.run_steps(2);
    assert_eq!(panes_shown(&harness), 0);
}
