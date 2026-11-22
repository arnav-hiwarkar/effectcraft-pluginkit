# Plugin cookbook

Short recipes for the things plugins do most. Each one is a file of the Cookbook plugin in
`plugins/cookbook`, which is built and tested by `plugins/new-plugin-check.sh`. Every code block on this
page names its source above the code and in its fence: a file such as `plugins/cookbook/src/tint.rs`, or
one function in a file such as `plugins/cookbook/src/tests.rs#tint_matches_its_stored_frame`. A test in `xtask` reads that
name and checks the block is exactly that file or function, so the recipes cannot go stale. Start with the
[quickstart](plugin-quickstart.md) if you have not made a plugin yet; every type and call is in the
[reference](plugin-reference.md).

The plugin's `src/lib.rs` declares each recipe's module, registers what the recipes contribute and exports
the plugin once with `craft_plugin::export!`. The two effects under `#[cfg(test)]` exist only for its
tests: one reads too far for its declared reach and one panics.

`plugins/cookbook/src/lib.rs`:

```rust plugins/cookbook/src/lib.rs
mod blur;
mod edges;
mod follow;
mod glow;
mod handle;
mod kernel;
mod matte;
mod measure;
mod memo;
mod nulls;
mod pin;
mod tint;

use craft_plugin::prelude::*;

fn plugin() -> Plugin {
    let plugin = Plugin::new()
        .effect(tint::Tint)
        .effect(blur::Blur)
        .effect(glow::Glow)
        .effect(kernel::Posterize)
        .effect(matte::Matte)
        .effect(edges::Edges)
        .effect(memo::Gamma)
        .effect(handle::Spotlight)
        .command("add-rig", "Add Rig", nulls::add_rig)
        .panel("rig", "Rig", nulls::Rig::default())
        .panel("follow", "Follow", follow::Follow::default())
        .panel("meter", "Meter", measure::Meter::default())
        .task("measure", "Measure Brightness", measure::measure)
        .tool("pin", "Pin", pin::Pin::default());
    #[cfg(test)]
    let plugin = plugin.effect(tests::Smear).effect(tests::Crash);
    plugin
}

craft_plugin::export!(plugin);

#[cfg(test)]
mod tests;
```

## 1. Tint: a pointwise effect with typed parameters

`params!` declares each parameter once. It gives the list for the spec (`Settings::params()`) and a struct
read in one call (`Settings::read(cx)`), so a misspelt id is a compile error instead of a silent default.
The field name is the parameter id. `Layout::pointwise()` tells the app each pixel needs only itself, so
it renders bands of the frame on every core.

`plugins/cookbook/src/tint.rs`:

```rust plugins/cookbook/src/tint.rs
use craft_plugin::prelude::*;

params! {
    struct Settings {
        amount: f64 = percent("Amount", 100.0),
        tint: [f32; 4] = color("Tint", [1.0, 0.55, 0.25, 1.0]),
    }
}

pub struct Tint;

impl Effect for Tint {
    fn spec(&self) -> EffectSpec {
        EffectSpec::new("tint", "Tint", "Cookbook")
            .params(Settings::params())
            .preset("Cold", [("tint", Value::List(vec![0.4, 0.7, 1.0, 1.0])), ("amount", Value::Number(80.0))])
    }

    fn layout(&self, _cx: &Cx) -> Layout {
        Layout::pointwise()
    }

    fn render(&self, cx: &Cx, frame: &mut Frame) -> Result<()> {
        let settings = Settings::read(cx);
        let mix = (settings.amount / 100.0) as f32;
        let gain = settings.tint.map(|tint| 1.0 + (tint - 1.0) * mix);
        frame.map(|[r, g, b, a]| [r * gain[0], g * gain[1], b * gain[2], a]);
        Ok(())
    }
}
```

## 2. A preset

`.preset(name, values)` on the spec, as in the Tint recipe above, names a set of parameter values. The
app lists presets with `effect.presets` and applies one with `effect.applyPreset` as one undo step.

## 3. Blur with a host kernel

Blurs, resampling, blends, curves, convolutions, warps and colour matrices are host kernels: name them as
passes of a plan and the app runs them natively on the CPU and with its own shader on the GPU, with no
pixel loop of your own. Scale distances by `cx.scale` so a half-resolution preview matches the full render.

`plugins/cookbook/src/blur.rs`:

```rust plugins/cookbook/src/blur.rs
use craft_plugin::prelude::*;

params! {
    struct Settings {
        radius: f64 = slider("Radius", 8.0, 0.0..=200.0),
    }
}

pub struct Blur;

impl Effect for Blur {
    fn spec(&self) -> EffectSpec {
        EffectSpec::new("blur", "Blur", "Cookbook").params(Settings::params()).shader("")
    }

    fn layout(&self, cx: &Cx) -> Layout {
        let radius = (Settings::read(cx).radius * cx.scale) as f32;
        let plan = Plan::new().pass(PlanPass::blur(PlanImage::Source, PlanImage::Output, [radius; 2]));
        Layout::whole_frame().passes(plan)
    }
}
```

## 4. A glow from host kernels

Shrink, blur the small image, grow it back and screen it over the source. The effect spills past the
layer, so `layout` pads the frame by the radius and `.padded(padding)` asks the app for that room.

`plugins/cookbook/src/glow.rs`:

```rust plugins/cookbook/src/glow.rs
use craft_plugin::prelude::*;

params! {
    struct Settings {
        radius: f64 = slider("Radius", 24.0, 0.0..=400.0),
        amount: f64 = slider("Amount", 1.0, 0.0..=4.0),
    }
}

pub struct Glow;

impl Effect for Glow {
    fn spec(&self) -> EffectSpec {
        EffectSpec::new("glow", "Glow", "Cookbook").params(Settings::params()).shader("")
    }

    fn layout(&self, cx: &Cx) -> Layout {
        let settings = Settings::read(cx);
        let radius = ((settings.radius * cx.scale) as f32).clamp(0.0, 2048.0);
        let padding = radius.ceil() as u32;
        let frame = cx.padded(padding).size;
        let quarter = frame.map(|side| side.div_ceil(4));
        let mut plan = Plan::new();
        let (small, blurred, glow) = (plan.scratch(quarter), plan.scratch(quarter), plan.scratch(frame));
        let plan = plan
            .pass(PlanPass::resample(PlanImage::Source, small, Filter::Bilinear))
            .pass(PlanPass::blur(small, blurred, [radius / 4.0; 2]))
            .pass(PlanPass::resample(blurred, glow, Filter::Bilinear))
            .pass(PlanPass::blend_images(PlanImage::Source, glow, PlanImage::Output, BlendMode::Screen, settings.amount as f32));
        Layout::whole_frame().padded(padding).passes(plan)
    }
}
```

## 5. Your own GPU kernel, with the same pass on the CPU

Kernels are WGSL functions in the string given to `.shader`. `Layout::passes(plan)` runs the plan on the
GPU when there is one and otherwise calls `pass` for bands of each pass on all cores, so both paths give
the same picture. `pass.f` holds the floats the plan passed.

`plugins/cookbook/src/kernel.rs`:

```rust plugins/cookbook/src/kernel.rs
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
```

## 6. Read another layer

A `LayerParam` field (the `layer(name)` builder) lets the user pick a layer. `.image(cx)` returns its
pixels on the same grid as the frame, or `None` when no layer is picked; read them with `at(x, y)` in
whole-frame coordinates.

`plugins/cookbook/src/matte.rs`:

```rust plugins/cookbook/src/matte.rs
use craft_plugin::prelude::*;

params! {
    struct Settings {
        matte: LayerParam = layer("Matte Layer"),
    }
}

pub struct Matte;

impl Effect for Matte {
    fn spec(&self) -> EffectSpec {
        EffectSpec::new("matte", "Luma Matte", "Cookbook").params(Settings::params())
    }

    fn render(&self, cx: &Cx, frame: &mut Frame) -> Result<()> {
        let Some(matte) = Settings::read(cx).matte.image(cx) else {
            frame.map(|pixel| pixel);
            return Ok(());
        };
        let left = frame.origin[0];
        for (y, read, out) in frame.rows() {
            for (x, (out, pixel)) in out.iter_mut().zip(read).enumerate() {
                let [r, g, b, _] = matte.at((left + x) as isize, y as isize);
                let luma = 0.2126 * r + 0.7152 * g + 0.0722 * b;
                *out = pixel.map(|c| c * luma);
            }
        }
        Ok(())
    }
}
```

## 7. Read neighbouring pixels

Say how far each row reads with `Layout::neighbourhood(rows)`. The app gives each band that many extra
rows above and below; `frame.input.at(x, y)` reads them by whole-frame coordinates and returns transparent
black outside. A test render (recipe 13) compares a banded render with a whole one, so a reach that is too
small fails the test with the pixel where it shows.

`plugins/cookbook/src/edges.rs`:

```rust plugins/cookbook/src/edges.rs
use craft_plugin::prelude::*;

params! {
    struct Settings {
        width: f64 = slider("Width", 2.0, 1.0..=20.0),
    }
}

pub struct Edges;

impl Effect for Edges {
    fn spec(&self) -> EffectSpec {
        EffectSpec::new("edges", "Find Edges", "Cookbook").params(Settings::params())
    }

    fn layout(&self, cx: &Cx) -> Layout {
        Layout::neighbourhood(step(cx) as u32)
    }

    fn render(&self, cx: &Cx, frame: &mut Frame) -> Result<()> {
        let step = step(cx) as isize;
        let (left, input) = (frame.origin[0], frame.input);
        for (y, row) in frame.rows_mut() {
            for (x, pixel) in row.iter_mut().enumerate() {
                let (x, y) = ((left + x) as isize, y as isize);
                let dx = input.at(x + step, y)[1] - input.at(x - step, y)[1];
                let dy = input.at(x, y + step)[1] - input.at(x, y - step)[1];
                let edge = (dx * dx + dy * dy).sqrt().min(1.0);
                *pixel = [edge, edge, edge, 1.0];
            }
        }
        Ok(())
    }
}

fn step(cx: &Cx) -> f64 {
    (Settings::read(cx).width * cx.scale).round().max(1.0)
}
```

## 8. Remember work between frames

`cx.remember(name, bytes)` stores bytes for this effect on this layer and `cx.recall(name)` reads them
back in a later frame, so work that depends only on the parameters is done once. The app may drop stored
bytes when memory is short; always be able to compute them again.

`plugins/cookbook/src/memo.rs`:

```rust plugins/cookbook/src/memo.rs
use craft_plugin::prelude::*;

params! {
    struct Settings {
        gamma: f64 = slider("Gamma", 2.2, 0.2..=5.0),
    }
}

pub struct Gamma;

fn table(gamma: f64) -> Vec<u8> {
    (0..1024).flat_map(|i| ((i as f64 / 1023.0).powf(1.0 / gamma) as f32).to_le_bytes()).collect()
}

impl Effect for Gamma {
    fn spec(&self) -> EffectSpec {
        EffectSpec::new("gamma", "Gamma", "Cookbook").params(Settings::params())
    }

    fn layout(&self, _cx: &Cx) -> Layout {
        Layout::pointwise()
    }

    fn render(&self, cx: &Cx, frame: &mut Frame) -> Result<()> {
        let gamma = Settings::read(cx).gamma;
        let key = format!("table/{gamma}");
        let bytes = match cx.recall(&key) {
            Some(bytes) => bytes,
            None => {
                let bytes = table(gamma);
                cx.remember(&key, &bytes)?;
                bytes
            }
        };
        let table: Vec<f32> = bytes.chunks_exact(4).map(|b| f32::from_le_bytes([b[0], b[1], b[2], b[3]])).collect();
        let curve = |c: f32| table[(c.clamp(0.0, 1.0) * 1023.0) as usize];
        frame.map(|[r, g, b, a]| [curve(r), curve(g), curve(b), a]);
        Ok(())
    }
}
```

## 9. A handle on the viewer

`overlay` draws on the Composition viewer while the effect is selected. `overlay.point(cx, id)` gives a
point parameter a handle; `.radius(id, centre, pixels, unit)` adds a handle that sets a size by its
distance from the centre. The app maps layer pixels through the layer's transform and the zoom.

`plugins/cookbook/src/handle.rs`:

```rust plugins/cookbook/src/handle.rs
use craft_plugin::prelude::*;

params! {
    struct Settings {
        centre: [f64; 2] = point("Centre", [0.5, 0.5]),
        radius: f64 = slider("Radius", 60.0, 1.0..=2000.0),
    }
}

pub struct Spotlight;

impl Effect for Spotlight {
    fn spec(&self) -> EffectSpec {
        EffectSpec::new("spotlight", "Spotlight", "Cookbook").params(Settings::params())
    }

    fn layout(&self, _cx: &Cx) -> Layout {
        Layout::pointwise()
    }

    fn render(&self, cx: &Cx, frame: &mut Frame) -> Result<()> {
        let Settings { centre, radius } = Settings::read(cx);
        let (radius, left) = (radius * cx.scale, frame.origin[0]);
        for (y, read, out) in frame.rows() {
            for (x, (out, pixel)) in out.iter_mut().zip(read).enumerate() {
                let distance = ((left + x) as f64 - centre[0]).hypot(y as f64 - centre[1]);
                let light = (1.0 - distance / radius).clamp(0.0, 1.0) as f32;
                *out = pixel.map(|c| c * light);
            }
        }
        Ok(())
    }

    fn overlay(&self, cx: &Cx, overlay: &mut Overlay) {
        let Settings { centre, radius } = Settings::read(cx);
        overlay.point(cx, "centre").radius("radius", centre, radius, 1.0);
    }
}
```

## 10. A panel button that edits the project

A panel draws its widgets in `ui` and the app calls it again when a widget is used. Project edits go
through the app's own commands; `engine.batch` makes several of them one undo step. The same function can
be a menu command. Add `document.write` to `permissions` in `plugin.json`. `log!` writes a line with the
file and line it came from to the plugin's log, which `cargo xtask plugin dev` prints. Show an error the
user can act on with `ui.error`.

`plugins/cookbook/src/nulls.rs`:

```rust plugins/cookbook/src/nulls.rs
use craft_plugin::prelude::*;

pub fn add_rig() -> Result<()> {
    let steps = json!([
        { "command": "layer.newNull", "params": { "name": "Rig Root" } },
        { "command": "layer.newNull", "params": { "name": "Rig Aim" } },
    ]);
    host::command("engine.batch", json!({ "label": "Add Rig", "steps": steps }))?;
    log!(info, "added a rig of two nulls");
    Ok(())
}

#[derive(Default)]
pub struct Rig {
    error: Option<String>,
}

impl Panel for Rig {
    fn ui(&mut self, ui: &mut Ui) {
        if ui.button("Add Rig") {
            self.error = add_rig().err().map(|error| error.to_string());
        }
        if let Some(error) = &self.error {
            ui.error(error);
        }
    }
}
```

## 11. React to the selection and the time

`ui.on(topic)` subscribes the panel to an app event and returns the event on the call it arrived in. Ask
for it in every call.

`plugins/cookbook/src/follow.rs`:

```rust plugins/cookbook/src/follow.rs
use craft_plugin::prelude::*;

#[derive(Default)]
pub struct Follow {
    layers: Vec<u64>,
    time: f64,
}

impl Panel for Follow {
    fn ui(&mut self, ui: &mut Ui) {
        if let Some(AppEvent::Selection { layers, .. }) = ui.on(Topic::Selection) {
            self.layers = layers.clone();
        }
        if let Some(AppEvent::Time { time, .. }) = ui.on(Topic::Time) {
            self.time = *time;
        }
        ui.label(&format!("{} layers selected at {:.2} s", self.layers.len(), self.time));
    }
}
```

## 12. A background task with progress

Work longer than a panel call is a task. It runs on its own instance, reports progress (an error after the
user cancels, so `?` ends it) and returns JSON. The panel starts it, polls it on a timer and shows an
error when the start or a poll fails. Reading frames needs `document.read`.

`plugins/cookbook/src/measure.rs`:

```rust plugins/cookbook/src/measure.rs
use craft_plugin::prelude::*;

pub fn measure(task: &Task) -> Result<serde_json::Value> {
    let frames = task.args()["frames"].as_u64().unwrap_or(10).max(1);
    let mut total = 0.0;
    for n in 0..frames {
        task.progress(n as f64 / frames as f64, &format!("Frame {} of {frames}", n + 1))?;
        let image = host::frame(FrameRequest { comp: None, layer: None, time: n as f64 / 10.0, scale: 0.25 })?;
        let sum: f32 = image.pixels().iter().map(|p| (p[0] + p[1] + p[2]) / 3.0).sum();
        total += f64::from(sum) / image.pixels().len() as f64;
    }
    Ok(json!({ "brightness": total / frames as f64 }))
}

#[derive(Default)]
pub struct Meter {
    running: Option<u64>,
    state: Option<TaskState>,
    error: Option<String>,
}

impl Panel for Meter {
    fn ui(&mut self, ui: &mut Ui) {
        if let Some(id) = self.running
            && ui.every("progress", 100)
        {
            match host::task(id) {
                Ok(state) => {
                    if state.status != TaskStatus::Running {
                        self.running = None;
                    }
                    self.state = Some(state);
                }
                Err(error) => (self.running, self.error) = (None, Some(error.to_string())),
            }
        }
        if let Some(state) = &self.state {
            ui.progress(state.progress as f32, &state.message);
        }
        if self.running.is_none() && ui.button("Measure") {
            match host::start("measure", json!({ "frames": 10 })) {
                Ok(id) => (self.running, self.error) = (Some(id), None),
                Err(error) => self.error = Some(error.to_string()),
            }
            ui.every("progress", 100);
        }
        if let Some(error) = &self.error {
            ui.error(error);
        }
    }
}
```

## 13. Test an effect against a stored frame

Add `craft-plugin-host` with the `testing` feature as a dev-dependency (the templates do). `assert_frame!`
renders the effect in the test process through the same host code the app uses, on a built-in test image,
and compares it with `tests/frames/<name>.png`. The first run stores the frame; look at it and commit it.
When a frame differs the test fails, writes `<name>.actual.png` and `<name>.diff.png` under the plugin's
`target/frames/` (red marks the pixels that differ) and prints their paths. Run with `UPDATE_FRAMES=1` to
accept the new frames.

`plugins/cookbook/src/tests.rs#tint_matches_its_stored_frame`:

```rust plugins/cookbook/src/tests.rs#tint_matches_its_stored_frame
#[test]
fn tint_matches_its_stored_frame() {
    let plugin = TestPlugin::of(crate::plugin);
    assert_frame!(plugin.render("tint").set("amount", 100.0), "tint");
}
```

`render(effect)` is a builder: `.set(id, value)`, `.on(Pattern::Checker)`, `.input(picture)`,
`.size(w, h)`, `.layer(id, picture)`, `.mask(points, closed)`, `.time(s)`, `.scale(f)`, `.tolerance(levels)`.
`run()` returns the picture, `layout()` the layout, `overlay()` the overlay.

`plugins/cookbook/src/tests.rs#a_test_renders_on_a_chosen_image_and_compares_with_a_stored_frame`:

```rust plugins/cookbook/src/tests.rs#a_test_renders_on_a_chosen_image_and_compares_with_a_stored_frame
#[test]
fn a_test_renders_on_a_chosen_image_and_compares_with_a_stored_frame() {
    let plugin = TestPlugin::of(crate::plugin);
    let render = plugin.render("blur").on(Pattern::Checker).size(160, 90).set("radius", 6.0);
    assert_frame!(render, "blur-checker");
}
```

A misspelt parameter or effect id fails with the ids the effect has.

## 14. Test a panel, a task and a tool

`panel(id)` draws a panel; `click(label)` presses the widget with that label and draws again; `texts()`
lists what it shows. Commands the plugin runs are recorded, not run: `commands()` lists them, and
`answer(command, json)` sets what a command returns.

`plugins/cookbook/src/tests.rs#the_rig_button_adds_two_nulls_in_one_undo_step_and_logs_where`:

```rust plugins/cookbook/src/tests.rs#the_rig_button_adds_two_nulls_in_one_undo_step_and_logs_where
#[test]
fn the_rig_button_adds_two_nulls_in_one_undo_step_and_logs_where() {
    let plugin = TestPlugin::of(crate::plugin);
    plugin.panel("rig").unwrap().click("Add Rig").unwrap();
    let (command, params) = &plugin.commands()[0];
    assert_eq!((command.as_str(), &params["label"]), ("engine.batch", &json!("Add Rig")));
    assert_eq!(plugin.logs(), ["src/nulls.rs:9: added a rig of two nulls"]);
    plugin.command("add-rig").unwrap();
    assert_eq!(plugin.commands().len(), 2);
}
```

`plugins/cookbook/src/tests.rs#a_panic_reaches_the_test_and_the_log_with_its_file_and_line`:

```rust plugins/cookbook/src/tests.rs#a_panic_reaches_the_test_and_the_log_with_its_file_and_line
#[test]
fn a_panic_reaches_the_test_and_the_log_with_its_file_and_line() {
    let plugin = TestPlugin::of(crate::plugin);
    let error = plugin.render("crash").size(4, 2).run().unwrap_err();
    assert_eq!(error, "the plugin panicked at src/tests.rs:37:9: index out of bounds: the len is 8 but the index is 8");
    assert!(plugin.logs().contains(&format!("render of effect `crash` failed: {error}")), "{:?}", plugin.logs());
}
```

## 15. A viewer tool

A tool has a toolbar button. While it is picked the plugin gets the pointer on the Composition viewer and
draws its own overlay; `event.comp` is the pointer in comp pixels at any zoom, which is also the position
of a layer in the comp. The tool records a pin only after the app made the null, so a failed click leaves
nothing behind.

`plugins/cookbook/src/pin.rs`:

```rust plugins/cookbook/src/pin.rs
use craft_plugin::prelude::*;

#[derive(Default)]
pub struct Pin {
    pins: Vec<[f64; 2]>,
}

fn add_null(name: &str, at: [f64; 2]) -> Result<()> {
    let added = host::command("layer.newNull", json!({ "name": name }))?;
    let layer = added["layer"].as_u64().ok_or("layer.newNull did not return the new layer")?;
    host::command("prop.set", json!({ "layer": layer, "path": "transform/position", "value": [at[0], at[1], 0.0] }))?;
    Ok(())
}

impl Tool for Pin {
    fn event(&mut self, event: &ToolEvent, overlay: &mut Overlay) -> Result<()> {
        if event.kind == ToolEventKind::Down {
            add_null(&format!("Pin {}", self.pins.len() + 1), event.comp)?;
            self.pins.push(event.comp);
        }
        for pin in &self.pins {
            overlay.circle(*pin, 4.0);
        }
        overlay.circle(event.comp, 8.0);
        Ok(())
    }
}
```

`plugins/cookbook/src/tests.rs#the_pin_tool_adds_a_null_where_it_is_clicked_and_rings_the_pointer`:

```rust plugins/cookbook/src/tests.rs#the_pin_tool_adds_a_null_where_it_is_clicked_and_rings_the_pointer
#[test]
fn the_pin_tool_adds_a_null_where_it_is_clicked_and_rings_the_pointer() {
    let plugin = TestPlugin::of(crate::plugin);
    plugin.answer("layer.newNull", json!({ "layer": 3 }));
    let overlay = plugin.tool("pin", pointer(ToolEventKind::Down, [10.0, 20.0])).unwrap();
    let position = json!({ "layer": 3, "path": "transform/position", "value": [10.0, 20.0, 0.0] });
    assert_eq!(plugin.commands(), [("layer.newNull".to_string(), json!({ "name": "Pin 1" })), ("prop.set".to_string(), position)]);
    let Shape::Circle { center, radius, .. } = &overlay.marks[1].shape else { panic!("{:?}", overlay.marks) };
    assert_eq!((*center, *radius), ([10.0, 20.0], 8.0));
}
```

## 16. Measure the speed

`cargo xtask plugin bench` builds the sandboxed and the native module and renders every effect at 1080p
and 4K with its default parameters, best of five, with the machine's load before and after:

```sh
cargo xtask plugin bench plugins/cookbook
```

For one effect with chosen values, `render` prints the time of the best of `--rounds` renders and writes the
PNG:

```sh
cargo xtask plugin render plugins/cookbook --effect glow --set radius=40 --rounds 5 --out /tmp/glow.png
```
