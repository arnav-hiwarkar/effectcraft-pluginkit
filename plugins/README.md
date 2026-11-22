# Plugins

Each folder here is one plugin: a standalone Rust crate outside the workspace, built against the SDK in
`crates/plugin-sdk`. They are the reference for what a plugin can do.

| Plugin | Kind | What it does |
|---|---|---|
| [Lumen Glow](lumen-glow) | Effect | A wide, energy-conserving glow from a pyramid of blurs, with threshold, falloff, aspect, tint, chromatic aberration and views |
| [Plasma Beam](plasma-beam) | Effect | A white-hot core with layered coloured glow between two points or along the layer's masks, with presets, taper, trim, flicker and distortion |
| [Halo Flares](halo) | Effect | A lens flare from a light point: core glow, star rays, anamorphic streak, chromatic ring, iris ghosts along the line through the centre, lens-dirt bloom and a hoop, with six looks, edge trigger, shimmer, occlusion by another layer and Add or Screen compositing |
| [Glide](glide) | Panel and commands | A cubic-bezier easing editor with a preset library that applies the curve to the selected keyframes in one undo step |

## Halo Flares

Pick a **Look** (Sun, Anamorphic Blue, Warm Studio, Sci-Fi Ring, Vintage Zoom, Night Street, or Custom) and
move **Light Position**. Each element group (Glow, Rays, Streak, Ring, Ghosts, Dirt, Hoop) scales the look's
brightness, size and shape by percentages and tints it with its own colour; ray count, ghost count and iris
blades take the look's value at 0. **Behaviour** dims the flare as the light reaches the layer edge, shimmers
the rays with time, and, with **Occlusion** on, dims it by the alpha of the **Occluding Layer** around the light.
Sizes are in layer pixels and follow the render scale.

The flare is a function of pixel position, so the CPU render is pointwise and the GPU render is one kernel,
`halo_render`, fed by the same 48 floats and 6 integers (`src/flare.rs` and `src/halo.wgsl` hold the same element
functions, including the exp, log, sine and arctangent approximations). `plugins/halo/check` runs the plugin
through the app and checks that both paths agree on every look at 1080p:

```sh
cd plugins/halo/check && CARGO_TARGET_DIR=../../../target cargo test -j 4
```

## Build and run

```sh
cargo xtask plugin plugins/lumen-glow
```

This writes the unpacked folder `plugins/dist/org.craftkit.lumen-glow` and the package
`plugins/dist/org.craftkit.lumen-glow-<version>.craftplugin`. Add `--native` to put this machine's native
build in the same package.

To install the package, use **Window ▸ Plug-ins ▸ Manage Plug-ins ▸ Install Plug-in...** and approve it, or:

```sh
effectcraft-cli exec plugin.install '{"path": "plugins/dist/org.craftkit.lumen-glow-<version>.craftplugin"}'
effectcraft-cli exec plugin.approve '{"id": "org.craftkit.lumen-glow"}'
```

To try an unpacked folder without installing it, list the folders in `EFFECTCRAFT_PLUGIN_PATH`, separated like `PATH`:

```sh
EFFECTCRAFT_PLUGIN_PATH="$PWD/plugins/dist/org.craftkit.lumen-glow:$PWD/plugins/dist/org.craftkit.glide" cargo run -p effectcraft
```

To write a plugin, run `cargo xtask plugin new <name> [--panel] [--gpu]`, then `cargo xtask plugin dev plugins/<name>`
with the app running (`cargo run -p effectcraft -- --control 9877`). `plugins/new-plugin-check.sh` generates each kind
of new plugin, runs its test and builds it for WebAssembly.

`plugins/e2e.sh` builds every plugin here, installs Plasma Beam from its `.craftplugin` file, loads the other
bundles in `effectcraft-cli`, renders a frame with each effect and runs `glide/verify.sh`.

To write your own, read [Write a plugin](../docs/plugin-sdk.md).
