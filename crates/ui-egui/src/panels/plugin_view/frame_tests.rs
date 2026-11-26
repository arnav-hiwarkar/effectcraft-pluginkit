use egui_kittest::Harness;
use egui_kittest::kittest::{NodeT, Queryable};
use serde_json::json;

use super::panel_tests::{shows, until};
use crate::EffectcraftApp;
use crate::dock::PanelKind;
use crate::panels::plugin_tools::tests::{harness, probe_session};

const PROBE: &str = "test.viewer-tool-ui";
const PREVIEW: &str = "plugin.test.viewer-tool-ui.preview";

fn open_preview() -> Harness<'static, EffectcraftApp> {
    let mut session = probe_session();
    session.execute("layer.newSolid", json!({"name": "Red", "color": "#ff0000"})).unwrap();
    let mut harness = harness(session);
    let preview = PanelKind::plugin(PROBE, "preview");
    until(&mut harness, "the preview panel registering", |h| super::running_panel(h.state(), preview).is_some());
    let ctx = harness.ctx.clone();
    crate::menus::invoke(harness.state_mut(), &ctx, "window.pluginPanel", json!({"plugin": PROBE, "panel": "preview"})).unwrap();
    until(&mut harness, "the preview showing", |h| shows(h, "0 clicks"));
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
    let mut harness = open_preview();
    until(&mut harness, "the red comp in the frame", |h| near(colour_at_frame_centre(h), [255, 0, 0]));
    let calls = harness.state().plugin_ui.panels.calls(PREVIEW);
    harness.state_mut().session.execute("layer.newSolid", json!({"name": "Blue", "color": "#0000ff"})).unwrap();
    until(&mut harness, "the edit redrawing the frame", |h| near(colour_at_frame_centre(h), [0, 0, 255]));
    assert_eq!(harness.state().plugin_ui.panels.calls(PREVIEW), calls, "the app redrew the frame without asking the plugin");
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
    let mut harness = open_preview();
    let field = || egui_kittest::kittest::by().predicate(|node| node.placeholder() == Some("Composition"));
    harness.get(field()).click();
    harness.step();
    harness.get(field()).type_text("Nowhere");
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
            let source = super::Source { plugin: PROBE, images, session: Some(session) };
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
