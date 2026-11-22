# Plugin quickstart

From nothing to your own effect in EffectCraft in about five minutes. For the whole guide see
[Write a plugin](plugin-sdk.md); for short recipes, the [cookbook](plugin-cookbook.md).

You need Rust 1.95 or newer (`rustc --version` prints it) with its WebAssembly target, and a checkout of
EffectCraft. Add the target once:

```sh
rustup target add wasm32-unknown-unknown
```

You use two terminals, both in the root of the EffectCraft checkout: **terminal 1** runs the app, **terminal
2** runs every other command on this page. Until step 6 you need only terminal 2.

## 1. Make the plugin

In terminal 2:

```sh
cargo xtask plugin new my-tint
```

```
made plugins/my-tint from the effect template
  test it:      cargo test --manifest-path plugins/my-tint/Cargo.toml
  look at it:   cargo xtask plugin render plugins/my-tint
  check it:     cargo xtask plugin check plugins/my-tint
  run it live:  cargo xtask plugin dev plugins/my-tint   (in a second terminal; start the app in the first with: cargo run -p effectcraft -- --control 9877)
  then apply My Tint from Effects & Presets, Color Correction
```

`--kind` picks what to start from:

| Kind | Starts with |
|---|---|
| `effect` (the default) | A pointwise effect with typed parameters and a stored-frame test |
| `gpu-effect` | An effect with its own WGSL kernel and the same pass on the CPU |
| `panel` | A panel that follows the selection and adds a null with a button |
| `tool` | A viewer tool that drops a null where you click |
| `task` | A panel that runs a background task with progress and cancel |

The folder holds `Cargo.toml`, `plugin.json` (the id, name, licence and the permissions it asks for) and
`src/lib.rs`. The parameters are declared once with `params!`, the effect reads them into a struct, and
`export!` makes the plugin loadable. This is the whole `src/lib.rs` the command wrote:

`plugins/my-tint/src/lib.rs`:

```rust plugins/my-tint/src/lib.rs
use craft_plugin::prelude::*;

params! {
    struct Settings {
        amount: f64 = percent("Amount", 100.0),
        tint: [f32; 4] = color("Tint", [1.0, 0.55, 0.25, 1.0]),
    }
}

struct Tint;

impl Effect for Tint {
    fn spec(&self) -> EffectSpec {
        EffectSpec::new("my-tint", "My Tint", "Color Correction").params(Settings::params())
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

fn plugin() -> Plugin {
    Plugin::new().effect(Tint)
}

craft_plugin::export!(plugin);

#[cfg(test)]
mod tests {
    use craft_plugin_host::assert_frame;
    use craft_plugin_host::testing::TestPlugin;

    #[test]
    fn full_amount_tints_the_test_image() {
        let plugin = TestPlugin::of(super::plugin);
        assert_frame!(plugin.render("my-tint").set("amount", 100.0), "full");
    }

    #[test]
    fn zero_amount_leaves_every_pixel_as_it_was() {
        let plugin = TestPlugin::of(super::plugin);
        let render = plugin.render("my-tint").set("amount", 0.0);
        assert!(render.run().unwrap() == render.source(), "an amount of 0 changed the picture");
    }
}
```

## 2. Look at it without the app

```sh
cargo xtask plugin render plugins/my-tint
```

```
rendered `my-tint` at 320x180 in 0.39 ms (sandboxed): plugins/my-tint/target/my-tint.png
```

It builds the sandboxed module and renders the effect on a built-in test image. Set parameters, the image
and the size:

```sh
cargo xtask plugin render plugins/my-tint --set amount=50 --set tint=0.2,0.6,1,1 --pattern checker --size 640x360
```

```
rendered `my-tint` at 640x360 in 0.75 ms (sandboxed): plugins/my-tint/target/my-tint.png
```

`--set` reads each value as the kind its parameter declares (a number, a colour as `r,g,b` or `r,g,b,a`, …)
and a wrong value names the parameter and what it takes. The patterns are `photo`, `gradient`, `checker`,
`shapes`, `text` and `empty`; `--in picture.png` renders on your own picture, and `--native` uses a native
build.

## 3. Test it

```sh
cargo test --manifest-path plugins/my-tint/Cargo.toml
```

```
running 2 tests
test tests::zero_amount_leaves_every_pixel_as_it_was ... ok
test tests::full_amount_tints_the_test_image ... ok
test result: ok. 2 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.04s
```

The tests at the end of `src/lib.rs` render the effect in the test process, through the same host code the
app uses. `full_amount_tints_the_test_image` compares the render with a stored frame: the first run stores
`plugins/my-tint/tests/frames/full.png`; look at it. `zero_amount_leaves_every_pixel_as_it_was` checks
that an amount of 0 changes nothing.

## 4. Change it

Make the effect invert the picture before it tints it. In `plugins/my-tint/src/lib.rs` change the line in
`render`:

```diff
-        frame.map(|[r, g, b, a]| [r * gain[0], g * gain[1], b * gain[2], a]);
+        frame.map(|[r, g, b, a]| [(a - r) * gain[0], (a - g) * gain[1], (a - b) * gain[2], a]);
```

and run the tests again. Both fail. The zero-amount test says `an amount of 0 changed the picture`, and the
frame differs from the stored one. The frame test says how many pixels changed and by how much, and writes `full.actual.png` and
`full.diff.png` (red marks every pixel that changed) under the plugin's `target/frames/`:

```
frame `full` differs from the stored frame: 57600 of 57600 pixels are off by more than 1/255 (largest 255/255, first at (0, 0))
  expected:   …/plugins/my-tint/tests/frames/full.png
  actual:     …/plugins/my-tint/target/frames/full.actual.png
  difference: …/plugins/my-tint/target/frames/full.diff.png (red marks the pixels that differ)
If the new frame is right, run again with UPDATE_FRAMES=1.
```

The zero-amount test was right before and is wrong now: with an amount of 0 the effect should invert and
nothing more. Say so with the inverse of a pixel you know. The test image's top-left pixel is the opaque
blue `[0.2509722, 0.45083332, 0.94930553, 1.0]`, and inverted it is `[0.7490278, 0.5491667, 0.050694466,
1.0]`, so change the test to:

```diff
-    fn zero_amount_leaves_every_pixel_as_it_was() {
+    fn zero_amount_only_inverts() {
         let plugin = TestPlugin::of(super::plugin);
         let render = plugin.render("my-tint").set("amount", 0.0);
-        assert!(render.run().unwrap() == render.source(), "an amount of 0 changed the picture");
+        assert_eq!(render.source().at(0, 0), [0.2509722, 0.45083332, 0.94930553, 1.0]);
+        assert_eq!(render.run().unwrap().at(0, 0), [0.7490278, 0.5491667, 0.050694466, 1.0]);
     }
```

Then store the new frame, look at `full.png`, and run the tests once more without `UPDATE_FRAMES`:

```sh
UPDATE_FRAMES=1 cargo test --manifest-path plugins/my-tint/Cargo.toml
cargo test --manifest-path plugins/my-tint/Cargo.toml
```

A typo stops the build with the file and line:

```
error[E0609]: no field `amont` on type `Settings`
  --> src/lib.rs:23:29
   |
23 |         let mix = (settings.amont / 100.0) as f32;
   |                             ^^^^^ unknown field
   |
help: a field with a similar name exists
   |
23 |         let mix = (settings.amount / 100.0) as f32;
   |                                +
```

## 5. Check it

```sh
cargo xtask plugin check plugins/my-tint
```

```
checked plugins/my-tint: 0 errors, 0 warnings
```

`check` builds the plugin, loads it with the same checks the app runs when it loads a plugin (the manifest,
the permissions it asks for, reserved and malformed ids) and renders each effect once at its default
settings on a small test image. It reports, each with its fix, anything the app would refuse, a render
that fails, ids that clash, a module built without the standard flags, a package over the size budget, and
a missing licence or description. It exits with an error when anything would stop the plugin working.

## 6. Run it in the app

In terminal 1, start the app with its control channel:

```sh
cargo run -p effectcraft -- --control 9877
```

In terminal 2, let `dev` rebuild and reload the plugin on every save:

```sh
cargo xtask plugin dev plugins/my-tint
```

```
watching plugins/my-tint and loading it into the app on port 9877; Ctrl-C stops
```

`dev` loads the plugin for this session only, with the permissions its `plugin.json` asks for, so there is
nothing to approve. In the app, make something to apply it to: **Composition ▸ New Composition...**
(Cmd+N) and **OK**, then **Layer ▸ New ▸ Solid...** (Cmd+Y) and **OK**. With the solid selected, apply
**My Tint** from Effects & Presets, Color Correction. Each save prints one line and the frame in the app
changes:

```
reloaded com.example.my-tint: built in <seconds> s
```

A failed build prints the compiler's message and leaves the running plugin as it was. The plugin's own
`log!` lines, panics and failed calls print too while the app runs it:

```
[error] the plugin panicked at src/lib.rs:27:17: index out of bounds: the len is 4 but the index is 4
[error] render of effect `my-tint` failed: the plugin stopped: …
```

To read the log without `dev`, ask the running app from terminal 2:

```sh
cargo run -p effectcraft-cli -- exec plugin.log '{"id": "com.example.my-tint"}' --bridge 9877
```

## 7. Share it

```sh
cargo xtask plugin plugins/my-tint
```

```
…/plugins/dist/com.example.my-tint
…/plugins/dist/com.example.my-tint-0.1.0.craftplugin
```

The `.craftplugin` file is what a user installs: **Window ▸ Plug-ins ▸ Manage Plug-ins... ▸ Install
Plug-in...**, choose the file, then click **Allow and Enable** beside it to grant the permissions it asks
for and start it. `cargo xtask plugin bench plugins/my-tint` times the sandboxed and the native build at
1080p and 4K.

## When something goes wrong

| You see | Do |
|---|---|
| `--set amout=50: effect `my-tint` has no parameter `amout`; its parameters are amount, tint` | Use one of the ids listed |
| `effect `x` declares that each row reads rows at most 0 away, but rendering in bands changed pixel (12, 90)` | Return `Layout::neighbourhood(rows)` with the distance the effect reads, or `Layout::whole_frame()` |
| `permission denied: this plugin does not have `document.write`` | Add it to `permissions` in `plugin.json` |
| `the plugin panicked at src/lib.rs:27:17: …` | The file and line are where your code panicked |
| `frame `full` differs from the stored frame: …` | Open the `.diff.png`; accept with `UPDATE_FRAMES=1` if the change is right |
| A warning mark on the effect in Effect Controls | The render returned an error; `dev` prints it, and so does the `plugin.log` command in step 6 |
