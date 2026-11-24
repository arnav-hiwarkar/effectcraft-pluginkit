use craft_plugin_host::testing::{Picture, TestPlugin};

const SIZE: [u32; 2] = [128, 64];
const SIDE: f32 = 24.0;
const STEP: f32 = 16.0;

fn scene(left: f32) -> Picture {
    let mut picture = Picture::new(SIZE[0], SIZE[1]);
    for y in 0..SIZE[1] {
        for x in 0..SIZE[0] {
            let (fx, fy) = (x as f32, y as f32);
            let ground = 0.4 + 0.05 * (fx * 0.21 + fy * 0.13).sin() + 0.05 * (fy * 0.27 - fx * 0.09).sin();
            let (u, v) = (fx - left, fy - 20.0);
            let inside = (0.0..SIDE).contains(&u) && (0.0..SIDE).contains(&v);
            let value = if inside { 0.75 + 0.2 * (u * 0.6).sin() * (v * 0.5).cos() } else { ground };
            picture.pixels[(y * SIZE[0] + x) as usize] = [value, value, value, 1.0];
        }
    }
    picture
}

fn frame_at(time: f64) -> Picture {
    scene(20.0 + STEP * (time * 30.0).round() as f32)
}

fn square_error(picture: &Picture, left: f32) -> f32 {
    let expected = scene(left);
    let inside = |i: usize| {
        let (x, y) = ((i as u32 % SIZE[0]) as f32, (i as u32 / SIZE[0]) as f32);
        (left..left + SIDE).contains(&x) && (20.0..20.0 + SIDE).contains(&y)
    };
    let errors: Vec<f32> = (0..picture.pixels.len()).filter(|i| inside(*i)).map(|i| (picture.pixels[i][0] - expected.pixels[i][0]).abs()).collect();
    errors.iter().sum::<f32>() / errors.len() as f32
}

fn best_fit(picture: &Picture) -> (f32, f32) {
    (0..=64)
        .map(|quarter| 36.0 + quarter as f32 * 0.25)
        .map(|left| (left, square_error(picture, left)))
        .fold((0.0, f32::MAX), |best, next| if next.1 < best.1 { next } else { best })
}

#[test]
fn the_motion_vector_midpoint_of_a_moving_square_lands_within_a_pixel_of_halfway_and_frame_blending_does_not() {
    let plugin = TestPlugin::of(crate::plugin);
    let render = |method: f64| {
        let picture = plugin.render("timewarp").input(scene(0.0)).source_at(frame_at).set("speed", 0.5).set("method", method).time(3.0 / 30.0).run().unwrap();
        best_fit(&picture)
    };
    let (left, error) = render(0.0);
    assert!((left - 44.0).abs() <= 1.0 && error < 0.05, "motion vectors put the square at {left} with mean error {error}");
    let (_, blended) = render(1.0);
    assert!(blended > 3.0 * error, "frame blending leaves two half squares: mean error {blended} against {error}");
}

#[test]
fn a_whole_source_frame_is_shown_unchanged() {
    let plugin = TestPlugin::of(crate::plugin);
    let picture = plugin.render("timewarp").input(scene(0.0)).source_at(frame_at).set("speed", 0.5).time(4.0 / 30.0).run().unwrap();
    assert!(picture.pixels == scene(52.0).pixels, "source frame 2 at time 4 frames and half speed");
}
