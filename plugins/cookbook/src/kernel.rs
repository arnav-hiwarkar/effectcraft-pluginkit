use craft_plugin::prelude::*;

const KERNELS: &str = r#"
@compute @workgroup_size(16, 16)
fn posterize(@builtin(global_invocation_id) id: vec3<u32>) {
    if (!inside_out(id.xy)) { return; }
    let c = texel(src, vec2<i32>(id.xy));
    let levels = pass_f(0u);
    store(id.xy, vec4<f32>(floor(c.rgb * levels + 0.5) / levels, c.a));
}
"#;

params! {
    struct Settings {
        levels: f64 = slider("Levels", 4.0, 2.0..=32.0),
    }
}

pub struct Posterize;

impl Effect for Posterize {
    fn spec(&self) -> EffectSpec {
        EffectSpec::new("posterize", "Posterize", "Cookbook").params(Settings::params()).shader(KERNELS)
    }

    fn layout(&self, cx: &Cx) -> Layout {
        let levels = Settings::read(cx).levels as f32;
        let plan = Plan::new().pass(PlanPass::new("posterize", PlanImage::Source, PlanImage::Output).floats(&[levels]));
        Layout::pointwise().passes(plan)
    }

    fn pass(&self, _cx: &Cx, pass: &Pass, out: &mut Frame) -> Result<()> {
        let levels = pass.f[0];
        for (y, row) in out.rows_mut() {
            for (x, pixel) in row.iter_mut().enumerate() {
                let c = pass.src.at(x as isize, y as isize);
                *pixel = [(c[0] * levels + 0.5).floor() / levels, (c[1] * levels + 0.5).floor() / levels, (c[2] * levels + 0.5).floor() / levels, c[3]];
            }
        }
        Ok(())
    }
}
