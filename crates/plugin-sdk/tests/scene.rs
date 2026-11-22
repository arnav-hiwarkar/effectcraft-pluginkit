use craft_plugin::{RenderCx, Scene};

fn cx(size: [u32; 2], scale: f64, offset: [f64; 2]) -> RenderCx {
    RenderCx {
        params: Default::default(),
        time: 0.0,
        comp_time: 0.0,
        frame_rate: 24.0,
        scale,
        layer_size: [640.0, 360.0],
        offset,
        size,
        instance: 0,
        float: true,
        linear: true,
    }
}

fn pixel_through_clip_space(scene: &Scene, cx: &RenderCx, point: [f64; 3]) -> ([f64; 2], f64) {
    let m = scene.view_projection(cx).expect("a scene with a projection");
    let p = [point[0] as f32, point[1] as f32, point[2] as f32, 1.0];
    let clip: [f64; 4] = std::array::from_fn(|row| (0..4).map(|col| f64::from(m[col * 4 + row] * p[col])).sum());
    let ndc = [clip[0] / clip[3], clip[1] / clip[3], clip[2] / clip[3]];
    ([(ndc[0] + 1.0) / 2.0 * f64::from(cx.size[0]), (1.0 - ndc[1]) / 2.0 * f64::from(cx.size[1])], ndc[2])
}

#[test]
fn the_centred_camera_leaves_the_layer_plane_where_it_is() {
    let scene = Scene::centred([640.0, 360.0]);
    assert_eq!(scene.project([100.0, 50.0, 0.0]), Some([100.0, 50.0]));
    assert_eq!(scene.project([320.0, 180.0, 5000.0]), Some([320.0, 180.0]));
    let camera = scene.camera.expect("a camera");
    assert_eq!(camera.position, [320.0, 180.0, -640.0 * 50.0 / 36.0]);
}

#[test]
fn view_projection_lands_on_the_pixel_project_gives() {
    let mut scene = Scene::centred([640.0, 360.0]);
    let tilted = [[0.9, 0.1, 0.35, 12.0], [-0.05, 1.1, 0.2, -7.0], [0.0002, -0.0001, 0.0011, 1.0]];
    for projection in [scene.projection.expect("a projection"), tilted] {
        scene.projection = Some(projection);
        for cx in [cx([640, 360], 1.0, [0.0, 0.0]), cx([352, 212], 0.5, [16.0, 16.0])] {
            for point in [[320.0, 180.0, 0.0], [10.0, 300.0, 250.0], [600.0, 20.0, -400.0], [-50.0, 400.0, 900.0]] {
                let [x, y] = scene.project(point).expect("in front of the camera");
                let expected = [x * cx.scale + cx.offset[0], y * cx.scale + cx.offset[1]];
                let (pixel, depth) = pixel_through_clip_space(&scene, &cx, point);
                assert!((pixel[0] - expected[0]).abs() < 0.01 && (pixel[1] - expected[1]).abs() < 0.01, "{point:?}: {pixel:?} against {expected:?}");
                assert!((0.0..=1.0).contains(&depth), "{point:?}: depth {depth}");
            }
        }
    }
}

#[test]
fn clip_depth_is_one_at_the_near_plane_and_falls_with_distance() {
    let scene = Scene::centred([720.0, 360.0]);
    let cx = cx([720, 360], 1.0, [0.0, 0.0]);
    let depths: Vec<f64> = [-999.0, -500.0, 0.0, 1000.0, 9000.0].iter().map(|z| pixel_through_clip_space(&scene, &cx, [360.0, 180.0, *z]).1).collect();
    let expected = [1.0, 1.0 / 500.0, 1.0 / 1000.0, 1.0 / 2000.0, 1.0 / 10000.0];
    for (depth, expected) in depths.iter().zip(expected) {
        assert!((depth - expected).abs() < expected * 1e-4, "{depths:?}");
    }
}
