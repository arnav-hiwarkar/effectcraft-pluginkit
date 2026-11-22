use craft_plugin::math;
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

struct FastMaths;

impl Effect for FastMaths {
    fn spec(&self) -> EffectSpec {
        EffectSpec::new("fast-maths", "Fast Maths", "Probe")
    }

    fn layout(&self, _cx: &Cx) -> Layout {
        Layout::pointwise()
    }

    fn render(&self, _cx: &Cx, frame: &mut Frame) -> Result<()> {
        for pixel in frame.pixels.iter_mut() {
            let x = pixel[0];
            *pixel = [math::exp(x), math::sin(x), math::atan2(x, 2.0 - x), math::pow(x.abs(), 2.4)];
        }
        Ok(())
    }
}

struct Scale;

impl Effect for Scale {
    fn spec(&self) -> EffectSpec {
        EffectSpec::new("scale", "Scale", "Probe").param(Param::custom("level", "Level", "1"))
    }

    fn layout(&self, _cx: &Cx) -> Layout {
        Layout::pointwise()
    }

    fn render(&self, cx: &Cx, frame: &mut Frame) -> Result<()> {
        let level: f32 = cx.text("level").parse().map_err(|_| Error::from("the level is not a number"))?;
        for pixel in frame.pixels.iter_mut() {
            pixel[..3].iter_mut().for_each(|channel| *channel *= level);
        }
        Ok(())
    }

    fn controls(&self, instance: u64, _param: &str, value: &mut String, ui: &mut Ui) {
        ui.label(&format!("Level {value}"));
        ui.label(&format!("Instance {instance}"));
        if ui.button("Halve") {
            let level: f64 = value.parse().unwrap_or(1.0);
            *value = (level / 2.0).to_string();
        }
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

const FILL: [f32; 3] = [0.25, 0.5, 0.75];

const FILL_SHADER: &str = r#"
@vertex
fn vs_probe_fill(@location(0) corner: vec2<f32>) -> @builtin(position) vec4<f32> {
    return vec4<f32>(corner, 0.5, 1.0);
}

@fragment
fn fs_probe_fill() -> @location(0) vec4<f32> {
    return vec4<f32>(pass_f(0u), pass_f(1u), pass_f(2u), 1.0);
}
"#;

struct GpuFill;

impl Effect for GpuFill {
    fn spec(&self) -> EffectSpec {
        EffectSpec::new("gpu-fill", "GPU Fill", "Probe").param(Param::slider("last_index", "Last Index", 2.0, 0.0..=9.0)).shader(FILL_SHADER)
    }

    fn layout(&self, cx: &Cx) -> Layout {
        let mut plan = GpuPlan::new();
        let corners = plan.buffer(GpuData::Floats(vec![-1.0, -1.0, 3.0, -1.0, -1.0, 3.0]));
        let triangle = plan.buffer(GpuData::Indices(vec![0, 1, cx.f("last_index") as u32]));
        let fill = GpuPass::draw("probe_fill", GpuImage::Output).vertices(corners, &[2]).indices(triangle).clear().floats(&FILL);
        Layout::pointwise().gpu(plan.pass(fill))
    }

    fn render(&self, _cx: &Cx, frame: &mut Frame) -> Result<()> {
        frame.pixels.fill([FILL[0], FILL[1], FILL[2], 1.0]);
        Ok(())
    }
}

const MATTE_SHADER: &str = r#"
@compute @workgroup_size(16, 16)
fn probe_matte(@builtin(global_invocation_id) id: vec3<u32>) {
    if (!inside_out(id.xy)) { return; }
    store(id.xy, texel(aux, vec2<i32>(id.xy)));
}
"#;

struct GpuMatte;

impl Effect for GpuMatte {
    fn spec(&self) -> EffectSpec {
        EffectSpec::new("gpu-matte", "GPU Matte", "Probe").param(Param::layer("matte", "Matte")).shader(MATTE_SHADER)
    }

    fn layout(&self, _cx: &Cx) -> Layout {
        let mut plan = GpuPlan::new();
        let matte = plan.layer("matte");
        Layout::pointwise().gpu(plan.pass(GpuPass::new("probe_matte", GpuImage::Source, GpuImage::Output).aux(matte)))
    }

    fn render(&self, cx: &Cx, frame: &mut Frame) -> Result<()> {
        match cx.layer("matte") {
            Some(matte) => frame.pixels.copy_from_slice(matte.pixels()),
            None => frame.pixels.fill([0.0; 4]),
        }
        Ok(())
    }
}

type Taps = fn(&dyn Fn(isize, isize) -> Pixel, isize, isize) -> Vec<(Pixel, f32)>;

struct Passes;

impl Effect for Passes {
    fn spec(&self) -> EffectSpec {
        EffectSpec::new("passes", "Passes", "Probe")
    }

    fn pass(&self, _cx: &Cx, pass: &Pass, out: &mut Frame) -> Result<()> {
        let src = pass.src;
        let sum = |pixels: &[(Pixel, f32)]| -> Pixel { std::array::from_fn(|c| pixels.iter().map(|(pixel, weight)| pixel[c] * weight).sum()) };
        let kernel: Taps = match pass.kernel {
            "probe_red" => return Ok(out.pixels.fill([1.0, 0.0, 0.0, 1.0])),
            "probe_nothing" => return Ok(()),
            "probe_down" => |at, x, y| [(0, 0), (1, 0), (0, 1), (1, 1)].map(|(dx, dy)| (at(2 * x + dx, 2 * y + dy), 0.25)).to_vec(),
            "probe_up" => |at, x, y| [0, 1].map(|dy| (at(x / 2, y / 2 + dy), 0.5)).to_vec(),
            "probe_tall" => |at, x, y| (-3..=3).map(|dy| (at(x, y + dy), (dy + 4) as f32)).collect(),
            other => return Err(format!("no probe kernel `{other}`").into()),
        };
        for (y, row) in out.rows_mut() {
            for (x, pixel) in row.iter_mut().enumerate() {
                *pixel = sum(&kernel(&|x, y| src.at(x, y), x as isize, y as isize));
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

struct Recurse;

impl Effect for Recurse {
    fn spec(&self) -> EffectSpec {
        EffectSpec::new("recurse", "Recurse", "Probe")
    }

    fn render(&self, _cx: &Cx, frame: &mut Frame) -> Result<()> {
        let request = craft_plugin::FrameRequest { time: 0.0, scale: 1.0, ..Default::default() };
        let refused = host::frame(request).err().is_some_and(|e| e.0.contains("cannot render a frame while it renders"));
        frame.pixels[0][0] = if refused { 1.0 } else { 2.0 };
        Ok(())
    }
}

struct Memo;

impl Effect for Memo {
    fn spec(&self) -> EffectSpec {
        EffectSpec::new("memo", "Memo", "Probe")
    }

    fn render(&self, _cx: &Cx, frame: &mut Frame) -> Result<()> {
        host::cache_put("memo", &[1, 2, 3])?;
        frame.pixels[0][0] = 1.0;
        Ok(())
    }
}

struct Report;

impl Effect for Report {
    fn spec(&self) -> EffectSpec {
        EffectSpec::new("report", "Report", "Probe")
            .param(Param::slider("level", "Level", 10.0, 0.0..=100.0))
            .param(Param::point("spot", "Spot", [0.5, 0.5]))
            .reads_param_history()
            .preset("Loud", [("level", craft_plugin::Value::Number(90.0)), ("spot", craft_plugin::Value::List(vec![0.25, 0.75]))])
    }

    fn render(&self, cx: &Cx, frame: &mut Frame) -> Result<()> {
        let at_zero = cx.at(0.0).ok_or("no parameters at time 0")?;
        let history = cx.param_history();
        let report = json!({"instance": cx.instance.to_string(), "now": cx.f("level"), "at_zero": at_zero.f("level"), "at_zero_time": at_zero.time, "history": history.map(|h| h.to_string())});
        host::write_file(Root::Data, &format!("report-{}.json", cx.instance), report.to_string().as_bytes())?;
        frame.pixels[0] = [cx.f("level") as f32, at_zero.f("level") as f32, at_zero.time as f32, f32::from(u8::from(history.is_some()))];
        Ok(())
    }
}

struct SceneCamera;

impl Effect for SceneCamera {
    fn spec(&self) -> EffectSpec {
        EffectSpec::new("scene-camera", "Scene Camera", "Probe").reads_scene()
    }

    fn render(&self, cx: &Cx, frame: &mut Frame) -> Result<()> {
        let camera = cx.scene().and_then(|scene| scene.camera).ok_or("no scene camera")?;
        let [x, y, z] = camera.position.map(|c| c as f32);
        frame.pixels[0] = [x, y, z, 1.0];
        frame.pixels[1] = [camera.zoom as f32, 0.0, 0.0, 1.0];
        Ok(())
    }
}

struct HalfDone;

impl Effect for HalfDone {
    fn spec(&self) -> EffectSpec {
        EffectSpec::new("half-done", "Half Done", "Probe")
    }

    fn render(&self, cx: &Cx, frame: &mut Frame) -> Result<()> {
        frame.pixels.fill([9.0; 4]);
        if frame.origin[1] + frame.height < cx.size[1] as usize {
            return Ok(());
        }
        Err("gave up half way".into())
    }
}

struct MarkBand;

impl Effect for MarkBand {
    fn spec(&self) -> EffectSpec {
        EffectSpec::new("mark-band", "Mark Band", "Probe")
    }

    fn render(&self, _cx: &Cx, frame: &mut Frame) -> Result<()> {
        let (top, rows) = (frame.origin[1] as f32, frame.height as f32);
        for (y, row) in frame.rows_mut() {
            row.fill([top, rows, y as f32, 1.0]);
        }
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
        .effect(FastMaths)
        .effect(Scale)
        .effect(ShiftDown)
        .effect(ReadInputs)
        .effect(SceneCamera)
        .effect(Spin)
        .effect(HalfDone)
        .effect(MarkBand)
        .effect(Recurse)
        .effect(Memo)
        .effect(Report)
        .effect(GpuInvert)
        .effect(GpuSoften)
        .effect(Passes)
        .effect(GpuFill)
        .effect(GpuMatte)
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
        .command("read-user", "Read User", || {
            let path = String::from_utf8(host::read_file(Root::Bundle, "user-target.txt")?).map_err(|e| e.to_string())?;
            let bytes = host::read_file(Root::User, path.trim())?;
            host::write_file(Root::Data, "read.txt", &bytes)
        })
        .command("put-table", "Put A Lookup Table", || host::cache_put("table", &[4, 5, 6]))
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
        .command("start-count", "Start Counting", || {
            let id = host::start("count", json!({"steps": 3}))?;
            host::write_file(Root::Data, "task-id.txt", id.to_string().as_bytes())
        })
        .task("count", "Count", |task: &Task| {
            let steps = task.args()["steps"].as_u64().ok_or("count needs `steps`")?;
            for step in 0..steps {
                task.progress(step as f64 / steps as f64, &format!("step {step}"))?;
            }
            Ok(json!({"counted": steps}))
        })
        .task("forever", "Forever", |_task: &Task| {
            let mut n = 0u64;
            loop {
                n = std::hint::black_box(n.wrapping_add(1));
            }
        })
        .task("patient", "Patient", |task: &Task| {
            loop {
                task.progress(0.5, "waiting")?;
                std::hint::black_box(0);
            }
        })
        .command("pick-and-read", "Pick And Read", || {
            let request = craft_plugin::PickFile { title: "Choose".into(), ..Default::default() };
            let path = host::pick_file(request)?.ok_or("nothing was picked")?;
            let bytes = host::read_file(Root::User, &path)?;
            host::write_file(Root::Data, "picked.txt", &bytes)?;
            host::write_file(Root::User, &path, b"overwritten")
        })
        .command("pick-and-save", "Pick And Save", || {
            let request = craft_plugin::PickFile { save: true, title: "Save".into(), ..Default::default() };
            let path = host::pick_file(request)?.ok_or("nothing was picked")?;
            host::write_file(Root::User, &path, b"saved")
        })
        .task("pick", "Pick From A Task", |_task: &Task| host::pick_file(craft_plugin::PickFile::default()).map(|path| json!(path)))
        .task("frame", "Read A Frame", |task: &Task| {
            let args = task.args();
            let layer = args["layer"].as_str().map(String::from);
            let request =
                craft_plugin::FrameRequest { comp: None, layer, time: args["time"].as_f64().unwrap_or(0.0), scale: args["scale"].as_f64().unwrap_or(1.0) };
            let image = host::frame(request)?;
            let center = image.at(image.width as isize / 2, image.height as isize / 2);
            Ok(json!({"width": image.width, "height": image.height, "center": center}))
        })
        .task("audio", "Read Audio", |task: &Task| {
            let args = task.args();
            let layer = args["layer"].as_str().map(String::from);
            let request = craft_plugin::AudioRequest { layer, start: 0.0, duration: args["duration"].as_f64().unwrap_or(0.5), rate: 8000 };
            let audio = host::audio(request)?;
            let peak = audio.samples.iter().fold(0.0f32, |peak, sample| peak.max(sample.abs()));
            Ok(json!({"channels": audio.channels, "rate": audio.rate, "len": audio.samples.len(), "peak": peak}))
        })
        .task("edit", "Edit From A Task", |_task: &Task| host::command("layer.newNull", json!({"name": "From Task"})))
}

craft_plugin::export!(make);
