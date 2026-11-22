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
        let overall = spec::Overall::read(cx);
        let unit = f64::from(looks::look(overall.look).glow.size) * overall.scale / 10_000.0;
        overlay.point(cx, "light").radius("Glow/size", overall.light, unit * spec::Glow::read(cx).size, unit.max(1e-3)).named("Glow Size");
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

#[cfg(test)]
mod tests {
    use craft_plugin_host::assert_frame;
    use craft_plugin_host::testing::TestPlugin;

    #[test]
    fn the_sun_look_matches_its_stored_frame() {
        assert_frame!(TestPlugin::of(super::plugin), "flare", { "look": 1 }, "sun");
    }

    #[test]
    fn the_anamorphic_look_with_its_light_moved_matches_its_stored_frame() {
        let plugin = TestPlugin::of(super::plugin);
        assert_frame!(plugin.render("flare").set("look", 2.0).set("light", [200.0, 60.0]).set("Streak/size", 150.0), "anamorphic");
    }
}
