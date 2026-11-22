mod flare;
#[cfg(test)]
mod golden;
mod looks;
mod settings;
mod spec;

use craft_plugin::prelude::*;

use crate::flare::{Pack, composite, flare, place};

struct Halo;

fn gpu_pass(pack: &Pack) -> GpuPlan {
    GpuPlan::new().pass(GpuPass::new("halo_render", GpuImage::Source, GpuImage::Output).floats(&pack.f).uints(&pack.u))
}

impl Effect for Halo {
    fn spec(&self) -> EffectSpec {
        spec::spec()
    }

    fn layout(&self, cx: &Cx) -> Layout {
        let pack = settings::pack(cx, settings::occlusion(cx));
        Layout::pointwise().gpu(gpu_pass(&pack))
    }

    fn overlay(&self, cx: &Cx, overlay: &mut Overlay) {
        let unit = f64::from(looks::look(cx.index("look")).glow.size) * cx.f("scale") / 10_000.0;
        overlay.point(cx, "light").radius("Glow/size", cx.point("light"), unit * cx.f("Glow/size"), unit.max(1e-3)).named("Glow Size");
    }

    fn render(&self, cx: &Cx, frame: &mut Frame) -> Result<()> {
        let pack = settings::pack(cx, settings::occlusion(cx));
        let ghosts = place(&pack);
        let x0 = frame.origin[0];
        for (y, read, row) in frame.rows() {
            let py = y as f32 + 0.5;
            for (x, (pixel, source)) in row.iter_mut().zip(read).enumerate() {
                let light = flare(&pack, &ghosts, (x0 + x) as f32 + 0.5, py);
                *pixel = composite(&pack, *source, light);
            }
        }
        Ok(())
    }
}

fn plugin() -> Plugin {
    Plugin::new().effect(Halo)
}

craft_plugin::export!(plugin);
