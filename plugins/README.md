# Plugins

Each folder here is one plugin: a standalone Rust crate outside the workspace, built against the SDK in
`crates/plugin-sdk`. They are the reference for what a plugin can do.

| Plugin | Kind | What it does |
|---|---|---|
| [Lumen Glow](lumen-glow) | Effect | A wide, energy-conserving glow from a pyramid of blurs, with threshold, falloff, aspect, tint, chromatic aberration and views |
| [Ember](ember) | Effect | A deterministic 3D particle system: point, box, sphere or mask emitters, gravity, wind, drag, curl-noise turbulence and spin, colour, size and opacity over life, discs, streaks and sparks, motion blur and the comp camera |
| [Plasma Beam](plasma-beam) | Effect | A white-hot core with layered coloured glow between two points or along the layer's masks, with presets, taper, trim, flicker and distortion |
| [Halo Flares](halo) | Effect | A lens flare from a light point: core glow, star rays, anamorphic streak, chromatic ring, iris ghosts along the line through the centre, lens-dirt bloom and a hoop, with six looks, edge trigger, shimmer, occlusion by another layer and Add or Screen compositing |
| [Forge 3D](forge-3d) | Effect | Lit, depth-correct 3D models over a layer, drawn by GPU draw passes: a cube, sphere, torus, extruded star or the bundled OBJ model, a grid or ring replicator with a colour ramp, a material, and the comp's camera and lights |
| [Curves](curves) | Effect and panel | Tone curves for RGB, red, green, blue and alpha drawn as a graph in Effect Controls, and a Curve Presets panel of thumbnails |
| [Retro](retro) | Effects | CRT Display (curvature, aperture grille, slot or dot mask, rolling scanlines, convergence error, phosphor glow, noise and flicker) and Neon Outline (luma or alpha edges with a three-radius bloom), each with matching GPU kernels and a banded CPU render; `retro/bench.sh` times both |
| [Glide](glide) | Panel and commands | A cubic-bezier easing editor with a preset library that applies the curve to the selected keyframes in one undo step |
| [Cookbook](cookbook) | Effects, panels, a command, a task and a tool | Every recipe of [the cookbook](../docs/plugin-cookbook.md), each with its test; the page's code is checked against it |
| [Pulse](pulse) | Panel and task | Analyses a frequency band of a layer's audio in a cancellable background task and writes its energy per frame as slider keyframes in one undo step |

## Pulse

Open **Window ▸ Plug-ins ▸ Pulse**, type the audio layer's name (or leave it empty for the comp mix),
choose a band and press **Analyse**. A task reads the audio in 2 s chunks at 48 kHz, takes a 2048-point
FFT with a Hann window centred on each comp frame, and sums the band's energy. The panel shows
progress and a Cancel button, then draws the average spectrum and the resulting curve. The values,
scaled so the loudest frame is 100, go onto a Slider Control of a new null named `Pulse <low>-<high>
Hz`, as one keyframe per frame in one `engine.batch`: one undo removes them. `plugins/pulse/verify.sh`
drives `effectcraft-cli` over a 60 s tone whose level rises linearly and checks the curve, the undo and
cancel.

## Lumen Glow

One plan drives both renderers: `src/gpu.rs` builds the passes (emit, downsample, one blur-and-sum pass
per pyramid level, composite), `src/glow.wgsl` holds their GPU kernels and `src/passes/` the same kernels
for the CPU, which the app runs in bands on all cores.

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

To write a plugin, run `cargo xtask plugin new <name> [--kind effect|gpu-effect|panel|tool|task]`, then
`cargo xtask plugin dev plugins/<name>` with the app running (`cargo run -p effectcraft -- --control 9877`); the
[quickstart](../docs/plugin-quickstart.md) goes step by step. `plugins/new-plugin-check.sh` generates each kind of new
plugin, runs its tests and builds it for WebAssembly, then runs the Cookbook's tests.

`plugins/e2e.sh` builds every plugin here, installs Plasma Beam from its `.craftplugin` file, loads the other
bundles in `effectcraft-cli`, renders a frame with each effect and runs `glide/verify.sh`.

Forge 3D draws only on the GPU: with GPU Acceleration off it leaves the layer as it is. Files in a plugin's
`assets/` folder are copied into its bundle; Forge 3D reads `assets/gem.obj` from there. That model is our
own work, an icosahedron with a raised point on each face, written out by a script for this plugin.

To write your own, read [Write a plugin](../docs/plugin-sdk.md).

## Ember

Ember is a simulation. Frame N is stepped from frame N-1 with a fixed time step (two substeps per frame) and random
numbers seeded from the emitter seed, the substep and the particle index, so a frame never depends on render order.
Every tenth frame, and the last frame stepped, are stored in the plugin cache under a hash of every parameter the
simulation reads; a render steps forward from the nearest stored frame, and the cache is only a memo.
`layout()` steps the simulation once per frame, builds the screen-space draw list and stores it with row bins of
16 rows; each band of the parallel, pointwise render recalls the list and draws only the sprites in its bins.
`layout()` stops after 12 million particle-substeps and stores where it got to; the render finishes the catch-up.

```sh
cargo test --manifest-path plugins/ember/Cargo.toml --release            # unit tests and the host determinism tests
cargo run --manifest-path plugins/ember/Cargo.toml --release --example showcase -- <out-dir>   # camera PNG and timings
```

Keyframed parameters are read at the current frame only, so the simulation uses today's values for its whole history;
emitter motion is the constant `Emitter Motion` velocity, which `Inherit Emitter Motion` passes to new particles.

## Curves

The Curves effect keeps its five curves (RGB, Red, Green, Blue, Alpha) in one custom parameter as JSON.
Its control in Effect Controls is a graph: click to add a point, drag to move it, drag it off the graph or
Alt-click it to remove it. The channel popup picks the curve and Reset straightens it. The curves are
monotone cubics, sampled into a 1024-entry table per channel, applied to straight colour with
`Layout::pointwise()`. Values above 1 and below 0 continue the end segments in a straight line.

The Curve Presets panel shows a scrollable grid of preset thumbnails. A click sets the Curves effect of
the selected layer, and adds one first if the layer has none.

`curves/verify.sh` builds the bundle, applies the effect in `effectcraft-cli` and checks rendered pixels.
## Retro

`plugins/retro` holds two effects in one plugin, both under Stylize.

- **CRT Display** bends the picture into a curved screen with rounded black corners, then applies an aperture
  grille, slot mask or dot mask at a pitch in layer pixels, scanlines that roll slowly over time, a
  horizontal red and blue convergence error, a phosphor glow of the bright parts, a vignette, animated noise
  and flicker, and a brightness gain that makes up for the light the mask and scanlines remove.
- **Neon Outline** finds edges on luma or alpha, shapes them with a threshold, softness and thickness,
  colours the line core, and adds a bloom made from three blur radii summed with a falloff. It draws over
  the source or, with Lines Only, on its own.

Both do their wide blurs at a half, quarter or eighth of the resolution and declare
`Layout::neighbourhood(reach)`, so the app renders the CPU path in bands on all cores when the reach fits a
band. The GPU path is four compute passes per effect. `plugins/retro/check` tests the two paths against
each other through the real host (`cargo test --manifest-path plugins/retro/check/Cargo.toml`), and
`plugins/retro/bench.sh` prints the CPU sandboxed, CPU native and GPU times at 1920×1080.
