# About the plugin system

A plugin adds effects, panels, and commands to EffectCraft. This page explains how the system is
built and why. To write a plugin, read [Write a plugin](plugin-sdk.md). For the exact API, read the
[plugin reference](plugin-reference.md).

## What a plugin can add

| Contribution | Where it shows up | What it is built from |
|---|---|---|
| Effect | Effects & Presets, Effect Controls, the timeline, saved projects | Parameters the app draws and keyframes, plus a CPU render function, GPU passes, or both |
| Panel | A dockable tab, listed in **Window ▸ Plug-ins** | A function the app calls every frame the tab is visible |
| Command | **Window ▸ Plug-ins** | A function that runs app commands |
| Task | The plugin manager, while it runs | A function that runs in the background with progress and cancel |

A plugin effect is registered in the same registry as the built-in effects. Nothing downstream can tell
the two apart: Effect Controls, keyframes, expressions, presets, the render cache, and project files all
work from the registry entry.

## Three crates

| Crate | Path | Job |
|---|---|---|
| `craft-plugin` | `crates/plugin-sdk` | The SDK a plugin author depends on. Also holds the manifest and message types the host reads. |
| `craft-plugin-host` | `crates/plugin-host` | Loads plugins, runs them, checks permissions, and keeps the list of installed plugins. |
| `effectcraft-plugin` | `crates/plugin` | Connects the runtime to EffectCraft: effects, engine commands, and the file rules. |

The first two crates do not depend on EffectCraft. `cargo xtask layers` checks this. Another Craft app
adopts the system by writing its own version of the third crate, which is about 600 lines here.

## How the app and a plugin talk

There is one function in each direction.

- The app calls `craft_plugin_call` with a JSON request: `describe`, `layout`, `render`, `panel`,
  `command`, or `task`.
- The plugin calls `host_call` with a JSON request: run an app command, read a layer, read the masks,
  read or write a file, write a log line, start or ask about a task, render a frame, read audio, or
  open the file dialog.

A task runs on its own thread and instance. Its calls that need the document wait in a queue that the
app answers on the main thread once per UI frame (`effectcraft_plugin::pump`) and in the headless
`plugin.tasks` wait loop, so the document is never touched off the main thread. A sandboxed task is
cancelled through the same epoch clock that stops a call over its time limit.

Pixels never travel as JSON. A frame is a pointer and a length next to the request.

Both directions are Rust enums with one variant per request. Adding a capability means adding a
variant. An older plugin never sends the new variant, and an older app answers an unknown variant with
an error the plugin can handle, so the binary interface stays at version 1.

## Sandboxed and native plugins

The same plugin source builds two ways.

**Sandboxed** plugins are WebAssembly modules that run in Wasmtime inside the app process. A module
imports exactly one function, `host_call`. It has no file, network, clock, or process access of its own.
Each call has a time limit and each instance has a memory limit. A plugin that panics or exceeds a limit
is stopped, and the next call starts a fresh instance. The time limit counts the time the app spends
serving the plugin's host calls. The app cannot interrupt its own work, so a host call that is still
running at the limit, such as a read from a stalled disk, finishes first, and the plugin call then fails
as soon as that host call returns.

**Native** plugins are shared libraries loaded into the app. Their calls to the app go through the same
permission checks, but the code itself can do anything the app can. The manager shows a warning, and the
plugin does not load until the user accepts it. Native exists for work the sandbox cannot do: linking a
vendor library, running a licensing daemon, or driving a private GPU pipeline.

## Permissions and how they are enforced

A plugin lists the permissions it wants in `plugin.json`. The user sees that list before the plugin runs.
An update that asks for more waits for a new approval. Approval of a native plugin covers one version: every
update of a native plugin waits for a new approval.

| Permission | Allows |
|---|---|
| `document.read` | App commands that only read |
| `document.write` | App commands that change the project |
| `files.read` | Reading files outside the plugin's own folders |
| `files.write` | Writing files outside the plugin's own folders |

Every host call passes one check, `gate::check` in `crates/plugin-host/src/runtime/gate.rs`, before the
app sees it. For an app command, EffectCraft decides what the command needs in
`crates/plugin/src/gate.rs`:

1. **Is the command one a plugin may run at all?** A plugin may run the commands of the document
   families (`layer`, `comp`, `prop`, `keys`, `effect`, `mask`, `text`, `shape`, and the others in
   `DOCUMENT_FAMILIES`) and the File menu commands that name a file to read or write. Every other
   command is refused: scripts, settings, windows, help, render jobs, the clipboard, and plugin
   management. A plugin cannot make a command wait for a background job.
2. **Does the command change the project?** A command that is recorded in the undo history needs
   `document.write`. Any other command needs `document.read`.
3. **Does the command name a file?** It then needs both `files.read` and `files.write`. Commands such as
   Save and Import read and write more files than the one they name, so one permission alone would not
   describe what they do.
4. **Does the command reach outside the document?** `text.fonts` lists the user's installed fonts and
   needs `files.read`. `layer.setText` with a `font` saves the recent-fonts list to the settings file and
   needs `files.write`.

`census.tsv` next to `crates/plugin/src/gate.rs` records the decision for every command in the allowed
families, and a test fails when a new command appears there or a decision changes. Each new entry needs a
look at what the command touches besides the project.

Without both file permissions, a plugin also cannot write a text-valued property, because some effects
read such a property as a file path. The check looks the property up before the command runs, so in a
batch it cannot know what a later step will name. A batch therefore cannot write an effect property on a
layer the batch creates, or after any earlier step that changes the project other than a property write:
a step that adds, removes, duplicates or reorders effects or layers, or changes the selection, can change
which property `effects/#1/x` names. Such writes need both file permissions.

An `engine.batch` needs everything its steps need. After the permissions pass, the command is checked
by the automation confinement (`crates/automation/src/confine.rs`), the same rules that hold an AI agent
inside its folders: any path in the parameters must be inside the plugin's own folders unless the plugin
holds both file permissions.

Rendering needs no permission. An effect can read its own layer, a layer picked in a layer parameter,
the layer's masks, and the comp camera. With no network and no file access, it has nowhere to send them.

There is no network permission. A sandboxed plugin cannot open a connection.

## Rendering speed

| Path | Cost on top of the effect's own work | Use it for |
|---|---|---|
| GPU passes | None. The app runs the plugin's WGSL on its own textures. | Anything expressible as image passes |
| Native, CPU | None. The plugin writes into the app's buffer. | Trusted plugins |
| Sandboxed, CPU | One copy of the frame in and one out | Everything else |

Measured on an Apple M-series laptop with a 1920×1080 `f32` RGBA frame and a one-line effect: 1.1 ms
native, 3.6 ms sandboxed, 1.2 ms for the same loop in the app. The 2.5 ms gap is the two copies.
`cargo run --release -p craft-plugin-host --example bench` repeats the measurement after
`cargo test -p craft-plugin-host` has built the probe plugin.

A sandboxed instance is single-threaded. To use every core, an effect tells the app how far each output
row reads from its input row. The app then splits the frame into bands and renders them on separate
instances. An effect that needs the whole frame at once, such as a wide glow, describes its work as
passes ([Passes](plugin-gpu.md)): the app owns the images and runs each pass on the GPU, or on the CPU in
bands on all cores, copying into each instance only the source rows its band reads. Lumen Glow at its
default settings on a 1920×1080 layer (the padded frame is larger), sandboxed, on a shared ten-core
machine, fastest of 15: 85 ms as one whole-frame call, 29 ms as banded passes; native 49 ms and 13 ms.

Effects are stateless. The app keeps a pool of instances and may run any frame on any of them, in any
order. A panel or a command runs on one long-lived instance, so panel state persists between frames.

## Render cache

A plugin effect's cache key holds the plugin's revision: a hash of its version and module, and the
size, path and modification time of each file in the plugin's folder and its data folder (up to 256
files). The app looks at the folders at most twice a second. Reloading a changed plugin, or changing a
file the effect reads from those folders, gives every layer that runs the effect a new key. The same holds for a layer that samples such a layer through a
layer parameter, and for the comp's content key, which the thumbnails and the disk frame cache use. A
file outside those folders, read with `files.read`, is not part of the key.

A plugin's WGSL is compiled with wgpu's runtime checks on (`ShaderRuntimeChecks::checked`): naga bounds
every loop and checks every buffer and texture access. A bounded loop can still run for seconds. wgpu 30
has no per-dispatch time limit, so a kernel that runs too long is ended by the operating system's GPU
watchdog, if it has one, and a readback that waits longer than 10 s marks the app's GPU context failed
(`crates/gpu/src/readback.rs`). Until then the context is stalled for every effect, not only the plugin's.

## Panels

A panel is drawn in immediate mode across the sandbox boundary. Every frame the app sends the events
from the last frame and the plugin returns a list of widgets. The app draws them with egui and collects
the next events. A click therefore reaches the plugin one frame after it happens.

A plugin has no access to egui or to the app's fonts and colours. It names theme colours, and the app
resolves them.

## What plugins cannot do yet

- Open network connections from the sandbox.
- Draw custom controls inside Effect Controls. An effect can link a panel as its designer instead.
- Keep per-instance state between frames, such as a simulation cache.
- Read audio from inside an effect's render (`cx.audio`): a panel, command or task can read it. The
  render path reaches only footage items' audio (`EffectHost::audio`), not the comp mixer.
- Import or export file formats. A task with `pick_file` and `frame` can write an export itself.

Each of these is one more request variant and one more permission check, not a redesign.
