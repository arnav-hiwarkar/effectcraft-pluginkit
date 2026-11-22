# What kinds of plugins can be built

This page lists the kinds of After Effects plugins and says how each is built with the plugin kit.
To write a plugin, read [Write a plugin](plugin-sdk.md). For the design, read
[About the plugin system](plugin-system.md).

## By kind of plugin

| Kind | Well-known examples | Built from | Reference plugin |
|---|---|---|---|
| Image effect | Deep Glow, Magic Bullet Looks, Sapphire | An effect with a CPU render, GPU compute passes, or both | `plugins/lumen-glow` |
| Effect that follows masks or paths | Saber, 3D Stroke | An effect that reads `cx.masks()` | `plugins/plasma-beam` |
| Lens and light effect | Optical Flares, Knoll Light Factory | A pointwise effect with a layer parameter for occlusion | `plugins/halo` |
| Stylised look | CRT and neon looks | Passes that run on the GPU or on all CPU cores | `plugins/retro` |
| Effect with its own control | Curves, gradient ramps, colour wheels | `Param::custom` and `Effect::controls` | `plugins/curves` |
| Particle system | Particular, Stardust | A deterministic simulation with checkpoints in the plugin cache | `plugins/ember` |
| 3D object renderer | Element 3D, Helium | GPU draw passes with vertex buffers, depth, the comp camera and lights | `plugins/forge-3d` |
| Time effect | Twixtor, ReelSmart Motion Blur, Echo | An effect that declares `.reads_other_frames()` and calls `cx.source_at` | |
| Tracker, stabiliser, rotoscoping tool | Mocha, Lockdown | A panel, a task that reads frames with `host::frame`, and commands that write keyframes | |
| Audio tool | Sound Keys, audio spectrum | A panel and a task that reads samples with `host::audio` | `plugins/pulse` |
| Animation and workflow tool | Flow, Duik, Motion Tools, Overlord | A panel and commands that run app commands | `plugins/glide` |
| Preset or asset browser | Animation Composer, Motion Bro | A panel with images, a scroll area, and files shipped in the package | `plugins/curves` (Curve Presets panel) |
| Exporter | Bodymovin, AEJuice Export | A command or task that reads the project and frames and writes files the user picked | |
| Batch or render tool | Render queue helpers | A task with progress and cancel | |

## By capability

| A plugin needs to | It uses | Permission |
|---|---|---|
| Change pixels on the CPU | `Effect::render` | None |
| Change pixels on the GPU | `Layout::gpu` with compute kernels | None |
| Use every CPU core for multi-step work | `Layout::passes` and `Effect::pass` | None |
| Sample another layer on the GPU | `plan.layer(param)` | None |
| Draw 3D geometry | `GpuPass::draw` with buffers from the cache | None |
| Match the comp's 3D space | `cx.scene()`: camera matrices and lights | None |
| Keep data between frames | `cx.remember` and `cx.recall` | None |
| Read its parameters at another time | `cx.at(time)` | None |
| Offer presets | `EffectSpec::preset`, applied with `effect.applyPreset` | None |
| Read another layer, another time, or the masks | `cx.layer`, `cx.source_at`, `cx.masks` | None |
| Draw its own control in Effect Controls | `Param::custom` and `Effect::controls` | None |
| Show a panel | `Plugin::panel` | None |
| Show images in a panel | `host::upload_image` and `ui.image` | None |
| Read the project | `host::command` with a reading command | `document.read` |
| Change the project | `host::command`, grouped with `engine.batch` | `document.write` |
| Run for a long time | `Plugin::task`, `host::start`, `task.progress` | None |
| Get a rendered frame | `host::frame` | `document.read` |
| Read audio | `host::audio` | `document.read` |
| Read or write a file the user picks | `host::pick_file` | None |
| Read or write any file | `host::read_file(Root::User, ...)` | `files.read`, `files.write` |
| Ship data with the plugin | Files in the package, read with `Root::Bundle` | None |
| Use a vendor library, a licence server, or the network | A native plugin | The user approves each version |

## What still needs a native plugin or is not possible

- **Network access from the sandbox.** A sandboxed plugin cannot open a connection. A native plugin can.
- **New footage file formats.** A plugin cannot register a decoder. It can convert a file and import the
  result with the app's own import command.
- **Audio effects.** A panel, command or task can read audio. An effect's render cannot, and no plugin
  can change what the layer plays.
- **Replacing the comp renderer.** A plugin cannot take over how 3D layers intersect or cast shadows.
  It draws into its own layer.
