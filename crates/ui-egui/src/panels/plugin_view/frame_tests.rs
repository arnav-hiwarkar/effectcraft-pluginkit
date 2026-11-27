use egui_kittest::Harness;
use egui_kittest::kittest::{NodeT, Queryable};
use serde_json::json;

use super::panel_tests::{shows, until};
use crate::EffectcraftApp;
use crate::dock::PanelKind;
use crate::panels::plugin_tools::tests::{harness, probe_session};

const PROBE: &str = "test.viewer-tool-ui";
const PREVIEW: &str = "plugin.test.viewer-tool-ui.preview";

fn open_preview(comp: &str) -> Harness<'static, EffectcraftApp> {
    let mut session = probe_session();
    session.execute("layer.newSolid", json!({"name": "Red", "color": "#ff0000"})).unwrap();
    let mut harness = harness(session);
    let preview = PanelKind::plugin(PROBE, "preview");
    until(&mut harness, "the preview panel registering", |h| super::running_panel(h.state(), preview).is_some());
    let ctx = harness.ctx.clone();
    crate::menus::invoke(harness.state_mut(), &ctx, "window.pluginPanel", json!({"plugin": PROBE, "panel": "preview"})).unwrap();
    let counted = |h: &mut Harness<'_, EffectcraftApp>| {
        h.query_all(egui_kittest::kittest::by()).any(|node| node.accesskit_node().value().is_some_and(|text| text.ends_with(" clicks")))
    };
    until(&mut harness, "the preview showing", counted);
    let panel = crate::panels::plugin_tools::tests::element(&harness, PREVIEW);
    let field = |harness: &Harness<'static, EffectcraftApp>| {
        let inputs = harness.query_all(egui_kittest::kittest::by().role(egui::accesskit::Role::TextInput));
        inputs.into_iter().find(|node| panel.contains(node.rect().center())).map(|node| node.rect().center()).expect("the preview has its comp field")
    };
    let at = field(&harness);
    crate::panels::plugin_tools::tests::press_drag_release(&mut harness, &[at]);
    harness.step();
    harness.key_press_modifiers(egui::Modifiers::COMMAND, egui::Key::A);
    harness.step();
    let typed = egui::Event::Text(comp.to_string());
    harness.input_mut().events.push(typed);
    harness.step();
    harness
}

fn frame_centre(harness: &Harness<'_, EffectcraftApp>) -> Option<egui::Pos2> {
    let found = harness.state().auto.query(PREVIEW).into_iter().find(|element| element.label == "Frame")?;
    Some(egui::pos2(found.rect[0] + found.rect[2] / 2.0, found.rect[1] + found.rect[3] / 2.0))
}

fn colour_at_frame_centre(harness: &mut Harness<'_, EffectcraftApp>) -> Option<[u8; 3]> {
    let centre = frame_centre(harness)? * harness.ctx.pixels_per_point();
    let image = harness.render().ok()?;
    let pixel = image.get_pixel(centre.x as u32, centre.y as u32).0;
    Some([pixel[0], pixel[1], pixel[2]])
}

fn near(colour: Option<[u8; 3]>, wanted: [u8; 3]) -> bool {
    colour.is_some_and(|colour| colour.iter().zip(wanted).all(|(got, want)| got.abs_diff(want) <= 8))
}

#[test]
fn a_plugin_frame_shows_the_red_comp_rendered_by_the_app_and_redraws_on_an_edit_without_calling_the_plugin() {
    let _gpu = crate::frames::gpu_slot_test();
    let mut harness = open_preview("Viewer");
    until(&mut harness, "the red comp in the frame", |h| near(colour_at_frame_centre(h), [255, 0, 0]));
    let calls = harness.state().plugin_ui.panels.calls(&effectcraft_plugin::Owner::panel(PROBE, "preview", None));
    harness.state_mut().session.execute("layer.newSolid", json!({"name": "Blue", "color": "#0000ff"})).unwrap();
    until(&mut harness, "the edit redrawing the frame", |h| near(colour_at_frame_centre(h), [0, 0, 255]));
    assert_eq!(
        harness.state().plugin_ui.panels.calls(&effectcraft_plugin::Owner::panel(PROBE, "preview", None)),
        calls,
        "the app redrew the frame without asking the plugin"
    );
    assert!(effectcraft_plugin::image(PROBE, "view").is_none(), "the plugin uploaded no image for its frame");
    let centre = frame_centre(&harness).unwrap();
    harness.input_mut().events.push(egui::Event::PointerMoved(centre));
    for pressed in [true, false] {
        harness.input_mut().events.push(egui::Event::PointerButton {
            pos: centre,
            button: egui::PointerButton::Primary,
            pressed,
            modifiers: egui::Modifiers::NONE,
        });
        harness.step();
    }
    until(&mut harness, "the click reaching the plugin", |h| shows(h, "1 clicks"));
}

#[test]
fn a_plugin_frame_of_a_missing_comp_says_so_in_a_weak_label() {
    let _shared_preview = crate::frames::gpu_slot_test();
    let mut harness = open_preview("Nowhere");
    until(&mut harness, "the missing comp named", |h| shows(h, "no composition `Nowhere`"));
    let frames = harness.query_all_by_label("Frame").filter(|node| node.accesskit_node().role() == egui::accesskit::Role::Image).count();
    assert_eq!(frames, 0, "no frame is drawn for a comp that is not there");
}

#[test]
fn a_frame_whose_request_changes_on_every_draw_keeps_one_preview_with_one_render_and_ends_on_the_newest_request() {
    use craft_plugin::FrameRequest;
    use effectcraft_plugin::ui::Widget;
    let tokens = crate::theme::Tokens::for_kind(Default::default());
    let request = |step: u32| FrameRequest { time: f64::from(step) / 30.0, scale: 0.25, ..Default::default() };
    let state = (super::Images::default(), probe_session(), 0_u32);
    let mut harness = Harness::new_ui_state(
        move |ui, (images, session, step): &mut (super::Images, effectcraft_engine::Session, u32)| {
            let mut auto = crate::automation::Registry::default();
            let tree = vec![Widget::Frame { id: 7, request: request(*step), size: [80.0, 45.0] }];
            let source = super::Source { plugin: PROBE, images, session: Some(session), shown: None };
            super::draw(ui, &tree, &tokens, &mut auto, "plugin.test.panel", source);
        },
        state,
    );
    for step in 0..30 {
        harness.state_mut().2 = step;
        harness.step();
    }
    assert_eq!(harness.state().0.frames.len(), 1, "one preview per frame widget, not one per request");
    let newest = serde_json::to_string(&request(29)).unwrap();
    for _ in 0..2_000 {
        harness.step();
        let preview = harness.state().0.frames.values().next().unwrap();
        if preview.pending.is_none() && preview.asked.as_ref().is_some_and(|(_, asked)| *asked == newest) && matches!(preview.shown, Ok(Some(_))) {
            return;
        }
        std::thread::sleep(std::time::Duration::from_millis(5));
    }
    panic!("the preview never rendered the newest request");
}

type Previewed = (super::Images, effectcraft_engine::Session, u32);

fn previews(session: effectcraft_engine::Session, widgets: u64, size: [f32; 2], scale: f64) -> Harness<'static, Previewed> {
    use craft_plugin::FrameRequest;
    use effectcraft_plugin::ui::Widget;
    let tokens = crate::theme::Tokens::for_kind(Default::default());
    Harness::new_ui_state(
        move |ui, (images, session, step): &mut Previewed| {
            let mut auto = crate::automation::Registry::default();
            let tree: Vec<Widget> = (1..=widgets)
                .map(|id| Widget::Frame { id, request: FrameRequest { time: f64::from(*step) / 30.0 + id as f64 / 100.0, scale, ..Default::default() }, size })
                .collect();
            let source = super::Source { plugin: PROBE, images, session: Some(session), shown: None };
            super::draw(ui, &tree, &tokens, &mut auto, "plugin.test.panel", source);
        },
        (super::Images::default(), session, 0),
    )
}

fn shown_sizes(harness: &Harness<'_, Previewed>) -> Vec<[usize; 2]> {
    harness.state().0.frames.values().filter_map(|preview| preview.shown.as_ref().ok().cloned().flatten()).map(|texture| texture.size()).collect()
}

fn session_with_comp(width: u32, height: u32, color: &str) -> effectcraft_engine::Session {
    let mut session = probe_session();
    session.execute("comp.new", json!({"name": "Big", "width": width, "height": height, "frameRate": 24, "duration": 1})).unwrap();
    session.execute("layer.newSolid", json!({"name": "Fill", "color": color})).unwrap();
    session
}

#[test]
fn a_frame_preview_is_fitted_to_its_widget_and_to_the_texture_limit_keeping_its_aspect() {
    let _gpu = crate::frames::gpu_slot_test();
    let mut limited = previews(session_with_comp(2400, 1350, "#ff0000"), 1, [1600.0, 900.0], 1.0);
    limited.input_mut().max_texture_side = Some(1024);
    let mut fitted = previews(session_with_comp(1600, 900, "#ff0000"), 1, [80.0, 45.0], 1.0);
    for _ in 0..2_000 {
        limited.step();
        fitted.step();
        if !shown_sizes(&limited).is_empty() && !shown_sizes(&fitted).is_empty() {
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(5));
    }
    let (shown_limited, shown_fitted) = (shown_sizes(&limited), shown_sizes(&fitted));
    let ([limit], [fit]) = (shown_limited.as_slice(), shown_fitted.as_slice()) else { panic!("both previews show one texture") };
    let (limit, fit) = (*limit, *fit);
    assert!(limit[0] <= 1024 && limit[1] <= 1024, "{limit:?} fits the 1024 pixel texture limit");
    let ppp = fitted.ctx.pixels_per_point();
    assert!(fit[0] >= (80.0 * ppp) as usize && fit[0] < (160.0 * ppp) as usize, "{fit:?} is about as wide as the widget's pixels");
    for [width, height] in [limit, fit] {
        assert!((width as f32 / height as f32 - 16.0 / 9.0).abs() < 0.1, "{width}×{height} keeps the comp's 16:9");
    }
}

#[test]
fn many_frame_widgets_render_at_most_two_previews_at_once_and_every_one_ends_on_its_newest_request() {
    let _gpu = crate::frames::gpu_slot_test();
    let mut harness = previews(session_with_comp(960, 540, "#00ff00"), 8, [80.0, 45.0], 1.0);
    let mut most = 0;
    for step in 0..20 {
        harness.state_mut().2 = step;
        harness.step();
        most = most.max(harness.state().0.running.load(std::sync::atomic::Ordering::SeqCst));
    }
    for _ in 0..4_000 {
        harness.step();
        most = most.max(harness.state().0.running.load(std::sync::atomic::Ordering::SeqCst));
        let images = &harness.state().0;
        let newest = |preview: &super::Preview| preview.pending.is_none() && preview.waiting.is_none() && matches!(preview.shown, Ok(Some(_)));
        if images.frames.len() == 8 && images.frames.values().all(newest) {
            assert!(most <= super::RUNNING_PREVIEWS, "{most} previews rendered at once");
            return;
        }
        std::thread::sleep(std::time::Duration::from_millis(5));
    }
    let images = &harness.state().0;
    let states: Vec<String> = images
        .frames
        .values()
        .map(|p| format!("{:?} {:?} {:?} {:?}", p.pending.is_some(), p.waiting, p.shown.as_ref().map(|s| s.is_some()), p.asked.as_ref().map(|a| a.1.len())))
        .collect();
    panic!("not every preview rendered its newest request: {} {states:?}", images.running.load(std::sync::atomic::Ordering::SeqCst));
}

#[test]
fn a_preview_started_for_one_project_never_shows_after_another_project_opens() {
    let _gpu = crate::frames::gpu_slot_test();
    let mut harness = previews(session_with_comp(320, 180, "#ff0000"), 1, [160.0, 90.0], 1.0);
    harness.step();
    harness.state_mut().1 = session_with_comp(320, 180, "#0000ff");
    let colours = |harness: &mut Harness<'_, Previewed>| {
        let image = harness.render().expect("the harness renders");
        let red = image.pixels().any(|pixel| pixel[0] > 200 && pixel[1] < 60 && pixel[2] < 60);
        let blue = image.pixels().any(|pixel| pixel[2] > 200 && pixel[0] < 60 && pixel[1] < 60);
        (red, blue)
    };
    for _ in 0..400 {
        harness.step();
        let (red, blue) = colours(&mut harness);
        assert!(!red, "the first project's frame showed after the second opened");
        if blue {
            return;
        }
        std::thread::sleep(std::time::Duration::from_millis(5));
    }
    panic!("the second project's frame never showed");
}
