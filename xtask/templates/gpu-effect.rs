use craft_plugin::prelude::*;

const KERNELS: &str = r#"
@compute @workgroup_size(16, 16)
fn {{snake}}_tint(@builtin(global_invocation_id) id: vec3<u32>) {
    if (!inside_out(id.xy)) { return; }
    let c = texel(src, vec2<i32>(id.xy));
    let tinted = c.rgb * vec3<f32>(pass_f(1u), pass_f(2u), pass_f(3u));
    store(id.xy, vec4<f32>(mix(c.rgb, tinted, pass_f(0u)), c.a));
}
"#;

params! {
    struct Settings {
        amount: f64 = percent("Amount", 100.0),
        tint: [f32; 4] = color("Tint", [0.3, 0.7, 1.0, 1.0]),
    }
}

struct Tint;

impl Effect for Tint {
    fn spec(&self) -> EffectSpec {
        EffectSpec::new("{{name}}", "{{Title}}", "Color Correction").params(Settings::params()).shader(KERNELS)
    }

    fn layout(&self, cx: &Cx) -> Layout {
        let settings = Settings::read(cx);
        let [r, g, b, _] = settings.tint;
        let tint = GpuPass::new("{{snake}}_tint", GpuImage::Source, GpuImage::Output).floats(&[(settings.amount / 100.0) as f32, r, g, b]);
        Layout::pointwise().passes(GpuPlan::new().pass(tint))
    }

    fn pass(&self, _cx: &Cx, pass: &Pass, out: &mut Frame) -> Result<()> {
        let &[mix, r, g, b] = pass.f else { return Err(format!("kernel `{}` expects 4 floats", pass.kernel).into()) };
        let gain = [r, g, b, 1.0];
        for (y, row) in out.rows_mut() {
            for (x, pixel) in row.iter_mut().enumerate() {
                let c = pass.src.at(x as isize, y as isize);
                *pixel = std::array::from_fn(|i| c[i] + (c[i] * gain[i] - c[i]) * mix);
            }
        }
        Ok(())
    }
}

fn plugin() -> Plugin {
    Plugin::new().effect(Tint)
}

craft_plugin::export!(plugin);

#[cfg(test)]
mod tests {
    use craft_plugin_host::assert_frame;
    use craft_plugin_host::testing::TestPlugin;

    #[test]
    fn the_cpu_pass_tints_the_test_image() {
        let plugin = TestPlugin::of(super::plugin);
        assert_frame!(plugin.render("{{name}}").set("amount", 100.0), "full");
    }

    #[test]
    fn the_gpu_plan_passes_the_check_the_app_runs() {
        let plugin = TestPlugin::of(super::plugin);
        let render = plugin.render("{{name}}");
        let plan = render.layout().unwrap().gpu.unwrap();
        let source = render.source();
        assert_eq!(plan.validate([source.width, source.height]), Ok(()));
    }
}
