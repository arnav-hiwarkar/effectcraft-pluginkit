#[path = "../examples/harness/mod.rs"]
mod harness;

use craft_plugin_host::MaskPath;
use harness::{Kind, Scenario, bundle, camera};

fn trail() -> Scenario {
    let mut scenario = Scenario::new([480, 270])
        .param("Emitter/type", 3.0)
        .param("Emitter/rate", 1500.0)
        .param("Emitter/spread", 60.0)
        .param("Physics/turbulence", 250.0)
        .param("Particle/shape", 1.0)
        .param("Render/blend", 1.0)
        .scene(camera([480.0, 270.0], 0.6, 600.0));
    scenario.masks.push(MaskPath { name: "Mask".into(), points: vec![[40.0, 200.0], [200.0, 120.0], [440.0, 160.0]], closed: false, inverted: false });
    scenario
}

fn identical_three_ways(kind: Kind) {
    let bundle = bundle(kind);
    let scenario = trail();
    let cold = scenario.load(&bundle).frame(40);
    let warm = scenario.load(&bundle);
    let in_order: Vec<_> = (0..=40).map(|f| warm.frame(f)).collect();
    let again = warm.frame(40);
    assert_eq!(cold.len(), 480 * 270);
    assert!(cold == in_order[40], "frame 40 differs between a cold render and an in-order render");
    assert!(cold == again, "frame 40 differs when rendered a second time");
    let lit = cold.iter().filter(|p| p[0] > 0.1).count();
    assert!(lit > 2000, "only {lit} pixels carry particles");
    assert!(in_order[39] != in_order[40]);
}

#[test]
fn a_frame_is_identical_cold_in_order_and_again_through_the_sandbox() {
    identical_three_ways(Kind::Wasm);
}

#[test]
fn a_frame_is_identical_cold_in_order_and_again_natively() {
    identical_three_ways(Kind::Native);
}

#[test]
fn the_comp_camera_changes_where_particles_land_and_flat_view_ignores_it() {
    let bundle = bundle(Kind::Wasm);
    let burst = Scenario::new([320, 180]).param("Emitter/type", 2.0).list("Emitter/position", &[160.0, 90.0, 0.0]).param("Emitter/spread", 100.0);
    let flat = burst.clone().load(&bundle).frame(15);
    let seen = burst.clone().scene(camera([320.0, 180.0], 0.6, 400.0)).load(&bundle).frame(15);
    let flat_view = burst.scene(camera([320.0, 180.0], 0.6, 400.0)).param("Render/view", 1.0).load(&bundle).frame(15);
    assert!(flat != seen);
    assert!(flat == flat_view);
}
