use craft_plugin::prelude::*;
use craft_plugin::{Modifiers, UiEvent, UiEventKind, UiInput, Widget};

use crate::control::{self, ControlState};
use crate::curve::{Curves, Lut, Luts, Spline, identity};
use crate::presets::{PRESETS, THUMB, thumbnail};
use crate::{CurvesEffect, PARAM};
use craft_plugin_host::assert_frame;
use craft_plugin_host::testing::TestPlugin;

const S_CURVE: &[[f64; 2]] = &[[0.0, 0.0], [0.3, 0.2], [0.7, 0.8], [1.0, 1.0]];

fn close(a: f32, b: f32) -> bool {
    (a - b).abs() < 1e-5
}

#[test]
fn the_identity_curve_leaves_pixels_unchanged_including_values_above_one() {
    let luts = Luts::new(&Curves::default());
    for pixel in [[0.2, 0.4, 0.1, 0.5], [2.5, 1.0, 0.0, 1.0], [0.0, 0.0, 0.0, 0.0], [0.03, 0.7, 0.9, 1.0]] {
        assert_eq!(luts.pixel(pixel), pixel);
    }
}

#[test]
fn an_identity_lookup_table_built_from_more_points_also_leaves_values_unchanged() {
    let lut = Lut::new(&[[0.0, 0.0], [0.5, 0.5], [1.0, 1.0]]);
    for v in [0.0f32, 0.1, 0.5, 0.9, 1.0, 1.5, 3.0] {
        assert!(close(lut.apply(v), v), "{v} -> {}", lut.apply(v));
    }
}

#[test]
fn the_s_curve_maps_a_quarter_and_three_quarters_to_the_monotone_cubic_values() {
    let spline = Spline::new(S_CURVE);
    assert!((spline.at(0.25) - 0.152_199_074_074_074_07).abs() < 1e-12);
    assert!((spline.at(0.75) - 0.847_800_925_925_926).abs() < 1e-12);
    let lut = Lut::new(S_CURVE);
    assert!(close(lut.apply(0.25), 0.152_199_07));
    assert!(close(lut.apply(0.75), 0.847_800_9));
}

#[test]
fn values_above_one_continue_the_last_segment_in_a_straight_line() {
    let lut = Lut::new(&[[0.0, 0.0], [1.0, 0.5]]);
    assert!(close(lut.apply(1.0), 0.5));
    assert!(close(lut.apply(2.0), 1.0));
    assert!(close(lut.apply(-1.0), -0.5));
}

#[test]
fn the_monotone_spline_does_not_overshoot_between_steep_points() {
    let spline = Spline::new(&[[0.0, 0.0], [0.1, 0.9], [0.2, 1.0], [1.0, 1.0]]);
    let highest = (0..=1000).map(|i| spline.at(i as f64 / 1000.0)).fold(f64::MIN, f64::max);
    assert!(highest <= 1.0 + 1e-12, "{highest}");
}

#[test]
fn colour_curves_apply_to_straight_colour_and_alpha_is_mapped_by_its_own_curve() {
    let mut curves = Curves::default();
    curves.channels[1] = vec![[0.0, 0.0], [1.0, 0.5]];
    curves.channels[4] = vec![[0.0, 0.0], [1.0, 0.5]];
    let out = Luts::new(&curves).pixel([0.4, 0.2, 0.1, 0.5]);
    let wanted = [0.8 * 0.5 * 0.25, 0.4 * 0.25, 0.2 * 0.25, 0.25];
    assert!(out.iter().zip(wanted).all(|(a, b)| close(*a, b)), "{out:?}");
}

#[test]
fn a_saved_value_round_trips_and_broken_text_falls_back_to_straight_lines() {
    let mut curves = Curves::default();
    curves.channels[0] = S_CURVE.to_vec();
    assert_eq!(Curves::parse(&curves.to_text()), curves);
    assert_eq!(Curves::parse("not json"), Curves::default());
    assert_eq!(Curves::parse(r#"{"channels":[[[0.5,0.5]],[],[],[],[]]}"#).channels[0], identity());
}

fn pointer(pos: [f32; 2], phase: PointerPhase, alt: bool) -> Pointer {
    Pointer { pos, phase, modifiers: Modifiers { alt, ..Default::default() }, scroll: [0.0; 2] }
}

#[test]
fn clicking_adds_a_point_dragging_moves_it_and_dragging_it_off_removes_it() {
    let side = 200.0;
    let mut state = ControlState::default();
    let mut points = identity();
    assert!(control::edit(&mut state, &mut points, pointer([100.0, 50.0], PointerPhase::Down, false), side));
    assert_eq!(points, vec![[0.0, 0.0], [0.5, 0.75], [1.0, 1.0]]);
    assert!(control::edit(&mut state, &mut points, pointer([60.0, 100.0], PointerPhase::Drag, false), side));
    assert!((points[1][0] - 0.3).abs() < 1e-6 && points[1][1] == 0.5, "{points:?}");
    assert!(control::edit(&mut state, &mut points, pointer([60.0, 300.0], PointerPhase::Drag, false), side));
    assert_eq!(points, identity());
    assert_eq!(state.dragging, None);
}

#[test]
fn alt_click_removes_the_point_under_the_pointer_but_never_the_last_two() {
    let side = 100.0;
    let mut state = ControlState::default();
    let mut points = vec![[0.0, 0.0], [0.5, 0.5], [1.0, 1.0]];
    assert!(control::edit(&mut state, &mut points, pointer([51.0, 49.0], PointerPhase::Down, true), side));
    assert_eq!(points, identity());
    assert!(!control::edit(&mut state, &mut points, pointer([0.0, 100.0], PointerPhase::Down, true), side));
    assert_eq!(points.len(), 2);
}

#[test]
fn a_dragged_point_stays_between_its_neighbours() {
    let side = 100.0;
    let mut state = ControlState::default();
    let mut points = vec![[0.0, 0.0], [0.4, 0.4], [0.6, 0.6], [1.0, 1.0]];
    control::edit(&mut state, &mut points, pointer([40.0, 60.0], PointerPhase::Down, false), side);
    control::edit(&mut state, &mut points, pointer([90.0, 60.0], PointerPhase::Drag, false), side);
    assert!((points[1][0] - 0.599).abs() < 1e-9, "{points:?}");
}

#[test]
fn the_control_returns_the_edited_value_and_shows_a_channel_popup_reset_button_and_canvas() {
    let input = UiInput { size: [260.0, 200.0], ..Default::default() };
    let mut ui = Ui::new(&input);
    let mut value = Curves::default_text();
    control::draw(&mut ControlState::default(), &mut value, &mut ui);
    let widgets = ui.finish();
    let Widget::Row { children } = &widgets[0] else { panic!("row expected: {widgets:?}") };
    assert!(matches!(&children[0], Widget::Popup { options, .. } if options.len() == 5));
    assert!(matches!(&children[1], Widget::Button { text, .. } if text == "Reset"));
    let Some(Widget::Canvas { id, size, .. }) = widgets.iter().find(|w| matches!(w, Widget::Canvas { .. })) else { panic!("canvas expected") };
    assert_eq!(*size, [252.0, 252.0]);
    let input = UiInput {
        events: vec![UiEvent { id: *id, kind: UiEventKind::Pointer(pointer([126.0, 63.0], PointerPhase::Down, false)) }],
        size: [260.0, 200.0],
        ..Default::default()
    };
    let mut ui = Ui::new(&input);
    control::draw(&mut ControlState::default(), &mut value, &mut ui);
    assert_eq!(Curves::parse(&value).channels[0], vec![[0.0, 0.0], [0.5, 0.75], [1.0, 1.0]]);
}

#[test]
fn a_thumbnail_is_opaque_and_draws_the_curve_in_white_where_the_curve_passes() {
    let rgba = thumbnail(&PRESETS[5]);
    assert_eq!(rgba.len(), (THUMB * THUMB * 4) as usize);
    assert!(rgba.chunks_exact(4).all(|p| p[3] == 255));
    let at = |x: u32, y: u32| &rgba[((y * THUMB + x) * 4) as usize..][..3];
    assert_eq!(at(THUMB / 2, THUMB / 2), [240, 240, 240]);
    assert_eq!(at(THUMB / 2 + 10, THUMB / 2 - 10)[0], 32);
}

#[test]
fn arrow_keys_move_the_chosen_point_within_its_neighbours_and_delete_removes_it() {
    let mut points = S_CURVE.to_vec();
    let mut state = ControlState::default();
    let side = 100.0;
    control::edit(&mut state, &mut points, Pointer::new([30.0, 80.0], PointerPhase::Down), side);
    control::edit(&mut state, &mut points, Pointer::new([30.0, 80.0], PointerPhase::Up), side);
    let press = |name: &str, shift: bool| Key { name: name.into(), modifiers: Modifiers { shift, alt: false, command: false } };
    assert!(control::key(&mut state, &mut points, &press("Up", true)));
    assert!(control::key(&mut state, &mut points, &press("Right", false)));
    assert!((points[1][0] - 0.31).abs() < 1e-9 && (points[1][1] - 0.3).abs() < 1e-9, "{:?}", points[1]);
    for _ in 0..100 {
        control::key(&mut state, &mut points, &press("Right", true));
    }
    assert!((points[1][0] - 0.699).abs() < 1e-9, "{:?}", points[1]);
    assert!(control::key(&mut state, &mut points, &press("Delete", false)));
    assert_eq!(points, vec![[0.0, 0.0], [0.7, 0.8], [1.0, 1.0]]);
    assert!(!control::key(&mut state, &mut points, &press("Left", false)));
}

fn shown_channel(effect: &CurvesEffect, instance: u64, events: Vec<UiEvent>) -> (u64, u32) {
    let input = UiInput { events, size: [260.0, 200.0], ..Default::default() };
    let mut ui = Ui::new(&input);
    effect.controls(instance, PARAM, &mut Curves::default_text(), &mut ui);
    let widgets = ui.finish();
    let Widget::Row { children } = &widgets[0] else { panic!("row expected: {widgets:?}") };
    let Widget::Popup { id, selected, .. } = &children[0] else { panic!("popup expected: {children:?}") };
    (*id, *selected)
}

#[test]
fn each_instance_of_the_effect_keeps_its_own_selected_channel() {
    let effect = CurvesEffect::default();
    let (popup, _) = shown_channel(&effect, 1, vec![]);
    assert_eq!(shown_channel(&effect, 1, vec![UiEvent { id: popup, kind: UiEventKind::Number(2.0) }]).1, 2);
    assert_eq!(shown_channel(&effect, 2, vec![]).1, 0);
    assert_eq!(shown_channel(&effect, 1, vec![]).1, 2);
}

#[test]
fn an_s_curve_on_the_master_and_a_lifted_red_channel_match_the_stored_frame() {
    let plugin = TestPlugin::of(crate::plugin);
    let curves = r#"{"channels":[[[0,0],[0.3,0.2],[0.7,0.8],[1,1]],[[0,0.1],[1,1]],[[0,0],[1,1]],[[0,0],[1,0.8]],[[0,0],[1,1]]]}"#;
    assert_frame!(plugin.render("curves").set(PARAM, curves), "s-curve-red-lift");
}

#[test]
fn the_default_curves_leave_every_pixel_as_it_was() {
    let plugin = TestPlugin::of(crate::plugin);
    let render = plugin.render("curves");
    assert_eq!(render.run().unwrap(), render.source());
}
