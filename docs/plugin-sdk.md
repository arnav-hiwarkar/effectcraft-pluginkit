# Write a plugin

This page takes you from an empty folder to an effect running in EffectCraft, then shows panels,
commands, and native builds. For every type and call, see the [plugin reference](plugin-reference.md).
For how the system works, see [About the plugin system](plugin-system.md).

You need Rust and the WebAssembly target: `rustup target add wasm32-unknown-unknown`.

## Create an effect

A plugin is a folder with three files.

```
my-plugin/
  Cargo.toml
  plugin.json
  src/lib.rs
```

`Cargo.toml`:

```toml
[package]
name = "my-plugin"
version = "0.1.0"
edition = "2024"

[lib]
crate-type = ["cdylib"]

[dependencies]
craft-plugin = { path = "../../crates/plugin-sdk" }   # from a plugin in this repository's plugins/ folder

[profile.release]
opt-level = 3
lto = true
panic = "abort"

[workspace]
```

`plugin.json`:

```json
{
  "id": "com.example.tint",
  "name": "Tint",
  "version": "1.0.0",
  "author": "Example",
  "description": "Tints a layer.",
  "entry": { "wasm": "plugin.wasm" },
  "permissions": []
}
```

`src/lib.rs`:

```rust
use craft_plugin::prelude::*;

struct Tint;

impl Effect for Tint {
    fn spec(&self) -> EffectSpec {
        EffectSpec::new("tint", "Tint", "Color Correction")
            .param(Param::color("color", "Color", [1.0, 0.5, 0.0, 1.0]))
            .param(Param::percent("amount", "Amount", 100.0))
    }

    fn layout(&self, _cx: &Cx) -> Layout {
        Layout::pointwise()
    }

    fn render(&self, cx: &Cx, frame: &mut Frame) -> Result<()> {
        let tint = cx.color("color");
        let amount = (cx.f("amount") / 100.0) as f32;
        for pixel in frame.pixels.iter_mut() {
            let luma = 0.2126 * pixel[0] + 0.7152 * pixel[1] + 0.0722 * pixel[2];
            for c in 0..3 {
                pixel[c] += (luma * tint[c] - pixel[c]) * amount;
            }
        }
        Ok(())
    }
}

fn plugin() -> Plugin {
    Plugin::new().effect(Tint)
}

craft_plugin::export!(plugin);
```

Build and load it:

```sh
cargo xtask plugin plugins/my-plugin          # writes plugins/dist/com.example.tint/
effectcraft-cli exec plugin.load '{"path": "plugins/dist/com.example.tint"}'
```

`cargo xtask plugin` builds the crate for WebAssembly and writes a bundle: `plugin.json`, the module, and
the `assets` folder if the plugin has one. `plugin.load` runs a bundle for the current session with the
permissions it asks for. To load bundles every time the app starts while you develop, list their folders in
`EFFECTCRAFT_PLUGIN_PATH`.

The effect is now in Effects & Presets under Color Correction. Apply it, and Effect Controls shows the
Color and Amount rows with stopwatches.

Users install a bundle through **Plug-ins ▸ Manage Plug-ins**. The app copies the bundle into its plugin
folder and runs nothing until the user approves the permissions.


## Make the effect fast

Tell the app how much of the frame each output pixel needs, in `layout()`.

- If each pixel depends only on itself, return `Layout::pointwise()`. The app renders bands of the frame
  on all cores.
- If each row depends on rows at most `n` away, return `Layout::neighbourhood(n)`.
- If the effect spills past the layer, add `.padded(n)`.
- If the effect needs the whole frame, do the heavy work at reduced resolution, or run it on the GPU.
  See [GPU effects](plugin-gpu.md).

Scale every distance by `cx.scale`, so a half-resolution preview matches the full render. `cx.px(id)`
does this for a slider measured in layer pixels.

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

The panel appears in the **Plug-ins** menu. Its tab docks like the app's own panels.

## Add a command

```rust
Plugin::new().command("add-null", "Add Control Null", || {
    host::command("layer.newNull", serde_json::json!({ "name": "Control" }))?;
    Ok(())
})
```

The command appears in the **Plug-ins** menu.

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

## Store files

```rust
host::write_file(Root::Data, "presets.json", bytes)?;   // private to the plugin, always allowed
let lut = host::read_file(Root::Bundle, "assets/film.cube")?;  // shipped with the plugin, read-only
let any = host::read_file(Root::User, "/abs/path")?;    // needs files.read
```

## Build a native plugin

Build a native plugin only for work the sandbox cannot do. The source does not change.

```sh
cargo xtask plugin plugins/my-plugin --native
```

The bundle's `entry` then names the library for this platform. A native plugin renders in place with no
copy and may use threads. The user sees a warning that it runs with full access before enabling it.

## Check your work

- `effectcraft-cli exec plugin.list` shows each plugin's state and, when a plugin fails to load, the error.
- `cargo run --release -p craft-plugin-host --example render -- <bundle> <effect> out.png` renders an effect
  on a test pattern without the app and prints the render time. Run it with no arguments for the options.
- A render error shows as a warning mark on the effect in Effect Controls. The frame passes through
  unchanged.
