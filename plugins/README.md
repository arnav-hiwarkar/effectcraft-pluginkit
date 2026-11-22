# Plugins

Each folder here is one plugin: a standalone Rust crate outside the workspace, built against the SDK in
`crates/plugin-sdk`. They are the reference for what a plugin can do.

| Plugin | Kind | What it does |
|---|---|---|
| [Lumen Glow](lumen-glow) | Effect | A wide, energy-conserving glow from a pyramid of blurs, with threshold, falloff, aspect, tint, chromatic aberration and views |
| [Plasma Beam](plasma-beam) | Effect | A white-hot core with layered coloured glow between two points or along the layer's masks, with presets, taper, trim, flicker and distortion |
| [Forge 3D](forge-3d) | Effect | Lit, depth-correct 3D models over a layer, drawn by GPU draw passes: a cube, sphere, torus, extruded star or the bundled OBJ model, a grid or ring replicator with a colour ramp, a material, and the comp's camera and lights |
| [Glide](glide) | Panel and commands | A cubic-bezier easing editor with a preset library that applies the curve to the selected keyframes in one undo step |

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

Forge 3D draws only on the GPU: with GPU Acceleration off it leaves the layer as it is. Files in a plugin's
`assets/` folder are copied into its bundle; Forge 3D reads `assets/gem.obj` from there. That model is our
own work, an icosahedron with a raised point on each face, written out by a script for this plugin.

To write your own, read [Write a plugin](../docs/plugin-sdk.md).
