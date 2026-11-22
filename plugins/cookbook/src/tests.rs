use craft_plugin::prelude::*;
use craft_plugin::{AppEvent, Shape, ToolEventKind};
use craft_plugin_host::testing::{Pattern, json, pointer};
use craft_plugin_host::{assert_frame, this_plugin};

pub struct Smear;

impl Effect for Smear {
    fn spec(&self) -> EffectSpec {
        EffectSpec::new("smear", "Smear", "Cookbook")
    }

    fn layout(&self, _cx: &Cx) -> Layout {
        Layout::pointwise()
    }

    fn render(&self, _cx: &Cx, frame: &mut Frame) -> Result<()> {
        let width = frame.width;
        for y in (3..frame.height).rev() {
            let (above, here) = frame.pixels.split_at_mut(y * width);
            here[..width].copy_from_slice(&above[(y - 3) * width..(y - 2) * width]);
        }
        Ok(())
    }
}

#[test]
fn tint_matches_its_stored_frame() {
    assert_frame!("tint", { "amount": 100 }, "tint");
}

#[test]
fn tint_offers_a_cold_preset() {
    let plugin = this_plugin!();
    let spec = plugin.plugin().contributions().effects.iter().find(|spec| spec.id == "tint").unwrap().clone();
    assert_eq!(spec.presets[0].name, "Cold");
}

#[test]
fn a_test_renders_on_a_chosen_image_and_compares_with_a_stored_frame() {
    let plugin = this_plugin!();
    let render = plugin.render("blur").on(Pattern::Checker).size(160, 90).set("radius", 6.0);
    assert_frame!(render, "blur-checker");
}

#[test]
fn blur_turns_the_edge_between_two_checker_cells_grey() {
    let plugin = this_plugin!();
    let blurred = plugin.render("blur").on(Pattern::Checker).size(160, 90).set("radius", 6.0).run().unwrap();
    let edge = blurred.at(11, 5)[0];
    assert!((0.3..0.7).contains(&edge), "{edge}");
}

#[test]
fn glow_pads_the_layer_by_its_radius() {
    let plugin = this_plugin!();
    let glow = plugin.render("glow").set("radius", 24.0).run().unwrap();
    assert_eq!([glow.width, glow.height], [320 + 48, 180 + 48]);
    assert_frame!(plugin.render("glow").set("radius", 24.0), "glow");
}

#[test]
fn posterize_runs_its_cpu_pass_and_its_plan_passes_the_app_check() {
    let plugin = this_plugin!();
    let render = plugin.render("posterize").on(Pattern::Gradient).set("levels", 2.0);
    let poster = render.run().unwrap();
    assert_eq!(poster.at(10, 10), [0.0, 0.0, 1.0, 1.0]);
    assert_eq!(render.layout().unwrap().gpu.unwrap().validate([320, 180]), Ok(()));
}

#[test]
fn matte_darkens_where_the_matte_layer_is_dark() {
    let plugin = this_plugin!();
    let white = Pattern::Checker.picture(320, 180);
    let matted = plugin.render("matte").on(Pattern::Shapes).layer("matte", white).run().unwrap();
    assert_eq!(matted.at(0, 0), [0.0; 4]);
    let untouched = plugin.render("matte").on(Pattern::Shapes).run().unwrap();
    assert_eq!(untouched, plugin.render("matte").on(Pattern::Shapes).source());
}

#[test]
fn edges_reads_its_neighbours_within_the_reach_it_declares() {
    assert_frame!("edges", { "width": 3 }, "edges");
}

#[test]
fn an_effect_that_reads_rows_it_did_not_declare_is_caught_by_the_test_render() {
    let plugin = this_plugin!();
    let error = plugin.render("smear").run().unwrap_err();
    assert!(error.starts_with("effect `smear` declares that each row reads rows at most 0 away, but rendering in bands changed pixel"), "{error}");
}

#[test]
fn gamma_remembers_its_table_between_frames() {
    let plugin = this_plugin!();
    let first = plugin.render("gamma").set("gamma", 2.0).run().unwrap();
    assert!(plugin.plugin().cached("1/table/2").is_some());
    assert_eq!(plugin.render("gamma").set("gamma", 2.0).run().unwrap(), first);
}

#[test]
fn spotlight_draws_a_point_handle_and_a_radius_handle() {
    let plugin = this_plugin!();
    let overlay = plugin.render("spotlight").set("centre", [100.0, 50.0]).set("radius", 30.0).overlay().unwrap();
    let handles: Vec<[f64; 2]> = overlay.handles.iter().map(|handle| handle.at).collect();
    assert_eq!(handles, [[100.0, 50.0], [130.0, 50.0]]);
}

#[test]
fn the_rig_button_adds_two_nulls_in_one_undo_step_and_logs_where() {
    let plugin = this_plugin!();
    plugin.panel("rig").unwrap().click("Add Rig").unwrap();
    let (command, params) = &plugin.commands()[0];
    assert_eq!((command.as_str(), &params["label"]), ("engine.batch", &json!("Add Rig")));
    assert_eq!(plugin.logs(), ["src/nulls.rs:9: added a rig of two nulls"]);
    plugin.command("add-rig").unwrap();
    assert_eq!(plugin.commands().len(), 2);
}

#[test]
fn follow_shows_the_selection_and_the_time() {
    let plugin = this_plugin!();
    let mut panel = plugin.panel("follow").unwrap();
    panel.app_event(AppEvent::Selection { layers: vec![1, 2, 3], properties: vec![], keyframes: 0 }).unwrap();
    panel.app_event(AppEvent::Time { comp: 1, time: 1.5 }).unwrap();
    assert_eq!(panel.texts(), ["3 layers selected at 1.50 s"]);
}

#[test]
fn the_measure_task_reports_the_average_brightness() {
    let plugin = this_plugin!();
    let brightness = plugin.task("measure", json!({ "frames": 2 })).unwrap()["brightness"].as_f64().unwrap();
    assert!((0.3..0.6).contains(&brightness), "{brightness}");
    let mut meter = plugin.panel("meter").unwrap();
    meter.click("Measure").unwrap();
    meter.timer("progress").unwrap();
    assert!(!meter.texts().contains(&"Measure".to_string()), "{:?}", meter.texts());
}

#[test]
fn the_pin_tool_adds_a_null_and_rings_the_pointer() {
    let plugin = this_plugin!();
    let overlay = plugin.tool("pin", pointer(ToolEventKind::Down, [10.0, 20.0])).unwrap();
    assert_eq!(plugin.commands(), [("layer.newNull".to_string(), json!({ "name": "Pin 1" }))]);
    let Shape::Circle { center, radius, .. } = &overlay.marks[1].shape else { panic!("{:?}", overlay.marks) };
    assert_eq!((*center, *radius), ([10.0, 20.0], 8.0));
}
