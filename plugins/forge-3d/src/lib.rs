pub mod math;
pub mod mesh;
pub mod obj;
pub mod scene;

use std::collections::HashSet;
use std::sync::Mutex;

use craft_plugin::prelude::*;

use crate::mesh::Mesh;
use crate::scene::{Placement, Replicator};

const SHADER: &str = include_str!("forge.wgsl");
const MODELS: [&str; 5] = ["Cube", "Sphere", "Torus", "Star", "Gem (OBJ file)"];
const OBJ_PATH: &str = "assets/gem.obj";
const MAX_COPIES: f64 = 24.0;

struct Forge;

fn stored_mesh(model: usize) -> (String, String) {
    static STORED: Mutex<Option<HashSet<usize>>> = Mutex::new(None);
    let keys = (format!("forge-3d/mesh/{model}/vertices"), format!("forge-3d/mesh/{model}/indices"));
    let mut stored = STORED.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
    if stored.get_or_insert_with(HashSet::new).contains(&model) {
        return keys;
    }
    let mesh = build(model);
    if host::cache_put(&keys.0, &mesh.vertex_bytes()).and_then(|()| host::cache_put(&keys.1, &mesh.index_bytes())).is_ok() {
        stored.get_or_insert_with(HashSet::new).insert(model);
    }
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

fn position(cx: &Cx) -> [f32; 3] {
    let [x, y, z] = cx.point3("Transform/position");
    let scale = cx.scale.max(1e-9);
    [((x - cx.offset[0]) / scale) as f32, ((y - cx.offset[1]) / scale) as f32, (z / scale) as f32]
}

fn replicator(cx: &Cx) -> Replicator {
    let copies = |id: &str| cx.f(id).clamp(1.0, MAX_COPIES) as u32;
    match cx.index("Replicator/shape") {
        1 => Replicator::Grid {
            columns: copies("Replicator/columns"),
            rows: copies("Replicator/rows"),
            spacing: [cx.f("Replicator/spacing_x") as f32, cx.f("Replicator/spacing_y") as f32, cx.f("Replicator/step_z") as f32],
        },
        2 => Replicator::Ring { count: copies("Replicator/columns"), radius: cx.f("Replicator/radius") as f32 },
        _ => Replicator::Off,
    }
}

fn plan(cx: &Cx) -> Option<GpuPlan> {
    let scene = cx.scene().filter(|scene| scene.projection.is_some()).unwrap_or_else(|| Scene::centred(cx.layer_size));
    let view_projection = scene.view_projection(cx)?;
    let eye = scene.camera.map_or_else(|| Scene::centred(cx.layer_size).camera.unwrap_or_default().position, |camera| camera.position);
    let placement = Placement {
        position: position(cx),
        rotation: [cx.f("Transform/rotation_x") as f32, cx.f("Transform/rotation_y") as f32, cx.f("Transform/rotation_z") as f32],
        size: (cx.f("Transform/size") * cx.f("Transform/scale") / 100.0) as f32,
    };
    let first = cx.color("Material/colour");
    let last = if cx.bool("Replicator/ramp") { cx.color("Replicator/last_colour") } else { first };
    let instances = scene::instances(&placement, replicator(cx), first, last);
    let lights = scene::lights(&scene, cx.bool("Lighting/comp_lights"));
    let light_count = (lights.len() / scene::FLOATS_PER_LIGHT) as u32;

    let mut floats = Vec::with_capacity(24 + lights.len());
    floats.extend(view_projection);
    floats.extend(eye.map(|c| c as f32));
    floats.push((cx.f("Lighting/ambient") / 100.0) as f32);
    floats.push((cx.f("Material/specular") / 100.0) as f32);
    floats.push(scene::shininess((cx.f("Material/roughness") / 100.0) as f32));
    floats.push((cx.f("Material/rim") / 100.0) as f32);
    floats.push(0.0);
    floats.extend(lights);

    let (vertices, indices) = stored_mesh(cx.index("model").min(MODELS.len() - 1));
    let samples = if cx.bool("smooth") { 2 } else { 1 };
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
    let keep_layer = if cx.bool("show_layer") { 1.0 } else { 0.0 };
    Some(plan.pass(draw).pass(GpuPass::new("forge_over", GpuImage::Source, GpuImage::Output).aux(canvas).floats(&[keep_layer])))
}

impl Effect for Forge {
    fn spec(&self) -> EffectSpec {
        EffectSpec::new("forge", "Forge 3D", "3D")
            .param(Param::popup("model", "Model", &MODELS, 2))
            .group(
                "Transform",
                [
                    Param::point3("position", "Position", [0.5, 0.5, 0.0]),
                    Param::angle("rotation_x", "X Rotation", 20.0),
                    Param::angle("rotation_y", "Y Rotation", 30.0),
                    Param::angle("rotation_z", "Z Rotation", 0.0),
                    Param::slider("size", "Size", 150.0, 1.0..=2000.0),
                    Param::percent("scale", "Scale", 100.0),
                ],
            )
            .group(
                "Replicator",
                [
                    Param::popup("shape", "Shape", &["Off", "Grid", "Ring"], 0),
                    Param::slider("columns", "Copies", 5.0, 1.0..=MAX_COPIES),
                    Param::slider("rows", "Rows", 1.0, 1.0..=MAX_COPIES),
                    Param::slider("spacing_x", "Spacing X", 320.0, -4000.0..=4000.0),
                    Param::slider("spacing_y", "Spacing Y", 320.0, -4000.0..=4000.0),
                    Param::slider("step_z", "Step Z", 0.0, -4000.0..=4000.0),
                    Param::slider("radius", "Ring Radius", 500.0, 0.0..=8000.0),
                    Param::checkbox("ramp", "Colour Ramp", false),
                    Param::color("last_colour", "Last Colour", [0.2, 0.5, 1.0, 1.0]),
                ],
            )
            .group(
                "Material",
                [
                    Param::color("colour", "Base Colour", [0.9, 0.45, 0.15, 1.0]),
                    Param::percent("specular", "Specular", 60.0),
                    Param::percent("roughness", "Roughness", 35.0),
                    Param::percent("rim", "Rim", 15.0),
                ],
            )
            .group("Lighting", [Param::checkbox("comp_lights", "Use Comp Lights", true), Param::percent("ambient", "Ambient", 15.0)])
            .param(Param::checkbox("smooth", "Smooth Edges", true))
            .param(Param::checkbox("show_layer", "Show Layer", true))
            .reads_scene()
            .shader(SHADER)
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
