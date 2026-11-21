use craft_plugin::prelude::*;
use serde_json::json;

struct Invert;

impl Effect for Invert {
    fn spec(&self) -> EffectSpec {
        EffectSpec::new("invert", "Invert", "Probe").param(Param::slider("amount", "Amount", 1.0, 0.0..=1.0))
    }

    fn layout(&self, _cx: &Cx) -> Layout {
        Layout::pointwise()
    }

    fn render(&self, cx: &Cx, frame: &mut Frame) -> Result<()> {
        let amount = cx.f("amount") as f32;
        for pixel in frame.pixels.iter_mut() {
            for channel in &mut pixel[..3] {
                *channel += (1.0 - 2.0 * *channel) * amount;
            }
        }
        Ok(())
    }
}

struct ShiftDown;

impl Effect for ShiftDown {
    fn spec(&self) -> EffectSpec {
        EffectSpec::new("shift-down", "Shift Down", "Probe")
    }

    fn layout(&self, _cx: &Cx) -> Layout {
        Layout::neighbourhood(1)
    }

    fn render(&self, _cx: &Cx, frame: &mut Frame) -> Result<()> {
        let width = frame.width;
        let first_row_is_frame_top = frame.origin[1] == 0;
        for y in (1..frame.height).rev() {
            let (above, here) = frame.pixels.split_at_mut(y * width);
            here[..width].copy_from_slice(&above[(y - 1) * width..]);
        }
        if first_row_is_frame_top {
            frame.pixels[..width].fill([0.0; 4]);
        }
        Ok(())
    }
}

struct ReadInputs;

impl Effect for ReadInputs {
    fn spec(&self) -> EffectSpec {
        EffectSpec::new("read-inputs", "Read Inputs", "Probe").param(Param::layer("matte", "Matte"))
    }

    fn render(&self, cx: &Cx, frame: &mut Frame) -> Result<()> {
        let masks = cx.masks();
        let matte = cx.layer("matte").ok_or("no matte layer")?;
        let earlier = cx.source_at(cx.time - 1.0).ok_or("no earlier frame")?;
        frame.pixels[0] = [masks.len() as f32, masks[0].points[1][0] as f32, matte.at(1, 0)[0], earlier.at(0, 0)[3]];
        Ok(())
    }
}

struct Spin;

impl Effect for Spin {
    fn spec(&self) -> EffectSpec {
        EffectSpec::new("spin", "Spin", "Probe")
    }

    fn render(&self, _cx: &Cx, frame: &mut Frame) -> Result<()> {
        let mut n = 0u64;
        loop {
            n = n.wrapping_add(1);
            frame.pixels[0][0] = n as f32;
        }
    }
}

#[derive(Default)]
struct Counter {
    clicks: u32,
    speed: f64,
}

impl Panel for Counter {
    fn ui(&mut self, ui: &mut Ui) {
        if ui.button("Count") {
            self.clicks += 1;
        }
        ui.slider("Speed", &mut self.speed, 0.0..=10.0);
        ui.label(&format!("{} clicks at speed {}", self.clicks, self.speed));
    }
}

fn make() -> Plugin {
    Plugin::new()
        .effect(Invert)
        .effect(ShiftDown)
        .effect(ReadInputs)
        .effect(Spin)
        .panel("counter", "Counter", Counter::default())
        .command("edit", "Edit Document", || {
            let reply = host::command("layer.rename", json!({"name": "from plugin"}))?;
            host::write_file(Root::Data, "last-reply.json", reply.to_string().as_bytes())
        })
        .command("read", "Read Document", || host::command("comp.info", json!({})).map(|_| ()))
        .command("escape-data", "Escape Data", || host::write_file(Root::Data, "../outside.txt", b"x"))
        .command("write-bundle", "Write Bundle", || host::write_file(Root::Bundle, "plugin.json", b"{}"))
        .command("write-user", "Write User", || {
            let path = String::from_utf8(host::read_file(Root::Bundle, "user-target.txt")?).map_err(|e| e.to_string())?;
            host::write_file(Root::User, path.trim(), b"written by plugin")
        })
        .command("run-script", "Run Script", || host::command("script.run", json!({"code": "1"})).map(|_| ()))
}

craft_plugin::export!(make);
