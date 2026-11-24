use craft_plugin_host::assert_frame;
use craft_plugin_host::testing::TestPlugin;

#[test]
fn the_linear_ramp_matches_its_stored_frame() {
    let plugin = TestPlugin::of(crate::plugin);
    assert_frame!(plugin.render("ramp").size(160, 90), "ramp-linear");
}

#[test]
fn the_radial_ramp_matches_its_stored_frame() {
    let plugin = TestPlugin::of(crate::plugin);
    assert_frame!(plugin.render("ramp").size(160, 90).set("shape", 1.0), "ramp-radial");
}

#[test]
fn the_noise_field_matches_its_stored_frame() {
    let plugin = TestPlugin::of(crate::plugin);
    assert_frame!(plugin.render("field").size(160, 90).set("scale", 24.0), "field");
}

#[test]
fn the_dots_match_their_stored_frame() {
    let plugin = TestPlugin::of(crate::plugin);
    assert_frame!(plugin.render("dots").size(160, 90).set("columns", 3.0).set("rows", 2.0).set("spacing", 40.0).set("radius", 12.0), "dots");
}

#[test]
fn the_dots_leave_everything_outside_their_grid_transparent() {
    let plugin = TestPlugin::of(crate::plugin);
    let picture = plugin.render("dots").size(160, 90).set("columns", 1.0).set("rows", 1.0).set("radius", 10.0).run().unwrap();
    assert_eq!(picture.at(80, 45)[3], 1.0);
    assert_eq!(picture.at(80, 20), [0.0; 4]);
    assert_eq!(picture.at(10, 45), [0.0; 4]);
}

#[test]
fn the_noise_field_plan_passes_the_apps_check() {
    let plugin = TestPlugin::of(crate::plugin);
    let layout = plugin.render("field").size(160, 90).layout().unwrap();
    assert_eq!(layout.gpu.unwrap().validate([160, 90]), Ok(()));
}

#[test]
fn every_effect_renders_the_same_in_bands_as_whole_at_an_odd_height() {
    let plugin = TestPlugin::of(crate::plugin);
    for effect in ["ramp", "field", "dots"] {
        assert_eq!(plugin.render(effect).size(40, 257).run().err(), None, "{effect}");
    }
}

#[test]
fn every_effect_that_renders_differently_over_time_says_so() {
    let plugin = TestPlugin::of(crate::plugin);
    for effect in ["ramp", "field", "dots"] {
        assert_eq!(plugin.render(effect).size(160, 90).time_check(), Ok(()), "{effect}");
    }
}
