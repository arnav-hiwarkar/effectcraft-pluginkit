use craft_plugin::prelude::*;

struct Mark;

impl Effect for Mark {
    fn spec(&self) -> EffectSpec {
        EffectSpec::new("mark", "Mark", "Legacy").param(Param::point("spot", "Spot", [0.5, 0.5]))
    }

    fn layout(&self, _cx: &Cx) -> Layout {
        Layout::pointwise()
    }

    fn render(&self, cx: &Cx, frame: &mut Frame) -> Result<()> {
        let [x, y] = cx.point("spot");
        let [width, height] = cx.size.map(f64::from);
        frame.pixels.fill([(x / width) as f32, (y / height) as f32, 0.0, 1.0]);
        Ok(())
    }
}

fn plugin() -> Plugin {
    Plugin::new().effect(Mark)
}

craft_plugin::export!(plugin);
