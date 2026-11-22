pub mod math;
pub mod mesh;
pub mod obj;
pub mod scene;

use craft_plugin::prelude::*;

use crate::mesh::Mesh;
use crate::scene::{Placement, Replicator};

const SHADER: &str = include_str!("forge.wgsl");
const MODELS: [&str; 5] = ["Cube", "Sphere", "Torus", "Star", "Gem (OBJ file)"];
const OBJ_PATH: &str = "assets/gem.obj";
const MAX_COPIES: f64 = 24.0;

struct Forge;

fn stored_mesh(model: usize) -> (String, String) {
    let keys = (format!("forge-3d/mesh/{model}/vertices"), format!("forge-3d/mesh/{model}/indices"));
    if host::cache_has(&keys.0) && host::cache_has(&keys.1) {
        return keys;
    }
    let mesh = build(model);
    let _ = host::cache_put(&keys.0, &mesh.vertex_bytes()).and_then(|()| host::cache_put(&keys.1, &mesh.index_bytes()));
    keys
}

fn build(model: usize) -> Mesh {
    match model {
        0 => mesh::cube(),
        1 => mesh::sphere(),
        2 => mesh::torus(),
        3 => mesh::star(),
        _ => {
            let parsed = host::read_file(Root::Bundle, OBJ_PATH).map_err(|e| e.to_string()).and_then(|bytes| obj::parse(&String::from_utf8_lossy(&bytes)));
            parsed.unwrap_or_else(|why| {
                host::log(craft_plugin::LogLevel::Warn, format!("Forge 3D shows a star because {OBJ_PATH} could not be used: {why}"));
                mesh::star()
            })
        }
    }
}

params! {
    struct Controls {
        model: usize = popup("Model", &MODELS, 2),
        "Transform" {
            position: [f64; 3] = point3("Position", [0.5, 0.5, 0.0]),
            rotation_x: f32 = angle("X Rotation", 20.0),
            rotation_y: f32 = angle("Y Rotation", 30.0),
            rotation_z: f32 = angle("Z Rotation", 0.0),
            size: f64 = slider("Size", 150.0, 1.0..=2000.0).log(),
            scale: f64 = percent("Scale", 100.0),
        }
        "Replicator" {
            shape: usize = popup("Shape", &["Off", "Grid", "Ring"], 0),
            columns: f64 = slider("Copies", 5.0, 1.0..=MAX_COPIES),
            rows: f64 = slider("Rows", 1.0, 1.0..=MAX_COPIES),
            spacing_x: f32 = slider("Spacing X", 320.0, -4000.0..=4000.0),
            spacing_y: f32 = slider("Spacing Y", 320.0, -4000.0..=4000.0),
            step_z: f32 = slider("Step Z", 0.0, -4000.0..=4000.0),
            radius: f32 = slider("Ring Radius", 500.0, 0.0..=8000.0).soft(0.0..=2000.0),
            ramp: bool = checkbox("Colour Ramp", false),
            last_colour: [f32; 4] = color("Last Colour", [0.2, 0.5, 1.0, 1.0]),
        }
        "Material" {
            colour: [f32; 4] = color("Base Colour", [0.9, 0.45, 0.15, 1.0]),
            specular: f64 = percent("Specular", 60.0),
            roughness: f64 = percent("Roughness", 35.0),
            rim: f64 = percent("Rim", 15.0),
        }
        "Lighting" {
            comp_lights: bool = checkbox("Use Comp Lights", true),
            ambient: f64 = percent("Ambient", 15.0),
        }
        smooth: bool = checkbox("Smooth Edges", true),
        show_layer: bool = checkbox("Show Layer", true),
    }
}

fn position(cx: &Cx, [x, y, z]: [f64; 3]) -> [f32; 3] {
    let scale = cx.scale.max(1e-9);
    [((x - cx.offset[0]) / scale) as f32, ((y - cx.offset[1]) / scale) as f32, (z / scale) as f32]
}

fn replicator(c: &Controls) -> Replicator {
    let copies = |value: f64| value.clamp(1.0, MAX_COPIES) as u32;
    match c.shape {
        1 => Replicator::Grid { columns: copies(c.columns), rows: copies(c.rows), spacing: [c.spacing_x, c.spacing_y, c.step_z] },
        2 => Replicator::Ring { count: copies(c.columns), radius: c.radius },
        _ => Replicator::Off,
    }
}

fn plan(cx: &Cx) -> Option<GpuPlan> {
    let scene = cx.scene().filter(|scene| scene.projection.is_some()).unwrap_or_else(|| Scene::centred(cx.layer_size));
    let view_projection = scene.view_projection(cx)?;
    let eye = scene.camera.map_or_else(|| Scene::centred(cx.layer_size).camera.unwrap_or_default().position, |camera| camera.position);
    let c = Controls::read(cx);
    let placement =
        Placement { position: position(cx, c.position), rotation: [c.rotation_x, c.rotation_y, c.rotation_z], size: (c.size * c.scale / 100.0) as f32 };
    let last = if c.ramp { c.last_colour } else { c.colour };
    let instances = scene::instances(&placement, replicator(&c), c.colour, last);
    let lights = scene::lights(&scene, c.comp_lights);
    let light_count = (lights.len() / scene::FLOATS_PER_LIGHT) as u32;

    let mut floats = Vec::with_capacity(24 + lights.len());
    floats.extend(view_projection);
    floats.extend(eye.map(|c| c as f32));
    floats.push((c.ambient / 100.0) as f32);
    floats.push((c.specular / 100.0) as f32);
    floats.push(scene::shininess((c.roughness / 100.0) as f32));
    floats.push((c.rim / 100.0) as f32);
    floats.push(0.0);
    floats.extend(lights);

    let (vertices, indices) = stored_mesh(c.model.min(MODELS.len() - 1));
    let samples = if c.smooth { 2 } else { 1 };
    let mut plan = GpuPlan::new();
    let canvas = plan.scratch([cx.size[0] * samples, cx.size[1] * samples]);
    let vertices = plan.buffer(GpuData::Cache(vertices));
    let indices = plan.buffer(GpuData::Cache(indices));
    let instances = plan.buffer(GpuData::Floats(instances));
    let draw = GpuPass::draw("forge", canvas)
        .vertices(vertices, &mesh::LAYOUT)
        .indices(indices)
        .instances(instances, &scene::INSTANCE_LAYOUT)
        .depth()
        .clear()
        .floats(&floats)
        .uints(&[light_count]);
    let keep_layer = if c.show_layer { 1.0 } else { 0.0 };
    Some(plan.pass(draw).pass(GpuPass::new("forge_over", GpuImage::Source, GpuImage::Output).aux(canvas).floats(&[keep_layer])))
}

impl Effect for Forge {
    fn spec(&self) -> EffectSpec {
        EffectSpec::new("forge", "Forge 3D", "3D").params(Controls::params()).reads_scene().shader(SHADER)
    }

    fn layout(&self, cx: &Cx) -> Layout {
        match plan(cx) {
            Some(plan) => Layout::whole_frame().gpu(plan),
            None => Layout::whole_frame(),
        }
    }
}

fn plugin() -> Plugin {
    Plugin::new().effect(Forge)
}

craft_plugin::export!(plugin);

#[cfg(test)]
mod tests {
    use craft_plugin::GpuData;
    use craft_plugin_host::assert_frame;
    use craft_plugin_host::testing::TestPlugin;

    #[test]
    fn without_a_gpu_the_layer_is_left_as_it_is() {
        let plugin = TestPlugin::of(super::plugin);
        let render = plugin.render("forge");
        assert_eq!(render.run().unwrap(), render.source());
        assert_frame!(plugin.render("forge"), "layer-only");
    }

    #[test]
    fn a_three_by_two_grid_draws_six_copies_into_a_canvas_twice_the_frame_for_smooth_edges() {
        let plugin = TestPlugin::of(super::plugin);
        let render = plugin.render("forge").set("Replicator/shape", 1.0).set("Replicator/columns", 3.0).set("Replicator/rows", 2.0);
        let plan = render.layout().unwrap().gpu.expect("the effect plans its draw");
        assert_eq!(plan.validate([320, 180]), Ok(()));
        assert_eq!(plan.images, [[640, 360]]);
        assert_eq!(plan.passes.iter().map(|pass| pass.kernel.as_str()).collect::<Vec<_>>(), ["forge", "forge_over"]);
        let copies = plan.buffers.iter().find_map(|buffer| if let GpuData::Floats(floats) = buffer { Some(floats.len()) } else { None });
        assert_eq!(copies, Some(6 * 20));
        let plain = render.set("smooth", false).layout().unwrap().gpu.unwrap();
        assert_eq!(plain.images, [[320, 180]]);
    }
}
