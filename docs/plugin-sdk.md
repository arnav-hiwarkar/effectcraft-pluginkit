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

A group name is the first part of its parameters' ids and the name its twirl-down shows:

```rust
EffectSpec::new("edges", "Edges", "Stylize").group("Fine Tuning", [Param::slider("detail", "Detail", 2.0, 0.0..=10.0)])
```

reads `cx.f("Fine Tuning/detail")`. A part may not start with `#` or `@` or contain `/`.

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

An installed plugin costs almost nothing at start: the app keeps the list of what it adds (from
`Plugin::new()...`) and loads the plugin only when you first use one of its effects, panels, commands or
tasks. Rebuild and reinstall and the list is read again:

```rust
craft_plugin::export!(|| Plugin::new().effect(Tint).command("reset", "Reset Tint", reset))
```

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
  each pass on all cores. See [Passes](plugin-gpu.md). Bound every loop in your WGSL by a pass value or a
  constant: a loop that runs past 1 048 576 iterations in one invocation turns the shader off, and the
  effect's error and your log say so.

  ```wgsl
  for (var i = 0u; i < min(pass_u(0u), 256u); i++) { sum += texel(src, vec2<i32>(id.xy) + vec2<i32>(i32(i), 0)); }
  ```

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

An effect that draws something new without reading its input, such as a shape or a sprite, says so with
`.ignores_input()` and reports what it drew with `frame.drawn(rect)`. The app copies nothing in, and
everything outside the rectangle is transparent, so the effect writes only inside it and never clears the
rest of the frame:

```rust
fn layout(&self, _cx: &Cx) -> Layout {
    Layout::pointwise().ignores_input()
}

fn render(&self, cx: &Cx, frame: &mut Frame) -> Result<()> {
    let [x, y] = cx.point("centre").map(|at| at as i32);
    let dot = Rect { left: x - 50, top: y - 50, right: x + 50, bottom: y + 50 };
    let x0 = frame.origin[0] as i32;
    for (row_y, row) in frame.rows_mut() {
        if (dot.top..dot.bottom).contains(&(row_y as i32)) {
            for at in dot.left.max(x0)..dot.right.min(x0 + frame.width as i32) {
                row[(at - x0) as usize] = [1.0, 0.5, 0.0, 1.0];
            }
        }
    }
    frame.drawn(dot);
    Ok(())
}
```

`frame.pixels` holds leftovers, not the input, so write every pixel inside the rectangle you report. The
rectangle may reach past the frame; the app clips it.

Scale every distance by `cx.scale`, so a half-resolution preview matches the full render. `cx.px(id)`
does this for a slider measured in layer pixels.

Check a plan in a unit test with the same check the app runs:

```rust
let plan = Plan::new().pass(PlanPass::new("tint_apply", PlanImage::Source, PlanImage::Output).floats(&[1.0, 0.8, 0.6]));
assert_eq!(plan.validate([1920, 1080]), Ok(()));
```

## Host kernels

Blurs, resampling, blends, curves, convolutions, warps, colour matrices, sRGB transfers, brightness
curves, dither, light alpha, dilate and erode, directional blur, bilateral filtering and noise need no code
of your own.
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

Noise is pointwise too, so grain multiplied into the picture is one loop; `origin` places the pattern:

```rust
let mut plan = Plan::new();
let grain = plan.scratch(cx.size);
let plan = plan
    .pass(PlanPass::noise(grain, 3.0, 2, cx.instance as u32, [0, 0]))
    .pass(PlanPass::blend_images(PlanImage::Source, grain, PlanImage::Output, BlendMode::Multiply, 0.3));
```

An HSL range mask fuses with the blend that uses it. To darken everything but the reds and add a few sparks:

```rust
let mut plan = Plan::new();
let (reds, darker) = (plan.scratch(cx.size), plan.scratch(cx.size));
let sparks = (0..200).map(|i| Dab::new([i as f32 * 9.0, 40.0 + (i % 7) as f32 * 30.0], 4.0, [1.0, 0.6, 0.2])).collect();
let plan = plan
    .pass(PlanPass::hsl_range(PlanImage::Source, reds, [340.0, 20.0], [0.3, 1.0], [0.1, 0.9], 0.05))
    .pass(PlanPass::blend_images(PlanImage::Source, reds, darker, BlendMode::Multiply, 0.5))
    .pass(PlanPass::splat(darker, PlanImage::Output, sparks));
```

`craft_plugin::histogram(pixels)` counts straight values per channel in the plugin itself, for example
for auto levels in `render`.

A time effect reads other frames into the plan with `plan.source_at(layer_time)` (declare
`.reads_other_frames()`) and can track motion between them with `PlanPass::flow`. A frame halfway between
two source frames, each pulled along its motion vectors (`plugins/timewarp` does this with a speed):

```rust
let mut plan = Plan::new();
let (before, after) = (plan.source_at(t0), plan.source_at(t1));
let (forward, backward, later) = (plan.scratch(cx.size), plan.scratch(cx.size), plan.scratch(cx.size));
let half = [[0.5, 0.0, 0.0, 0.0], [0.0, 0.5, 0.0, 0.0], [0.0, 0.0, 0.5, 0.0], [0.0, 0.0, 0.0, 0.5]];
let plan = plan
    .pass(PlanPass::flow(before, after, forward))
    .pass(PlanPass::flow(after, before, backward))
    .pass(PlanPass::warp(before, backward, PlanImage::Output, [0.5, 0.5]))
    .pass(PlanPass::warp(after, forward, later, [0.5, 0.5]))
    .pass(PlanPass::matrix(PlanImage::Output, PlanImage::Output, half))
    .pass(PlanPass::matrix(later, later, half))
    .pass(PlanPass::blend_images(PlanImage::Output, later, PlanImage::Output, BlendMode::Add, 1.0));
```

In tests, `plugin.render(..).source_at(|layer_time| picture)` gives the frames at other times.

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

## Read depth and normals

On a layer from a multi-layer OpenEXR file or a precomp of a 3D comp, `cx.aux` gives its 3D channels
on the frame's pixels. This fog fades pixels by their distance:

```rust
fn spec(&self) -> EffectSpec {
    EffectSpec::new("fog", "Depth Fog", "3D").reads_aux([Channel::Depth])
}

fn render(&self, cx: &Cx, frame: &mut Frame) -> Result<()> {
    let Some(aux) = cx.aux(&[Channel::Depth])? else { return Ok(()) };
    let x0 = frame.origin[0];
    for (y, row) in frame.rows_in_place() {
        for (x, pixel) in row.iter_mut().enumerate() {
            let depth = aux.at(&Channel::Depth, x0 + x, y).map_or(0.0, |d| d[0]);
            let (fog, alpha) = ((depth / 2000.0).clamp(0.0, 1.0), pixel[3]);
            pixel[..3].iter_mut().for_each(|c| *c += (alpha - *c) * fog);
        }
    }
    Ok(())
}
```

Asking for a channel the spec does not declare returns an error naming the declaration. See
[3D channels](plugin-reference.md#3d-channels).

## Offer presets

`.preset(name, values)` on the spec names a set of parameter values, for example a look popup that
also sets the sliders. The app applies one with `effect.applyPreset {layer, effect, preset}` in one
undo step and lists them with `effect.presets`. A preset writes absolute values to the parameters it names
and leaves the others alone. `plugins/halo` has a preset per look that sets the look and its element counts:

```rust
.preset("Sun", [("look", Value::Number(1.0)), ("Rays/count", Value::Number(14.0)), ("Ghosts/count", Value::Number(7.0))])
```

See [Presets](plugin-reference.md#presets).

## Ship presets and LUTs with no code

Presets for any effect and `.cube` LUT files need no Rust. `cargo xtask plugin new film-looks --kind data`
writes `plugin.json` and a `warm.cube`:

```json
{ "id": "com.example.film-looks", "name": "Film Looks", "version": "0.1.0",
  "contributes": {
    "presets": [{ "effect": "ec.color.tint", "name": "Warm", "values": { "white": [1.0, 0.92, 0.8, 1.0], "amount": 60.0 } }],
    "luts": ["warm.cube"] } }
```

Install the folder or its `.craftplugin`; the preset is in the Tint effect's Presets menu and the LUT in Apply
Color LUT's Library. There is no entry, no permission and no approval prompt. Mistakes are refused when you
install, with the effect, parameter or file named. See [Data-only plugins](plugin-reference.md#data-only-plugins).

## Give a slider its own ranges

A slider's hard range is what is allowed, and `cx.f` never returns a value outside it. The soft range is what the
slider shows, and `.log()` spaces it by ratio:

```rust
Param::slider("gain", "Gain", 1.0, 0.0..=1000.0).soft(0.01..=10.0).log()
Param::percent("mix", "Mix", 50.0).range(-100.0..=300.0)
```

## Show controls only when they apply, and add a button

```rust
EffectSpec::new("glow", "Glow", "Stylize").params([
    Param::popup("mode", "Mode", &["Plain", "Soft"], 0),
    Param::slider("softness", "Softness", 4.0, 0.0..=10.0).visible_when("mode", Is::Equals(1.0)),
    Param::checkbox("manual", "Manual", false),
    Param::slider("amount", "Amount", 1.0, 0.0..=10.0).enabled_when("manual", Is::On),
    Param::button("reset", "Reset"),
])

fn button(&self, press: &Press) -> Result<()> {
    let path = format!("effects/@{}/softness", press.instance);
    host::command("prop.set", json!({"layer": press.layer, "path": path, "value": 0.0})).map(drop)
}
```

Hidden and greyed parameters keep their values and `render` still gets them. Hiding is display only; a
greyed parameter refuses user edits but not the plugin's own `host::command` writes. Everything `button`
changes is one undo step.

## Colour by a gradient

A gradient parameter opens the app's Gradient Editor and can be keyframed; `sample` reads it:

```rust
params! {
    struct Settings {
        ramp: Gradient = gradient("Ramp", &[(0.0, [0.0, 0.0, 0.0, 1.0]), (1.0, [1.0, 0.6, 0.2, 1.0])]),
    }
}

fn render(&self, cx: &Cx, frame: &mut Frame) -> Result<()> {
    let ramp = Settings::read(cx).ramp;
    frame.map(|pixel| ramp.sample(f64::from(0.299 * pixel[0] + 0.587 * pixel[1] + 0.114 * pixel[2])));
    Ok(())
}
```

See [Gradients](plugin-reference.md#gradients).

## Add a curve

A curve parameter gets the app's curve editor, with undo, from one line. Read it per pixel or as a table:

```rust
params! { struct Tone { tone: Curve = curve("Tone", &[[0.0, 0.0], [0.5, 0.6], [1.0, 1.0]]) } }

fn render(&self, cx: &Cx, frame: &mut Frame) -> Result<()> {
    let lut = Tone::read(cx).tone.lut::<256>();
    frame.map(|[r, g, b, a]| {
        let at = |v: f32| lut[(v.clamp(0.0, 1.0) * 255.0) as usize];
        [at(r), at(g), at(b), a]
    });
    Ok(())
}
```

`curve(...).channels()` adds Red, Green and Blue curves beside the master one; read them with
`Vec<Curve>` or `cx.curves("tone")`.

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

### Open a floating window

Declare where a panel opens and how big it is, then open it from a button, a command or a tool:

```rust
Plugin::new()
    .panel("files", "Files", Files::default())
    .icon("list")
    .panel("inspector", "Inspector", Inspector::default())
    .place(PanelPlace::Float)
    .size(PanelSize::Large)
```

In `Files::ui`, `host::show_panel("inspector")?` opens the inspector as a floating window; its own Close
button calls `host::close_panel("inspector", None)`.

### Show a long list

```rust
let picked = ui.list("files", self.files.len(), &mut self.selected, |row| Item::new(&self.files[row]));
if let Some(row) = picked.activated {
    self.open(row);
}
```

Only the rows near the view are built, so the list can hold 100 000 files. Use `ui.table` for columns and
`Item::depth` and `Item::open` for a tree.

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
custom value opens without the plugin: the value is kept, and the effect is marked missing (see
[Projects that use plugins](#projects-that-use-plugins)).

The row subscribes to app events and arms timers the same way a panel does. This one shows the layers
selected in the app and blinks while it shows:

```rust
fn controls(&self, instance: u64, _param: &str, _value: &mut String, ui: &mut Ui) {
    let mut rows = self.rows.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
    let row = rows.entry(instance).or_default();
    if let Some(AppEvent::Selection { layers, .. }) = ui.on(Topic::Selection) {
        row.selected = layers.len();
    }
    if ui.every("blink", 500) {
        row.lit = !row.lit;
    }
    ui.label(&format!("{} layers selected{}", row.selected, if row.lit { " ●" } else { "" }));
}
```

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

The command appears in **Window ▸ Plug-ins** and in the command palette, and the user can give it a
shortcut in Edit ▸ Keyboard Shortcuts. To suggest one, add `.shortcut` after it:

```rust
Plugin::new().command("add-null", "Add Control Null", add_null).shortcut("Alt+Shift+N")
```

The app keeps the suggestion only when no other command uses those keys; otherwise the command starts
with no shortcut and the log says which command holds them.

### Put a command in a menu, a context menu and the toolbar

```rust
Plugin::new()
    .command("add-null", "Add Control Null", add_null)
    .menu("Layer/Rigging")
    .context(Context::Layers)
    .toolbar()
    .icon("plus")
```

This puts **Add Control Null** at the end of **Layer ▸ Rigging**, in the timeline's layer context menu and
as a toolbar button with a plus sign. All of them run the same command, with the same shortcut.

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
let image = host::frame(FrameRequest { layer: Some("Logo".into()), time: 1.0, scale: 0.5, ..Default::default() })?;
let audio = host::audio(AudioRequest { layer: None, start: 0.0, duration: 2.0, rate: 48_000 })?;
```

With `source: true` the frame is the layer's source before its masks, effects and transform, in layer
pixels at the layer's time for the comp time asked: what a tracker reads. With `ids: true`, `comp` and
`layer` are ids (`comp.info`'s `id`, the selection's layers), never names, so a layer named `"7"` cannot
stand in for layer 7. An effect cannot call either while it renders.

## Ask the user for a file

```rust
let picked = host::pick_file(PickFile { save: false, title: "Choose a LUT".into(), extensions: vec!["cube".into()] })?;
if let Some(path) = picked {
    let lut = host::read_file(Root::User, &path)?;   // no files.read needed: the user chose it
}
```

Panels and commands only. With `save: true` the path is also writable.

## Ask the user for settings

```rust
let form = Form::new("Add Grid")
    .field(Param::slider("rows", "Rows", 4.0, 1.0..=50.0))
    .field(Param::checkbox("guides", "Show Guides", true));
if let Some(values) = host::form(form)? {
    let rows = values["rows"].clone();   // None: the user cancelled
}
```

The app draws the form as a dialog with OK and Cancel. Panels and commands only.

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

## Keep data in the project

A tracker analyses frames in a task and stores the result on the layer; its effect reads it back.
The plugin needs `document.read` and `document.write`.

```rust
.task("track", "Track", |task: &Task| {
    let layer = task.args()["layer"].as_u64().ok_or("track needs `layer`")?;
    let points: Vec<[f32; 2]> = analyse(task)?;
    host::set_data(Place::Layer(layer), "points", &serde_json::to_vec(&points)?)
})
```

```rust
fn render(&self, cx: &Cx, frame: &mut Frame) -> Result<()> {
    let points: Vec<[f32; 2]> = match cx.layer_data("points")? {
        Some(bytes) => serde_json::from_slice(&bytes)?,
        None => return Ok(()),
    };
    // …
}
```

A test gives the task its own frames and reads what it stored:

```rust
let plugin = TestPlugin::of(super::plugin);
plugin.frames(|request| moving_square(request.time));
plugin.task("track", json!({"layer": 7}))?;
let points = plugin.data(Place::Layer(7), "points");
plugin.set_data(Place::OwnLayer, "points", &points.unwrap());
let still = plugin.render("stabilise").time(0.5).run()?;
```

`plugin.start`, `plugin.cancel` and `plugin.wait` test Cancel. `plugins/tracker` is a complete example.

The write is one undo step and is saved with the project. When the data changes, the layer's
cached frames redraw. Use `Place::Project` for data that belongs to the whole project.

## Projects that use plugins

A saved project lists the plugins its effects come from, so nothing is needed in the plugin. Bump
`version` in `plugin.json` with each release: a project opened with another version says so.

```json
"plugins": [{"id": "com.example.tint", "version": "1.2.0"}]
```

Without the plugin the project still opens and plays; the effect keeps its values byte for byte and
Effect Controls marks it missing. A final render refuses until the plugin is installed:

```sh
effectcraft-cli render film.ecproj --queue --without-missing-plugins
```

## Rename a parameter without breaking old projects

Raise the effect's state version and move old values in `migrate`. Projects saved before open with the
value (and its keyframes) under the new id; a project from a newer release opened with this one keeps
everything it does not know.

```rust
fn spec(&self) -> EffectSpec {
    EffectSpec::new("glow", "Glow", "Light").version(1).param(Param::slider("strength", "Strength", 0.5, 0.0..=1.0))
}

fn migrate(&self, from: u32, values: &mut BTreeMap<String, Value>) {
    if from < 1 && let Some(amount) = values.remove("amt") {
        values.insert("strength".into(), amount);
    }
}
```

Test it with `plugin.migrate("glow", 0, saved)`, which returns the migrated values.

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

The approval covers these exact files. After a rebuild, raise `version` in `plugin.json` before installing
again, or `plugin.uninstall` first: the same version with other files is refused. If the app crashes while
your native plugin runs, it starts turned off next time; approve it again, or start with
`EFFECTCRAFT_SAFE_MODE=1` to open the app with no plugins.

## Check your work

- `effectcraft-cli exec plugin.list` shows each plugin's state and, when a plugin fails to load, the error.
- `effectcraft-cli exec plugin.log '{"id": "com.example.tint"}'` lists the plugin's log lines, panics and
  failed calls; pass the last `seq` seen as `after` to get only newer ones.
- `cargo xtask plugin render plugins/tint` renders an effect on a test pattern without the app and prints the
  render time.
- A render error shows as a warning mark on the effect in Effect Controls. The frame passes through
  unchanged.
