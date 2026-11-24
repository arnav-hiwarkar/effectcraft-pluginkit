# Write a plugin

This page takes you from nothing to an effect running in EffectCraft, then shows how to package it, and
covers panels, commands, and native builds. For every type and call, see the [plugin reference](plugin-reference.md).
For how the system works, see [About the plugin system](plugin-system.md).

You need Rust and the WebAssembly target: `rustup target add wasm32-unknown-unknown`.

## Make an effect in five minutes

The [quickstart](plugin-quickstart.md) walks through it step by step. In short:

```sh
cargo xtask plugin new tint                    # writes plugins/tint from the effect template
cargo test --manifest-path plugins/tint/Cargo.toml
cargo xtask plugin render plugins/tint         # a PNG of the effect, without the app
cargo run -p effectcraft -- --control 9877     # in another terminal
cargo xtask plugin dev plugins/tint            # rebuilds and reloads into the running app on every save
```

`--kind effect|gpu-effect|panel|tool|task` picks the template; each one builds, passes its own tests and
does something you can see. In the app, apply **Tint** from Effects & Presets, Color Correction. `dev`
prints one line per reload, and the plugin's `log!` lines, panics and failed calls with their file and line:

```
reloaded com.example.tint: built in <seconds> s
[error] the plugin panicked at src/lib.rs:27:17: index out of bounds: the len is 4 but the index is 4
[error] render of effect `tint` failed: the plugin stopped: …
```

The [cookbook](plugin-cookbook.md) has short recipes for the common jobs.

### What is in the folder

```
tint/
  Cargo.toml
  plugin.json
  src/lib.rs
  tests/frames/full.png     (written by the first test run)
```

`plugin.json` names the plugin and what it asks for:

```json
{
  "id": "com.example.tint",
  "name": "Tint",
  "version": "0.1.0",
  "author": "",
  "description": "Tint: tints the layer.",
  "license": "MIT OR Apache-2.0",
  "entry": { "wasm": "plugin.wasm" },
  "permissions": []
}
```

`src/lib.rs` declares the parameters with `params!`, the effect with `Effect`, and the plugin with
`export!`. The effect appears in Effects & Presets under Color Correction, and Effect Controls shows its
rows with a stopwatch each.

## Typed parameters

`params!` declares each parameter once and gives two things: the list for the spec and a struct read in one
call. The field name is the parameter id; a quoted name opens a group, whose ids become `Group/field`.

```rust
params! {
    struct Settings {
        radius: f64 = slider("Radius", 20.0, 0.0..=200.0),
        gain: f64 = slider("Gain", 1.0, 0.0..=1000.0).soft(0.01..=10.0).log(),
        "Colour" {
            tint: [f32; 4] = color("Tint", [1.0, 0.5, 0.25, 1.0]),
            mode: usize = popup("Mode", &["Add", "Screen"], 1),
        }
        matte: LayerParam = layer("Matte"),
    }
}

fn spec(&self) -> EffectSpec {
    EffectSpec::new("glow", "Glow", "Stylize").params(Settings::params())
}

fn render(&self, cx: &Cx, frame: &mut Frame) -> Result<()> {
    let Settings { radius, gain, tint, mode, matte } = Settings::read(cx);
    let radius = radius * cx.scale;
    …
}
```

A group whose name is not a plain id gets one derived from it, and the twirl-down keeps the name:

```rust
EffectSpec::new("edges", "Edges", "Stylize").group("Fine Tuning", [Param::slider("detail", "Detail", 2.0, 0.0..=10.0)])
```

reads `cx.f("fine_tuning/detail")` and shows a `Fine Tuning` group. In `params!` the quoted name is the id,
so write it as one (`"Colour"`, `"fine_tuning"`).

The builder after `=` is any `Param` builder without its id. A misspelt field is a compile error, where
`cx.f("radus")` silently reads 0, and a field whose type does not fit its builder (`String` with `slider`)
is one too. The string API stays for ids built at run time.

When the saved id cannot be the field name, state it with `as`:

```rust
params! {
    struct Emitter {
        "Emitter" {
            kind as "type": usize = popup("Emitter Type", &["Point", "Box"], 0),
            emitter_gain as "gain": f64 = slider("Gain", 1.0, 0.0..=10.0),
        },
        "Glow" {
            glow_gain as "gain": f64 = slider("Gain", 1.0, 0.0..=10.0),
        },
    }
}
```

This saves `Emitter/type`, `Emitter/gain` and `Glow/gain`.

## Test your plugin

Add `craft-plugin-host` with the `testing` feature as a dev-dependency (the templates do). A test renders an
effect in the test process through the same host code the app uses (bands, padding, passes and host
kernels), on a built-in test image, and compares it with a stored PNG:

```rust
use craft_plugin_host::assert_frame;
use craft_plugin_host::testing::{Pattern, TestPlugin};

#[test]
fn half_amount_matches_the_stored_frame() {
    let plugin = TestPlugin::of(super::plugin);
    assert_frame!(plugin.render("tint").set("amount", 50.0), "half");
}

#[test]
fn zero_amount_changes_nothing() {
    let plugin = TestPlugin::of(super::plugin);
    let render = plugin.render("tint").set("amount", 0.0).on(Pattern::Checker).size(64, 32);
    assert_eq!(render.run().unwrap(), render.source());
}
```

The first run stores `tests/frames/half.png`. A later run that differs fails, writes `half.actual.png` and
`half.diff.png` under `target/frames/` and prints their paths; `UPDATE_FRAMES=1 cargo test` stores the new frames.
Each straight RGBA channel may differ by one level of 255; `.tolerance(0)` on the render asks for an exact
match. A
render in bands that differs from a whole render fails with the first pixel that changed, which catches a
`Layout::neighbourhood` reach that is too small. Panels, commands, tasks and tools are tested the same way:
see [Testing](plugin-reference.md#testing).

A test gets the values the app would deliver. `.set` goes through `Param::fit`, the same function the app
uses, so a slider value past its hard range is clamped and a value of the wrong kind is an error. With
`radius: f64 = slider("Radius", 20.0, 0.0..=200.0)`:

```rust
let plugin = TestPlugin::of(super::plugin);
let over = plugin.render("glow").set("radius", 500.0).run().unwrap();
assert_eq!(over, plugin.render("glow").set("radius", 200.0).run().unwrap());
let error = plugin.render("glow").set("radius", "wide").run().unwrap_err();
assert_eq!(error, r#"parameter `radius` takes a number, not the text "wide""#);
```

## Log, errors and panics

`log!(warn, "radius {radius} is larger than the layer")` writes a line with its file and line to the
plugin's log (`info` when the level is left out). A returned `Err`, a panic (with its file and line) and
each line of the log reach `cargo xtask plugin dev`, `effectcraft-cli exec plugin.log '{"id": "…"}'` and,
in tests, `plugin.logs()`:

```
[warn] src/lib.rs:31: radius 900 is larger than the layer
[error] the plugin panicked at src/lib.rs:44:21: attempt to divide by zero
[error] render of effect `glow` failed: the plugin stopped: …
```

## Check, render and benchmark

| Command | Does |
|---|---|
| `cargo xtask plugin check <dir>` | Builds the plugin, loads it with the checks the app runs (manifest, permissions, reserved and malformed ids), renders each effect once at its default settings on a 320x180 test image and reports, each with its fix: anything the app would refuse, a render that fails, clashing ids, a module built without the standard flags, a package over 32 MiB, a missing licence or description. Exits non-zero on errors. |
| `cargo xtask plugin render <dir> [--effect id] [--set id=value]… [--pattern photo\|gradient\|checker\|shapes\|text\|empty] [--in a.png] [--size WxH] [--time s] [--scale f] [--rounds n] [--native] [--out b.png]` | Builds and renders one effect to a PNG and prints the time. `--set` is read as the kind its parameter declares: a number for a slider, percent or angle; `true`/`false`/`on`/`off` for a checkbox; an option name or its number for a menu; `x,y` or `x,y,z` for a point; `r,g,b` or `r,g,b,a` for a colour; anything for text. A wrong value names the parameter and what it takes. An unknown flag or a flag without its value is an error that lists the valid flags. A folder without `Cargo.toml` is taken as a built bundle. |
| `cargo xtask plugin bench <dir> [--rounds n]` | Builds the sandboxed and the native module and prints, for every effect at 1080p and 4K, the best sandboxed and native time and their ratio, with the machine's load average before and after. |

## Package and install a plugin

A `.craftplugin` file is one file a user can install. It is a zip archive with `plugin.json` at the root,
the plugin's module and its `assets` folder.

```sh
cargo xtask plugin plugins/tint
```

This writes the unpacked folder `plugins/dist/com.example.tint/` and the package
`plugins/dist/com.example.tint-0.1.0.craftplugin`. The same source always gives the same bytes.

To install the package, use any of these:

- **Window ▸ Plug-ins ▸ Manage Plug-ins ▸ Install Plug-in...**, then choose the file. On macOS you can also
  double-click the file.
- `effectcraft-cli exec plugin.install '{"path": "plugins/dist/com.example.tint-0.1.0.craftplugin"}'`
- `plugin.install` with a folder path installs an unpacked bundle the same way.

The app unpacks the file into a temporary folder next to the plugin folder and moves it into place, so a
broken package leaves the installed version working. It refuses a package with an absolute path, a `..`
path, a link, more than 4096 entries or more than 512 MiB of content. The plugin runs after the user
approves what it asks for.

`plugin.load` runs an unpacked bundle for the current session with the permissions it asks for, and
`EFFECTCRAFT_PLUGIN_PATH` loads bundle folders every time the app starts. `cargo xtask plugin dev` uses
`plugin.load`.


## Make the effect fast

Tell the app how much of the frame each output pixel needs, in `layout()`.

- If each pixel depends only on itself, return `Layout::pointwise()`. The app renders bands of the frame
  on all cores.
- If each row depends on rows at most `n` away, return `Layout::neighbourhood(n)`. Each band writes its
  own rows and reads `n` more rows of input above and below.
- If the effect spills past the layer, add `.padded(n)`, or `.padded_sides(left, top, right, bottom)` when
  it spills one way only (a horizontal streak needs no rows above and below).
- If an output row reads input rows that `n` cannot describe, such as a scale or an offset, add
  `.maps_input()` and say which input an output rectangle needs in `Effect::input`.
- If the effect needs the whole frame, describe it as passes: `Layout::whole_frame().passes(plan)`. The
  app runs each pass on the GPU when there is one, and otherwise calls your `pass` method for bands of
  each pass on all cores. See [Passes](plugin-gpu.md).

A frame has rows to write and rows to read. `frame.pixels` holds the rectangle this call writes
(`frame.width` by `frame.height` at `frame.origin` in the layer's frame), and `frame.input` the input
rows it may read, in the layer's coordinates. Reading never sees what the call wrote, so a neighbourhood
effect needs no copy of its input:

```rust
fn render(&self, _cx: &Cx, frame: &mut Frame) -> Result<()> {
    let (input, x0) = (frame.input, frame.origin[0] as isize);
    for (y, row) in frame.rows_mut() {
        for (x, pixel) in (x0..).zip(row.iter_mut()) {
            let y = y as isize;
            *pixel = [input.at(x, y - 1), input.at(x, y), input.at(x, y + 1)].iter().fold([0.0; 4], |sum, p| std::array::from_fn(|c| sum[c] + p[c] / 3.0));
        }
    }
    Ok(())
}
```

Write every pixel of `frame.pixels`. `frame.map(f)` writes each pixel from the input pixel under it, and
`frame.rows()` yields `(y, input row, output row)` for a loop over both.

An effect that reads no other pixels says so with `Layout::pointwise()`. Its `frame.pixels` then starts
out holding the input, and when whole rows show the app changes the layer's frame in place, so the
cheapest loop is `frame.rows_in_place()`: `(y, row)` with each row holding the input, changed where it lies.
In place, `frame.rows()` first copies the band so that its input rows stay as they were; use it only when
the effect reads input it has already overwritten.

```rust
fn layout(&self, _cx: &Cx) -> Layout {
    Layout::pointwise()
}

fn render(&self, cx: &Cx, frame: &mut Frame) -> Result<()> {
    let limit = cx.f("limit") as f32;
    for (_, row) in frame.rows_in_place() {
        row.iter_mut().for_each(|pixel| pixel[3] = pixel[3].min(limit));
    }
    Ok(())
}
```

The app asks an effect only for the part of the layer that shows in the frame it renders: a layer half
outside the comp, or a viewer zoomed in, renders that part. A band is part of that rectangle. An effect
that shifts its picture says where its input comes from:

```rust
fn layout(&self, _cx: &Cx) -> Layout {
    Layout::pointwise().maps_input()
}

fn input(&self, _cx: &Cx, out: Rect) -> Rect {
    Rect { top: out.top + 8, bottom: out.bottom + 8, ..out }
}
```

`frame.input` then holds the rows of that rectangle and of `out` itself, clipped to the frame. Columns
are always whole rows. A plugin built before frames had separate input keeps working unchanged: the app
gives it whole rows in place, as before, and copies them for it.

Scale every distance by `cx.scale`, so a half-resolution preview matches the full render. `cx.px(id)`
does this for a slider measured in layer pixels.

Check a plan in a unit test with the same check the app runs:

```rust
let plan = Plan::new().pass(PlanPass::new("tint_apply", PlanImage::Source, PlanImage::Output).floats(&[1.0, 0.8, 0.6]));
assert_eq!(plan.validate([1920, 1080]), Ok(()));
```

## Host kernels

Blurs, resampling, blends, curves, convolutions, warps, colour matrices, sRGB transfers, brightness
curves, dither and light alpha need no code of your own.
Name them as passes of a plan and the app runs them natively on the CPU and with its own WGSL on the
GPU, with the same result on both. In a sandboxed plugin they run at native speed with no frame copied
into the sandbox. A glow:

```rust
fn layout(&self, cx: &Cx) -> Layout {
    let radius = (cx.px("radius") as f32).clamp(0.0, 2048.0);
    let padding = radius.ceil() as u32;
    let frame = cx.padded(padding).size;
    let mut plan = Plan::new();
    let quarter = frame.map(|side| side.div_ceil(4));
    let (small, blurred, glow) = (plan.scratch(quarter), plan.scratch(quarter), plan.scratch(frame));
    let plan = plan
        .pass(PlanPass::resample(PlanImage::Source, small, Filter::Bilinear))
        .pass(PlanPass::blur(small, blurred, [radius / 4.0; 2]))
        .pass(PlanPass::resample(blurred, glow, Filter::Bilinear))
        .pass(PlanPass::blend_images(PlanImage::Source, glow, PlanImage::Output, BlendMode::Screen, cx.f("amount") as f32));
    Layout::whole_frame().padded(padding).passes(plan)
}
```

The effect needs no WGSL and no `pass` method: the GPU runs a plan of host kernels too. A wrong field
fails `plan.validate` with the pass and the field it refuses. Mix host kernels with your own passes in
one plan when a step is your own. [Passes](plugin-gpu.md#host-kernels) lists every kernel and its rules.

Pointwise kernels may write the image they read, and consecutive ones run as one loop, so a grade costs
one pass over the frame however many steps it has. Keep only the highlights, tint them, and put them back
in sRGB with dither:

```rust
let mut plan = Plan::new();
let work = plan.scratch(cx.size);
let (curve, range) = (vec![0.0, 0.0, 0.0, 1.0, 2.0], 2.0);
let plan = plan
    .pass(PlanPass::to_linear(PlanImage::Source, work))
    .pass(PlanPass::brightness_curve(work, work, Measure::MaxChannel, curve, range))
    .pass(PlanPass::matrix(work, work, [[1.0, 0.0, 0.0, 0.0], [0.0, 0.8, 0.0, 0.0], [0.0, 0.0, 0.5, 0.0], [0.0, 0.0, 0.0, 1.0]]))
    .pass(PlanPass::to_srgb(work, work))
    .pass(PlanPass::dither(work, PlanImage::Output, cx.instance as u32, [0, 0]));
```

Light that can go negative (a glow of signed colour) keeps the sRGB curve's straight segment below zero
instead of mirroring it, and `BlendMode::ScreenHdr` screens a base outside 0 to 1 without darkening it:

```rust
let plan = plan
    .pass(PlanPass::transfer(PlanImage::Source, work, Encoding::Linear, Negatives::Linear))
    .pass(PlanPass::blend_images(work, glow, work, BlendMode::ScreenHdr, 1.0))
    .pass(PlanPass::transfer(work, PlanImage::Output, Encoding::Srgb, Negatives::Linear));
```

## Fast maths

In a sandboxed plugin, `f32::exp2`, `powf`, `sin`, `atan2` and the other maths functions are not single
instructions. They are calls into a soft library that the compiler does not inline, and each call pays the
timeout check. A per-pixel loop that makes several of them runs measurably slower than it needs to, and a native
build gives slightly different digits than the sandboxed one because each uses its own library.

`craft_plugin::math` has inline replacements. They use no tables and no library calls, so the sandboxed and the
native plugin compute the same bits. The accuracy is 2e-7 to 2e-5 depending on the function, listed in the
[reference](plugin-reference.md#fast-maths). That is far below one level of an 8-bit image.

```rust
use craft_plugin::math::{linear_to_srgb, saturate};

fn render(&self, cx: &Cx, frame: &mut Frame) -> Result<()> {
    let gain = cx.f("gain") as f32;
    frame.map(|mut pixel| {
        pixel[..3].iter_mut().for_each(|c| *c = linear_to_srgb(saturate(*c * gain)));
        pixel
    });
    Ok(())
}
```

An effect that calls several of them for every pixel can take the cheaper forms in `math::fast`, whose
errors (2e-6 to 1.2e-5) are still far below one level of a 16-bit image. Halo Flares uses them:

```rust
use craft_plugin::math::fast::{exp, ln};

fn glow(r: f32, radius: f32, falloff: f32) -> f32 {
    let q = (r / radius) * (r / radius);
    exp(-falloff * ln(1.0 + q))
}
```

Use them in the loop that runs for every pixel, and keep `std` for work done once per frame. `sqrt`, `abs`,
`floor`, `min` and `max` are single instructions already; keep calling those on `f32`.


## Replay a simulation

A simulation renders frame 40 by stepping from frame 0, and each step needs the parameters at its
own time. `cx.at(layer_time)` gives the whole `Cx` at another layer time, with keyframes and
expressions applied, and `cx.param_history()` gives a hash of the parameters' whole animation for
keying checkpoints:

```rust
fn render(&self, cx: &Cx, frame: &mut Frame) -> Result<()> {
    let key = cx.param_history().map(|history| format!("ck/{history:016x}"));
    for step in 0..=(cx.time * cx.frame_rate) as u32 {
        let then = cx.at(f64::from(step) / cx.frame_rate).ok_or("no parameters")?;
        let emitter = then.point("emitter");
    }
    Ok(())
}
```

Declare `.reads_param_history()` in the spec. When `param_history` is `None` (an expression is
involved), do not reuse checkpoints between frames. `plugins/ember` does this.

## Follow another layer and the camera

`cx.placement("target")` says where the layer chosen in a layer parameter sits, through both layers'
parents, and `cx.scene_at(comp_time)` gives the camera and lights at any time:

```rust
fn render(&self, cx: &Cx, frame: &mut Frame) -> Result<()> {
    let target = cx.placement("target")?;
    let [x, y, _] = target.apply([target.size[0] / 2.0, target.size[1] / 2.0, 0.0]);
    let centre = [x * cx.scale + cx.offset[0], y * cx.scale + cx.offset[1]];
    let blur = cx.scene().and_then(|scene| scene.camera?.focus).map_or(0.0, |focus| focus.aperture);
    let next = cx.scene_at(cx.comp_time + 1.0 / cx.frame_rate)?;
    Ok(())
}
```

Declare `.reads_scene().reads_other_frames()` in the spec; without them these calls return an error
that names the missing declaration. See [Placement](plugin-reference.md#placement).

## Offer presets

`.preset(name, values)` on the spec names a set of parameter values, for example a look popup that
also sets the sliders. The app applies one with `effect.applyPreset {layer, effect, preset}` in one
undo step and lists them with `effect.presets`. A preset writes absolute values to the parameters it names
and leaves the others alone. `plugins/halo` has a preset per look that sets the look and its element counts:

```rust
.preset("Sun", [("look", Value::Number(1.0)), ("Rays/count", Value::Number(14.0)), ("Ghosts/count", Value::Number(7.0))])
```

See [Presets](plugin-reference.md#presets).

## Give a slider its own ranges

A slider's hard range is what is allowed, and `cx.f` never returns a value outside it. The soft range is what the
slider shows, and `.log()` spaces it by ratio:

```rust
Param::slider("gain", "Gain", 1.0, 0.0..=1000.0).soft(0.01..=10.0).log()
Param::percent("mix", "Mix", 50.0).range(-100.0..=300.0)
```

## Add a panel

A panel keeps its own state. The app calls `ui` once to draw the panel and keeps what it returned: it
draws that tree every frame without calling the plugin. It calls `ui` again, off the UI thread, when the
user uses one of the panel's widgets, when an app event the panel subscribed to happens, when a timer of
the panel fires, or when the panel changes size. Until the new tree arrives the old one stays on screen,
so a slow panel never stalls the app. Each click reaches `ui` once: two clicks made while `ui` was busy
come as two calls, so `ui.button` returns `true` twice.

```rust
#[derive(Default)]
struct Counter {
    clicks: u32,
}

impl Panel for Counter {
    fn ui(&mut self, ui: &mut Ui) {
        if ui.button("Count") {
            self.clicks += 1;
        }
        ui.label(&format!("{} clicks", self.clicks));
    }
}

fn plugin() -> Plugin {
    Plugin::new().panel("counter", "Counter", Counter::default())
}
```

The panel appears in **Window ▸ Plug-ins**. Its tab docks like the app's own panels.

A widget is known by its label and how many times that label came before it, and a click arrives on the
next call. Draw each item of a list that can change inside `ui.keyed(item_id, |ui| …)`, so a click on
one item's "Delete" still reaches that item when another item is added above it.

### React to the app

`ui.on(topic)` subscribes the panel to an app event and returns it on the call it arrived in. Ask for it
in every call; a panel that stops asking stops getting it. Each kind arrives at most once per UI frame,
with its latest contents.

```rust
impl Panel for Follow {
    fn ui(&mut self, ui: &mut Ui) {
        if let Some(AppEvent::Selection { layers, keyframes, .. }) = ui.on(Topic::Selection) {
            self.status = format!("{} layers and {keyframes} keyframes selected", layers.len());
        }
        if let Some(AppEvent::Time { time, .. }) = ui.on(Topic::Time) {
            self.time = *time;
        }
        ui.label(&self.status);
    }
}
```

`plugins/glide` reads the curve of the selected keyframes this way.

### Timers

`ui.every(name, ms)` redraws the panel every `ms` milliseconds while the panel asks for it, and returns
`true` on the call the timer fired in. `ui.after(name, ms)` fires once, `ms` after the call that first
asked for it. Timers run at most every 16 ms and wait while the panel's tab is hidden.

```rust
if self.task.is_some() && ui.every("progress", 100) {
    self.poll_task();
}
```

### Keyboard

Every widget can be reached with Tab and used with the keyboard. A canvas takes the keyboard focus when it
is clicked or reached with Tab, keeps the arrow keys while it has it, and gets each key press with the
modifiers held:

```rust
ui.canvas("graph", [200.0, 200.0], |c| {
    for key in &c.keys {
        match key.name.as_str() {
            "Left" => self.nudge(if key.modifiers.shift { -0.1 } else { -0.01 }),
            "Delete" => self.remove_point(),
            _ => {}
        }
    }
    for pointer in &c.pointers {      // every pointer event of the call, in order
        self.drag(pointer);
    }
});
```

The canvas of a custom control in Effect Controls gets keys the same way. `plugins/curves` moves the chosen
point with the arrow keys.

### Clipboard

With the `clipboard` permission, a panel, command or task reads and replaces the text on the system
clipboard:

```rust
let text = host::clipboard()?;
host::set_clipboard(&format!("{text} (copied by my plugin)"))?;
```

### Drag and drop

`ui.drop_target` returns what the user dropped on it: files from the desktop or the Media Browser, project
items, or data another plugin dragged. `ui.drag_source` lets the user drag a file from the plugin's folders
(into the Project panel or a timeline, which import it) or plain data to another plugin's drop target.

```rust
match ui.drop_target("bin", |ui| ui.weak("Drop footage here")) {
    Some(Dropped::Files { paths }) => self.queue.extend(paths.iter().cloned()),
    Some(Dropped::Items { ids }) => self.items.extend(ids),
    Some(Dropped::Data { plugin, data }) => self.note = format!("{plugin} sent {data}"),
    None => {}
}
ui.drag_source("export", Drag::File { root: Root::Data, path: "out/look.cube".into() }, |ui| ui.label("look.cube"));
```

A dropped file counts as one the user picked: the plugin reads it with `host::read_file(Root::User, path)` and needs
no `files.read`. Only that file opens, not the folder it is in, and it is not writable. A dropped project item
gives the item id only.

```rust
if let Some(Dropped::Files { paths }) = ui.drop_target("lut", |ui| ui.weak("Drop a .cube file"))
    && let Some(path) = paths.first()
{
    self.lut = host::read_file(Root::User, path).ok();
}
```

### Show images

Upload RGBA pixels once under a key, then show them by key. Uploading again under the same key replaces
the image.

```rust
host::upload_image("thumb/0", 56, 56, &rgba8)?;      // straight RGBA, 4 bytes per pixel
ui.scroll("thumbs", 240.0, |ui| {
    ui.grid(3, |ui| {
        if ui.image("thumb/0", [56.0, 56.0]) {          // true on the frame it was clicked
            apply_preset(0)?;
        }
        ui.tooltip("Soft Contrast");                    // for the widget just before it
    });
});
```

A canvas draws an uploaded image with `c.image(key, min, max)`. The app drops a plugin's images when the
plugin stops.

## Draw a control in Effect Controls

A custom parameter holds text the plugin owns, usually JSON. The effect draws its control with the same
widgets as a panel, in the parameter's row, at the width of Effect Controls.

```rust
impl Effect for Curves {
    fn spec(&self) -> EffectSpec {
        EffectSpec::new("curves", "Curves", "Color Correction").param(Param::custom("curve", "Curve", "[]"))
    }

    fn controls(&self, instance: u64, param: &str, value: &mut String, ui: &mut Ui) {
        if ui.button("Reset") {
            *value = "[]".into();
        }
    }

    fn render(&self, cx: &Cx, frame: &mut Frame) -> Result<()> {
        let curve = cx.text("curve");
        …
    }
}
```

When `controls` changes `value`, the app sets the parameter at the time the user made the change, even if
the playhead moved before the reply arrived. One drag is one undo step. `controls` runs
in the long-lived instance, like a panel, so state kept in the effect (a selected channel, the point being
dragged) lasts between frames. Keep that state per `instance`: it is the same for one effect on one layer
and differs between two copies of the effect. `render` runs in pooled instances and sees only `cx`. A project with a
custom value opens without the plugin: the value is kept and the layer renders without the effect.

`plugins/curves` is a complete example.

## On-viewer controls

A selected effect can draw on the Composition viewer and give the user handles to drag there. A point
handle for an existing parameter is one line:

```rust
impl Effect for Halo {
    fn overlay(&self, cx: &Cx, overlay: &mut Overlay) {
        overlay.point(cx, "light");
    }
}
```

A ring that sets a size, a guide line and a label take one line each:

```rust
fn overlay(&self, cx: &Cx, overlay: &mut Overlay) {
    let centre = cx.point("light");
    overlay.point(cx, "light").radius("size", centre, cx.f("size") * 2.0, 2.0);
    overlay.line(&[centre, cx.point("target")]).text(centre, "Light");
}
```

Coordinates are layer pixels, the same values `cx.point` returns here (in `overlay`, `cx.scale` is 1 and
`cx.offset` is 0). The app maps them through the layer's transform, the 3D view, the zoom and the pixel
aspect, and does the hit testing. A drag is one undo step and sets a key at the current time when the
parameter is keyframed, as a drag in Effect Controls does. Tab moves between handles and the arrow keys
move the focused one by 1 pixel (10 with Shift).

The app calls `overlay` again only when the effect's values, the time or the selection change, and
redraws the last overlay on every other frame. A point parameter with no handle of its own keeps the
app's built-in crosshair.

An overlay is capped at 4096 shapes, 256 handles, 4096 points per line and 512 characters per label;
the first ones drawn are kept, so draw the important marks first.

`plugins/halo` (light and glow ring) and `plugins/plasma-beam` (start and end) are complete examples.

## Viewer tools

A tool has a button in the toolbar. While it is picked, the plugin gets the pointer and key events on
the Composition viewer and draws its own overlay.

```rust
#[derive(Default)]
struct Pin;

impl Tool for Pin {
    fn event(&mut self, event: &ToolEvent, overlay: &mut Overlay) -> Result<()> {
        if event.kind == ToolEventKind::Down {
            host::command("layer.newNull", json!({ "name": "Pin" }))?;
        }
        overlay.circle(event.comp, 6.0);
        Ok(())
    }
}

Plugin::new().tool("pin", "Pin", Pin::default())
```

`event.comp` is the pointer in comp pixels at any zoom; `event.layer` is the same point in the first
selected layer's pixels. A tool's overlay is in comp pixels unless it calls `overlay.space(Space::Layer)`.
Everything one press-to-release gesture changes in the project is one undo step, named after the tool;
an edit made elsewhere during the gesture stays its own step. Changing the project needs the
`document.write` permission. Events reach `event` in order; moves made while `event` was busy arrive as
the latest one.

## Add a command

```rust
Plugin::new().command("add-null", "Add Control Null", || {
    host::command("layer.newNull", serde_json::json!({ "name": "Control" }))?;
    Ok(())
})
```

The command appears in **Window ▸ Plug-ins**.

## Read and edit the project

Panels and commands use the app's own commands, the same ones the menus, the CLI, and automation use.
Add the permissions to `plugin.json`: `document.read` to read, `document.write` to edit. A command
that names a file, such as `file.saveCopy`, also needs both `files.read` and `files.write`.

```rust
let info = host::command("comp.info", serde_json::json!({}))?;
```

To make several edits one undo step, send them as an `engine.batch`:

```rust
host::command("engine.batch", serde_json::json!({
    "label": "Add Rig Nulls",
    "steps": [
        { "command": "layer.newNull", "params": { "name": "Root" } },
        { "command": "layer.newNull", "params": { "name": "Aim" } }
    ]
}))?;
```

To find a command, run `effectcraft-cli commands --filter keys.`.

## Run a long task

Panel and command calls must finish in 2 s and 60 s. Work that takes longer, such as analysing audio,
tracking or exporting, is a task:

```rust
Plugin::new().task("analyse", "Analyse Audio", |task: &Task| {
    let total = 100;
    for done in 0..total {
        task.progress(done as f64 / total as f64, "Analysing")?;
    }
    Ok(serde_json::json!({ "peaks": 3 }))
})
```

Start it from a panel or a command, then ask for its state on a timer while it runs:

```rust
let id = host::start("analyse", serde_json::json!({ "layer": "Music" }))?;
if ui.every("progress", 100) {
    let state = host::task(id)?;  // status, progress, message, result, error
}
host::cancel(id)?;
```

`task.progress` returns an error after the user cancels, so `?` ends the task. A task runs on its own
instance, so it shares no memory with the panel: pass what it needs in `args` and read what it found in
`state.result`. Its edits go through the app's main thread like a command's; send them as one
`engine.batch` to make them one undo step. Running tasks show in **Window ▸ Plug-ins ▸ Manage
Plug-ins** with a progress bar and a Cancel button. `plugins/pulse` is a complete example.

## Read frames and audio

With `document.read`, a panel, command or task can render the comp, or one layer with its effects, and
read the audio mix or one layer's audio:

```rust
let image = host::frame(FrameRequest { comp: None, layer: Some("Logo".into()), time: 1.0, scale: 0.5 })?;
let audio = host::audio(AudioRequest { layer: None, start: 0.0, duration: 2.0, rate: 48_000 })?;
```

An effect cannot call either while it renders.

## Ask the user for a file

```rust
let picked = host::pick_file(PickFile { save: false, title: "Choose a LUT".into(), extensions: vec!["cube".into()] })?;
if let Some(path) = picked {
    let lut = host::read_file(Root::User, &path)?;   // no files.read needed: the user chose it
}
```

Panels and commands only. With `save: true` the path is also writable.

## Store files

A missing file is not a failed read: `read_file` returns a `FileError` with a `kind`.

```rust
let saved = match host::read_file(Root::Data, "presets.json") {
    Ok(bytes) => bytes,
    Err(error) if error.kind == FileErrorKind::NotFound => Vec::new(),
    Err(error) => return Err(error.into()),
};
```

```rust
host::write_file(Root::Data, "presets.json", bytes)?;   // private to the plugin, always allowed
let lut = host::read_file(Root::Bundle, "assets/film.cube")?;  // shipped with the plugin, read-only
let any = host::read_file(Root::User, "/abs/path")?;    // needs files.read
```

## Add a native build

Add a native build only for work the sandbox cannot do. The source does not change.

```sh
cargo xtask plugin plugins/my-plugin --native
```

The package then holds both builds, and the manifest's `entry` names each:

```json
"entry": { "wasm": "plugin.wasm", "native": { "macos-aarch64": "libx.dylib" } }
```

The plugin runs sandboxed. For a plugin with both builds, the manager shows **Run Unsandboxed**. It
loads the native build, which renders on the app's memory with one copy per band and may use threads, and it shows the
warning that the plugin has full access to the computer. The choice holds for that version only; a new
version runs sandboxed again. **Run Sandboxed** switches back. From the command line:

```sh
effectcraft-cli exec plugin.setSandbox '{"id": "com.example.tint", "sandboxed": false}'
```

A package built on one platform carries that platform's library. On another platform it runs
sandboxed only.

A plugin whose manifest has no `wasm` entry is native only. The user sees the warning before enabling
it, and `plugin.load` and `EFFECTCRAFT_PLUGIN_PATH` refuse it. Install it and approve it instead:

```sh
effectcraft-cli exec plugin.install '{"path": "plugins/dist/com.example.tint-0.1.0.craftplugin"}'
effectcraft-cli exec plugin.approve '{"id": "com.example.tint"}'
```

## Check your work

- `effectcraft-cli exec plugin.list` shows each plugin's state and, when a plugin fails to load, the error.
- `effectcraft-cli exec plugin.log '{"id": "com.example.tint"}'` lists the plugin's log lines, panics and
  failed calls; pass the last `seq` seen as `after` to get only newer ones.
- `cargo xtask plugin render plugins/tint` renders an effect on a test pattern without the app and prints the
  render time.
- A render error shows as a warning mark on the effect in Effect Controls. The frame passes through
  unchanged.
