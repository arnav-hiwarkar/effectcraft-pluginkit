# Write a plugin

This page takes you from nothing to an effect running in EffectCraft, then shows how to package it, and
covers panels, commands, and native builds. For every type and call, see the [plugin reference](plugin-reference.md).
For how the system works, see [About the plugin system](plugin-system.md).

You need Rust and the WebAssembly target: `rustup target add wasm32-unknown-unknown`.

## Make an effect in five minutes

Start the app with its control channel on, then make a plugin and run it.

```sh
cargo run -p effectcraft -- --control 9877     # in one terminal
cargo xtask plugin new tint                    # in another: writes plugins/tint
cargo xtask plugin dev plugins/tint            # builds it and loads it into the running app
```

`dev` prints one line each time it loads the plugin:

```
reloaded com.example.tint: built in 7.9 s
```

In the app, apply **Tint** from Effects & Presets, Color Correction. Now open `plugins/tint/src/lib.rs`
and change `render`: for example, replace `*channel *= gain;` with `*channel = 1.0 - *channel * gain;`.
Save. A few seconds later `dev` prints a new line and the frame in the app changes. `dev` checks your files
once a second and stops when you press Ctrl-C.

`cargo xtask plugin new` takes two options. `--panel` adds a panel, and `--gpu` adds a GPU kernel to the
effect. The new plugin has its own test: `cargo test --manifest-path plugins/tint/Cargo.toml`.

You need Rust and the WebAssembly target: `rustup target add wasm32-unknown-unknown`.

### What is in the folder

```
tint/
  Cargo.toml
  plugin.json
  src/lib.rs
```

`plugin.json` names the plugin and what it asks for:

```json
{
  "id": "com.example.tint",
  "name": "Tint",
  "version": "0.1.0",
  "author": "",
  "description": "Tint, made with cargo xtask plugin new.",
  "entry": { "wasm": "plugin.wasm" },
  "permissions": []
}
```

`src/lib.rs` is the effect:

```rust
use craft_plugin::prelude::*;

struct TintEffect;

impl Effect for TintEffect {
    fn spec(&self) -> EffectSpec {
        EffectSpec::new("tint", "Tint", "Color Correction").param(Param::percent("amount", "Amount", 100.0))
    }

    fn layout(&self, _cx: &Cx) -> Layout {
        Layout::pointwise()
    }

    fn render(&self, cx: &Cx, frame: &mut Frame) -> Result<()> {
        let gain = (cx.f("amount") / 100.0) as f32;
        for pixel in frame.pixels.iter_mut() {
            for channel in &mut pixel[..3] {
                *channel *= gain;
            }
        }
        Ok(())
    }
}

fn plugin() -> Plugin {
    Plugin::new().effect(TintEffect)
}

craft_plugin::export!(plugin);
```

The effect appears in Effects & Presets under Color Correction, and Effect Controls shows the Amount row
with a stopwatch.

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
- If each row depends on rows at most `n` away, return `Layout::neighbourhood(n)`. Each band is
  rendered with `n` extra rows above and below, so a smaller `n` does less repeated work.
- If the effect spills past the layer, add `.padded(n)`.
- If the effect needs the whole frame, describe it as passes: `Layout::whole_frame().passes(plan)`. The
  app runs each pass on the GPU when there is one, and otherwise calls your `pass` method for bands of
  each pass on all cores. See [Passes](plugin-gpu.md).

Scale every distance by `cx.scale`, so a half-resolution preview matches the full render. `cx.px(id)`
does this for a slider measured in layer pixels.

Check a plan in a unit test with the same check the app runs:

```rust
let plan = GpuPlan::new().pass(GpuPass::new("tint_apply", GpuImage::Source, GpuImage::Output).floats(&[1.0, 0.8, 0.6]));
assert_eq!(plan.validate([1920, 1080]), Ok(()));
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
    for pixel in frame.pixels.iter_mut() {
        pixel[..3].iter_mut().for_each(|c| *c = linear_to_srgb(saturate(*c * gain)));
    }
    Ok(())
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

## Offer presets

`.preset(name, values)` on the spec names a set of parameter values, for example a look popup that
also sets the sliders. The app applies one with `effect.applyPreset {layer, effect, preset}` in one
undo step and lists them with `effect.presets`. `plugins/halo` has a preset per look. See
[Presets](plugin-reference.md#presets).

## Add a panel

A panel keeps its own state. The app calls `ui` every frame the tab is visible.

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

When `controls` changes `value`, the app sets the parameter. One drag is one undo step. `controls` runs
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
Everything one press-to-release gesture changes in the project is one undo step, named after the tool.
Changing the project needs the `document.write` permission.

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

Start it from a panel or a command, then ask for its state each frame:

```rust
let id = host::start("analyse", serde_json::json!({ "layer": "Music" }))?;
let state = host::task(id)?;      // status, progress, message, result, error
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
- `cargo run --release -p craft-plugin-host --example render -- <bundle> <effect> out.png` renders an effect
  on a test pattern without the app and prints the render time. Run it with no arguments for the options.
- A render error shows as a warning mark on the effect in Effect Controls. The frame passes through
  unchanged.
