# Plugin quickstart

From nothing to your own effect in EffectCraft in about five minutes. Every command runs from the root of
the EffectCraft checkout. For the whole guide see [Write a plugin](plugin-sdk.md); for short recipes, the
[cookbook](plugin-cookbook.md).

You need Rust and its WebAssembly target:

```sh
rustup target add wasm32-unknown-unknown
```

## 1. Make the plugin

```sh
cargo xtask plugin new my-tint
```

```
made plugins/my-tint from the effect template
  test it:      cargo test --manifest-path plugins/my-tint/Cargo.toml
  look at it:   cargo xtask plugin render plugins/my-tint
  run it live:  cargo xtask plugin dev plugins/my-tint   (with the app started by: cargo run -p effectcraft -- --control 9877)
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
`src/lib.rs`. The parameters are declared once, and the effect reads them into a struct:

```rust
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
```

## 2. Look at it without the app

```sh
cargo xtask plugin render plugins/my-tint
```

```
rendered `my-tint` at 320x180 in 0.27 ms (sandboxed): plugins/my-tint/target/my-tint.png
```

It builds the sandboxed module and renders the effect on a built-in test image. Set parameters, the image
and the size:

```sh
cargo xtask plugin render plugins/my-tint --set amount=50 --set tint=0.2,0.6,1,1 --pattern checker --size 640x360
```

The patterns are `photo`, `gradient`, `checker`, `shapes`, `text` and `empty`; `--in picture.png` renders on
your own picture, and `--native` uses a native build.

## 3. Test it

```sh
cargo test --manifest-path plugins/my-tint/Cargo.toml
```

The test renders the effect in the test process, through the same host code the app uses, and compares it
with a stored frame. The first run stores `plugins/my-tint/tests/frames/full.png`; look at it.

```rust
#[cfg(test)]
mod tests {
    use craft_plugin_host::assert_frame;
    use craft_plugin_host::testing::TestPlugin;

    #[test]
    fn full_amount_tints_the_test_image() {
        assert_frame!(TestPlugin::of(super::plugin), "my-tint", { "amount": 100 }, "full");
    }

    #[test]
    fn zero_amount_leaves_every_pixel_as_it_was() {
        let plugin = TestPlugin::of(super::plugin);
        let render = plugin.render("my-tint").set("amount", 0.0);
        assert_eq!(render.run().unwrap(), render.source());
    }
}
```

## 4. Change it

Open `plugins/my-tint/src/lib.rs` and change `render`, for example to invert before tinting:

```rust
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
```

Replace `[r * gain[0], g * gain[1], b * gain[2], a]` with
`[(a - r) * gain[0], (a - g) * gain[1], (a - b) * gain[2], a]` and run the tests again. Both fail: an amount
of 0 now changes the picture, and the frame differs from the stored one. The frame test says how many pixels
changed and by how much, and writes `full.actual.png` and `full.diff.png` (red marks every pixel that
changed) beside the stored frame:

```
frame `full` differs from the stored frame: 57600 of 57600 pixels are off by more than 1/255 (largest 255/255, first at (0, 0))
  expected:   …/plugins/my-tint/tests/frames/full.png
  actual:     …/plugins/my-tint/tests/frames/full.actual.png
  difference: …/plugins/my-tint/tests/frames/full.diff.png (red marks the pixels that differ)
If the new frame is right, run again with UPDATE_FRAMES=1.
```

A typo stops the build with the file and line:

```
src/lib.rs:23:29: error[E0609]: no field `amont` on type `Settings`: unknown field
```

If the new picture is what you want, change or remove the zero-amount test and store the frame:

```sh
UPDATE_FRAMES=1 cargo test --manifest-path plugins/my-tint/Cargo.toml
```

## 5. Check it

```sh
cargo xtask plugin check plugins/my-tint
```

```
checked plugins/my-tint: 0 errors, 0 warnings
```

`check` builds the plugin and reports, each with its fix: unknown permissions, permissions the code never
needs or needs and does not ask for, parameters read by a misspelt id, parameters declared with `params!`
and never read, ids that clash, a module built without the standard flags, a package over the size budget,
and a missing licence or description. It exits with an error when anything would stop the plugin working.

## 6. Run it in the app

Start the app with its control channel, then let `dev` rebuild and reload the plugin on every save:

```sh
cargo run -p effectcraft -- --control 9877
cargo xtask plugin dev plugins/my-tint
```

Apply **My Tint** from Effects & Presets, Color Correction. Each save prints one line and the frame in the
app changes:

```
reloaded com.example.my-tint: built in <seconds> s
```

Compile errors print with their file and line, and so do the plugin's own `log!` lines, panics and failed
calls while the app runs it:

```
[error] the plugin panicked at src/lib.rs:27:17: index out of bounds: the len is 4 but the index is 4
[error] render of effect `my-tint` failed: the plugin stopped: …
```

## 7. Share it

```sh
cargo xtask plugin plugins/my-tint
```

This writes `plugins/dist/com.example.my-tint-0.1.0.craftplugin`, one file a user installs with **Window ▸
Plug-ins ▸ Manage Plug-ins ▸ Install Plug-in...**. `cargo xtask plugin bench plugins/my-tint` times the
sandboxed and the native build at 1080p and 4K.

## When something goes wrong

| You see | Do |
|---|---|
| `effect `my-tint` has no parameter `amout`; its parameters are ["amount", "tint"]` | Use one of the ids listed; `params!` fields avoid this |
| `effect `x` declares that each row reads rows at most 0 away, but rendering in bands changed pixel (12, 90)` | Return `Layout::neighbourhood(rows)` with the distance the effect reads, or `Layout::whole_frame()` |
| `permission denied: this plugin does not have `document.write`` | Add it to `permissions` in `plugin.json` |
| `the plugin panicked at src/lib.rs:27:17: …` | The file and line are where your code panicked |
| `frame `full` differs from the stored frame: …` | Open the `.diff.png`; accept with `UPDATE_FRAMES=1` if the change is right |
| A warning mark on the effect in Effect Controls | The render returned an error; `dev` prints it, and so does `effectcraft-cli exec plugin.log '{"id": "com.example.my-tint"}'` |
