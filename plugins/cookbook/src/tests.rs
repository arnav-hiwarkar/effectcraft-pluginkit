use craft_plugin::prelude::*;
use craft_plugin::{AppEvent, Shape, ToolEventKind};
use craft_plugin_host::assert_frame;
use craft_plugin_host::testing::{Pattern, TestPlugin, json, pointer};

pub struct Smear;

impl Effect for Smear {
    fn spec(&self) -> EffectSpec {
        EffectSpec::new("smear", "Smear", "Cookbook")
    }

    fn layout(&self, _cx: &Cx) -> Layout {
        Layout::pointwise()
    }

    fn render(&self, _cx: &Cx, frame: &mut Frame) -> Result<()> {
        let (left, input) = (frame.origin[0], frame.input);
        for (y, row) in frame.rows_mut() {
            for (x, pixel) in row.iter_mut().enumerate() {
                *pixel = input.at((left + x) as isize, y as isize - 3);
            }
        }
        Ok(())
    }
}

pub struct Crash;

impl Effect for Crash {
    fn spec(&self) -> EffectSpec {
        EffectSpec::new("crash", "Crash", "Cookbook")
    }

    fn render(&self, _cx: &Cx, frame: &mut Frame) -> Result<()> {
        let past_the_end = frame.pixels.len();
        frame.pixels[past_the_end] = [1.0; 4];
        Ok(())
    }
}

#[test]
fn a_panic_reaches_the_test_and_the_log_with_its_file_and_line() {
    let plugin = TestPlugin::of(crate::plugin);
    let error = plugin.render("crash").size(4, 2).run().unwrap_err();
    assert_eq!(error, "the plugin panicked at src/tests.rs:37:9: index out of bounds: the len is 8 but the index is 8");
    assert!(plugin.logs().contains(&format!("render of effect `crash` failed: {error}")), "{:?}", plugin.logs());
}

#[test]
fn tint_matches_its_stored_frame() {
    let plugin = TestPlugin::of(crate::plugin);
    assert_frame!(plugin.render("tint").set("amount", 100.0), "tint");
}

#[test]
fn tint_offers_a_cold_preset() {
    let plugin = TestPlugin::of(crate::plugin);
    let spec = plugin.plugin().contributions().effects.iter().find(|spec| spec.id == "tint").unwrap().clone();
    assert_eq!(spec.presets[0].name, "Cold");
}

#[test]
fn a_test_renders_on_a_chosen_image_and_compares_with_a_stored_frame() {
    let plugin = TestPlugin::of(crate::plugin);
    let render = plugin.render("blur").on(Pattern::Checker).size(160, 90).set("radius", 6.0);
    assert_frame!(render, "blur-checker");
}

#[test]
fn blur_turns_the_edge_between_two_checker_cells_grey() {
    let plugin = TestPlugin::of(crate::plugin);
    let blurred = plugin.render("blur").on(Pattern::Checker).size(160, 90).set("radius", 6.0).run().unwrap();
    let edge = blurred.at(11, 5)[0];
    assert!((0.3..0.7).contains(&edge), "{edge}");
}

#[test]
fn glow_pads_the_layer_by_its_radius() {
    let plugin = TestPlugin::of(crate::plugin);
    let glow = plugin.render("glow").set("radius", 24.0).run().unwrap();
    assert_eq!([glow.width, glow.height], [320 + 48, 180 + 48]);
    assert_frame!(plugin.render("glow").set("radius", 24.0), "glow");
}

#[test]
fn posterize_runs_its_cpu_pass_and_its_plan_passes_the_app_check() {
    let plugin = TestPlugin::of(crate::plugin);
    let render = plugin.render("posterize").on(Pattern::Gradient).set("levels", 2.0);
    let poster = render.run().unwrap();
    assert_eq!(poster.at(10, 10), [0.0, 0.0, 1.0, 1.0]);
    assert_eq!(render.layout().unwrap().gpu.unwrap().validate([320, 180]), Ok(()));
}

#[test]
fn matte_darkens_where_the_matte_layer_is_dark() {
    let plugin = TestPlugin::of(crate::plugin);
    let white = Pattern::Checker.picture(320, 180);
    let matted = plugin.render("matte").on(Pattern::Shapes).layer("matte", white).run().unwrap();
    assert_eq!(matted.at(0, 0), [0.0; 4]);
    let untouched = plugin.render("matte").on(Pattern::Shapes).run().unwrap();
    assert_eq!(untouched, plugin.render("matte").on(Pattern::Shapes).source());
}

#[test]
fn edges_reads_its_neighbours_within_the_reach_it_declares() {
    let plugin = TestPlugin::of(crate::plugin);
    assert_frame!(plugin.render("edges").set("width", 3.0), "edges");
}

#[test]
fn an_effect_that_reads_rows_it_did_not_declare_is_caught_by_the_test_render() {
    let plugin = TestPlugin::of(crate::plugin);
    let error = plugin.render("smear").run().unwrap_err();
    assert!(error.starts_with("effect `smear` declares that each row reads rows at most 0 away, but rendering in bands changed pixel"), "{error}");
}

#[test]
fn gamma_remembers_its_table_between_frames() {
    let plugin = TestPlugin::of(crate::plugin);
    let first = plugin.render("gamma").set("gamma", 2.0).run().unwrap();
    assert!(plugin.plugin().cached("1/table/2").is_some());
    assert_eq!(plugin.render("gamma").set("gamma", 2.0).run().unwrap(), first);
}

#[test]
fn spotlight_draws_a_point_handle_and_a_radius_handle() {
    let plugin = TestPlugin::of(crate::plugin);
    let overlay = plugin.render("spotlight").set("centre", [100.0, 50.0]).set("radius", 30.0).overlay().unwrap();
    let handles: Vec<[f64; 2]> = overlay.handles.iter().map(|handle| handle.at).collect();
    assert_eq!(handles, [[100.0, 50.0], [130.0, 50.0]]);
}

#[test]
fn the_rig_button_adds_two_nulls_in_one_undo_step_and_logs_where() {
    let plugin = TestPlugin::of(crate::plugin);
    plugin.panel("rig").unwrap().click("Add Rig").unwrap();
    let (command, params) = &plugin.commands()[0];
    assert_eq!((command.as_str(), &params["label"]), ("engine.batch", &json!("Add Rig")));
    assert_eq!(plugin.logs(), ["src/nulls.rs:9: added a rig of two nulls"]);
    plugin.command("add-rig").unwrap();
    assert_eq!(plugin.commands().len(), 2);
}

#[test]
fn follow_shows_the_selection_and_the_time() {
    let plugin = TestPlugin::of(crate::plugin);
    let mut panel = plugin.panel("follow").unwrap();
    panel.app_event(AppEvent::Selection { layers: vec![1, 2, 3], properties: vec![], keyframes: 0 }).unwrap();
    panel.app_event(AppEvent::Time { comp: 1, time: 1.5 }).unwrap();
    assert_eq!(panel.texts(), ["3 layers selected at 1.50 s"]);
}

#[test]
fn the_measure_task_reports_the_average_brightness() {
    let plugin = TestPlugin::of(crate::plugin);
    let brightness = plugin.task("measure", json!({ "frames": 2 })).unwrap()["brightness"].as_f64().unwrap();
    assert!((0.3..0.6).contains(&brightness), "{brightness}");
    let mut meter = plugin.panel("meter").unwrap();
    meter.click("Measure").unwrap();
    meter.timer("progress").unwrap();
    assert!(!meter.texts().contains(&"Measure".to_string()), "{:?}", meter.texts());
}

#[test]
fn the_pin_tool_adds_a_null_where_it_is_clicked_and_rings_the_pointer() {
    let plugin = TestPlugin::of(crate::plugin);
    plugin.answer("layer.newNull", json!({ "layer": 3 }));
    let overlay = plugin.tool("pin", pointer(ToolEventKind::Down, [10.0, 20.0])).unwrap();
    let position = json!({ "layer": 3, "path": "transform/position", "value": [10.0, 20.0, 0.0] });
    assert_eq!(plugin.commands(), [("layer.newNull".to_string(), json!({ "name": "Pin 1" })), ("prop.set".to_string(), position)]);
    let Shape::Circle { center, radius, .. } = &overlay.marks[1].shape else { panic!("{:?}", overlay.marks) };
    assert_eq!((*center, *radius), ([10.0, 20.0], 8.0));
}

pub struct Reversed;

impl Effect for Reversed {
    fn spec(&self) -> EffectSpec {
        EffectSpec::new("reversed", "Reversed", "Cookbook").param(Param::slider("gain", "Gain", 1.0, 10.0..=1.0))
    }
}

#[test]
fn an_effect_with_a_reversed_range_is_refused_when_it_loads() {
    let error = TestPlugin::in_process(|| Plugin::new().effect(Reversed)).err();
    let expected = "the plugin could not be loaded: effect `reversed`: parameter `gain` has the range 10..=1; give the smaller end first";
    assert_eq!(error.as_deref(), Some(expected));
}

#[test]
fn two_harnesses_in_one_test_each_have_their_own_plugin_and_data_folder() {
    let (first, second) = (TestPlugin::of(crate::plugin), TestPlugin::of(crate::plugin));
    for plugin in [&first, &second] {
        plugin.answer("layer.newNull", json!({ "layer": 3 }));
        plugin.tool("pin", pointer(ToolEventKind::Down, [1.0, 1.0])).unwrap();
    }
    let named = |plugin: &TestPlugin| plugin.commands().into_iter().find(|(id, _)| id == "layer.newNull").map(|(_, params)| params);
    assert_eq!((named(&first), named(&second)), (Some(json!({ "name": "Pin 1" })), Some(json!({ "name": "Pin 1" }))));
    assert_ne!(first.data_dir(), second.data_dir());
}

#[test]
fn every_effect_renders_the_same_in_bands_as_whole_at_an_odd_height() {
    let plugin = craft_plugin_host::testing::TestPlugin::of(crate::plugin);
    for effect in plugin.plugin().contributions().effects.iter().map(|spec| spec.id.clone()).filter(|id| !["smear", "crash"].contains(&id.as_str())) {
        assert_eq!(plugin.render(&effect).size(40, 257).run().err(), None, "{effect}");
    }
}
