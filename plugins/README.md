# Plugins

Each folder here is one plugin: a standalone Rust crate outside the workspace, built against the SDK in
`crates/plugin-sdk`. They are the reference for what a plugin can do.

| Plugin | Kind | What it does |
|---|---|---|
| [Lumen Glow](lumen-glow) | Effect | A wide, energy-conserving glow from a pyramid of blurs, with threshold, falloff, aspect, tint, chromatic aberration and views |
| [Plasma Beam](plasma-beam) | Effect | A white-hot core with layered coloured glow between two points or along the layer's masks, with presets, taper, trim, flicker and distortion |
| [Glide](glide) | Panel and commands | A cubic-bezier easing editor with a preset library that applies the curve to the selected keyframes in one undo step |

## Build and run

```sh
cargo xtask plugin plugins/lumen-glow        # writes plugins/dist/org.craftkit.lumen-glow
```

To load bundles when the app starts, list their folders in `EFFECTCRAFT_PLUGIN_PATH`, separated like `PATH`:

```sh
EFFECTCRAFT_PLUGIN_PATH="$PWD/plugins/dist/org.craftkit.lumen-glow:$PWD/plugins/dist/org.craftkit.glide" cargo run -p effectcraft
```

To install a bundle for good, use **Window ▸ Plug-ins ▸ Manage Plug-ins ▸ Install Plug-in...** and approve it.

`plugins/e2e.sh` builds every plugin here, loads the bundles in `effectcraft-cli`, renders a frame with each
effect and runs `glide/verify.sh`.

To write your own, read [Write a plugin](../docs/plugin-sdk.md).
