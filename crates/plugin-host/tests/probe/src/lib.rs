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

const GPU_KERNELS: &str = r#"
@compute @workgroup_size(16, 16)
fn probe_invert(@builtin(global_invocation_id) id: vec3<u32>) {
    if (!inside_out(id.xy)) { return; }
    let c = texel(src, vec2<i32>(id.xy));
    store(id.xy, vec4<f32>(c.rgb + (vec3<f32>(c.a) - 2.0 * c.rgb) * pass_f(0u), c.a));
}

@compute @workgroup_size(16, 16)
fn probe_down(@builtin(global_invocation_id) id: vec3<u32>) {
    if (!inside_out(id.xy)) { return; }
    let p = vec2<i32>(id.xy) * 2;
    let sum = texel(src, p) + texel(src, p + vec2<i32>(1, 0)) + texel(src, p + vec2<i32>(0, 1)) + texel(src, p + vec2<i32>(1, 1));
    store(id.xy, sum * 0.25);
}

@compute @workgroup_size(16, 16)
fn probe_up(@builtin(global_invocation_id) id: vec3<u32>) {
    if (!inside_out(id.xy)) { return; }
    let c = texel(src, vec2<i32>(id.xy));
    let soft = sample_uv(aux, out_uv(id.xy));
    store(id.xy, c + (soft - c) * pass_f(0u));
}
"#;

struct GpuInvert;

impl Effect for GpuInvert {
    fn spec(&self) -> EffectSpec {
        EffectSpec::new("gpu-invert", "GPU Invert", "Probe").param(Param::slider("amount", "Amount", 1.0, 0.0..=1.0)).shader(GPU_KERNELS)
    }

    fn layout(&self, cx: &Cx) -> Layout {
        let pass = GpuPass::new("probe_invert", GpuImage::Source, GpuImage::Output).floats(&[cx.f("amount") as f32]);
        Layout::pointwise().gpu(GpuPlan::new().pass(pass))
    }

    fn render(&self, cx: &Cx, frame: &mut Frame) -> Result<()> {
        let amount = cx.f("amount") as f32;
        for pixel in frame.pixels.iter_mut() {
            let alpha = pixel[3];
            for channel in &mut pixel[..3] {
                *channel += (alpha - 2.0 * *channel) * amount;
            }
        }
        Ok(())
    }
}

const SOFTEN_PADDING: u32 = 4;

struct GpuSoften;

impl GpuSoften {
    fn half(frame: [usize; 2]) -> [usize; 2] {
        [frame[0].div_ceil(2), frame[1].div_ceil(2)]
    }
}

impl Effect for GpuSoften {
    fn spec(&self) -> EffectSpec {
        EffectSpec::new("gpu-soften", "GPU Soften", "Probe").param(Param::slider("strength", "Strength", 0.75, 0.0..=1.0)).shader(GPU_KERNELS)
    }

    fn layout(&self, cx: &Cx) -> Layout {
        let pad = 2 * SOFTEN_PADDING;
        let [w, h] = GpuSoften::half([(cx.size[0] + pad) as usize, (cx.size[1] + pad) as usize]);
        let mut plan = GpuPlan::new();
        let half = plan.scratch([w as u32, h as u32]);
        let plan = plan
            .pass(GpuPass::new("probe_down", GpuImage::Source, half))
            .pass(GpuPass::new("probe_up", GpuImage::Source, GpuImage::Output).aux(half).floats(&[cx.f("strength") as f32]));
        Layout::whole_frame().padded(SOFTEN_PADDING).gpu(plan)
    }

    fn render(&self, cx: &Cx, frame: &mut Frame) -> Result<()> {
        let (width, height) = (frame.width, frame.height);
        let at = |x: i64, y: i64| -> Pixel {
            if x < 0 || y < 0 || x >= width as i64 || y >= height as i64 { [0.0; 4] } else { frame.pixels[y as usize * width + x as usize] }
        };
        let [hw, hh] = GpuSoften::half([width, height]);
        let mut half = vec![[0.0f32; 4]; hw * hh];
        for y in 0..hh {
            for x in 0..hw {
                let (sx, sy) = (2 * x as i64, 2 * y as i64);
                let block = [at(sx, sy), at(sx + 1, sy), at(sx, sy + 1), at(sx + 1, sy + 1)];
                half[y * hw + x] = std::array::from_fn(|c| (block[0][c] + block[1][c] + block[2][c] + block[3][c]) * 0.25);
            }
        }
        let clamped = |x: i64, y: i64| half[y.clamp(0, hh as i64 - 1) as usize * hw + x.clamp(0, hw as i64 - 1) as usize];
        let strength = cx.f("strength") as f32;
        for y in 0..height {
            for x in 0..width {
                let fx = (x as f32 + 0.5) / width as f32 * hw as f32 - 0.5;
                let fy = (y as f32 + 0.5) / height as f32 * hh as f32 - 0.5;
                let (x0, y0) = (fx.floor(), fy.floor());
                let (tx, ty) = (fx - x0, fy - y0);
                let (xi, yi) = (x0 as i64, y0 as i64);
                let (a, b, c, d) = (clamped(xi, yi), clamped(xi + 1, yi), clamped(xi, yi + 1), clamped(xi + 1, yi + 1));
                let pixel = &mut frame.pixels[y * width + x];
                for k in 0..4 {
                    let top = a[k] + (b[k] - a[k]) * tx;
                    let bottom = c[k] + (d[k] - c[k]) * tx;
                    let soft = top + (bottom - top) * ty;
                    pixel[k] += (soft - pixel[k]) * strength;
                }
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

struct HalfDone;

impl Effect for HalfDone {
    fn spec(&self) -> EffectSpec {
        EffectSpec::new("half-done", "Half Done", "Probe")
    }

    fn render(&self, _cx: &Cx, frame: &mut Frame) -> Result<()> {
        frame.pixels.fill([9.0; 4]);
        Err("gave up half way".into())
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
        .effect(HalfDone)
        .effect(GpuInvert)
        .effect(GpuSoften)
        .panel("counter", "Counter", Counter::default())
        .command("edit", "Edit Document", || {
            let reply = host::command("layer.newNull", json!({"name": "From Plugin"}))?;
            host::write_file(Root::Data, "last-reply.json", reply.to_string().as_bytes())
        })
        .command("read", "Read Document", || host::command("comp.info", json!({})).map(|_| ()))
        .command("batch-edit", "Edit In A Batch", || {
            let steps = json!([{"command": "layer.newNull", "params": {"name": "One"}}, {"command": "layer.newNull", "params": {"name": "Two"}}]);
            host::command("engine.batch", json!({"label": "Two Nulls", "steps": steps})).map(|_| ())
        })
        .command("save-copy", "Save A Copy", || {
            let path = String::from_utf8(host::read_file(Root::Bundle, "user-target.txt")?).map_err(|e| e.to_string())?;
            host::command("file.saveCopy", json!({"path": path.trim()})).map(|_| ())
        })
        .command("escape-data", "Escape Data", || host::write_file(Root::Data, "../outside.txt", b"x"))
        .command("write-bundle", "Write Bundle", || host::write_file(Root::Bundle, "plugin.json", b"{}"))
        .command("write-user", "Write User", || {
            let path = String::from_utf8(host::read_file(Root::Bundle, "user-target.txt")?).map_err(|e| e.to_string())?;
            host::write_file(Root::User, path.trim(), b"written by plugin")
        })
        .command("approve-itself", "Approve Itself", || host::command("plugin.approve", json!({"id": "test.gate-self"})).map(|_| ()))
        .command("read-inside-edit", "Read Inside An Edit", || {
            let steps = json!([{"command": "comp.info", "params": {}}, {"command": "layer.newNull", "params": {}}]);
            host::command("engine.batch", json!({"steps": steps})).map(|_| ())
        })
        .command("open-website", "Open Website", || host::command("help.website", json!({})).map(|_| ()))
        .command("write-link", "Write Through A Link", || host::write_file(Root::Data, "link.txt", b"x"))
        .command("greet", "Greet", || {
            host::notify("3 layers renamed");
            Ok(())
        })
        .command("relay", "Relay A Command", || {
            let request: serde_json::Value = serde_json::from_slice(&host::read_file(Root::Bundle, "relay.json")?)?;
            let command = request["command"].as_str().ok_or("relay.json names no command")?;
            host::command(command, request["params"].clone()).map(|_| ())
        })
        .command("run-script", "Run Script", || host::command("script.run", json!({"code": "1"})).map(|_| ()))
}

craft_plugin::export!(make);
