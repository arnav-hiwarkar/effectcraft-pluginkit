# About the plugin system

A plugin adds effects, panels, and commands to EffectCraft. This page explains how the system is
built and why. To write a plugin, read [Write a plugin](plugin-sdk.md). For the exact API, read the
[plugin reference](plugin-reference.md).

## What a plugin can add

| Contribution | Where it shows up | What it is built from |
|---|---|---|
| Effect | Effects & Presets, Effect Controls, the timeline, saved projects | Parameters the app draws and keyframes, plus a CPU render function, GPU passes, or both |
| Panel | A dockable tab, listed in **Window ▸ Plug-ins** | A function the app calls when the panel's widgets, app events or timers ask for a new drawing |
| Command | **Window ▸ Plug-ins** | A function that runs app commands |
| Task | The plugin manager, while it runs | A function that runs in the background with progress and cancel |
| Viewer tool | The toolbar, and the Composition viewer while it is picked | A function that gets the viewer's pointer and key events and returns what to draw |
| On-viewer controls | The Composition viewer, while the effect is selected | Marks and handles an effect returns; the app draws them and turns a drag into a parameter change |

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
  `command`, `task`, `overlay`, `drag`, or `tool`.
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
| `clipboard` | Reading and replacing the text on the system clipboard |

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
| Native, CPU | One copy of each band into the new frame, which replaces the old one when every band succeeded | Trusted plugins |
| Sandboxed, CPU | One copy of the frame in and one out | Everything else |

Measured on a ten-core Apple M-series laptop that was also running other builds, 1920×1080 `f32` RGBA,
median of 15, with `cargo run --release -p craft-plugin-host --example bench`:

| Effect | Native | Sandboxed | Sandboxed before bands and parallel copies |
|---|---|---|---|
| One-line effect | 0.5 ms | 1.7 ms | 3.6 ms |
| Plasma Beam | 2.0 ms | 5.7 ms | 15.1 ms |
| Lumen Glow, CPU passes, padded frame | 14.3 ms | 32.9 ms | 85 ms |

A sandboxed effect still takes two to three and a half times as long as the native build of the same
source. Both now use every core. What remains is the copy of each band into and out of the instance, and
slower code for `powf`, `exp` and similar maths, which a WebAssembly build has to bundle. A compiled
module is cached on disk, so loading a sandboxed plugin takes a few milliseconds after its first run.

A sandboxed instance is single-threaded. To use every core, an effect tells the app how far each output
row reads from its input row. The app then splits the frame into bands and renders them on separate
instances. An effect that needs the whole frame at once, such as a wide glow, describes its work as
passes ([Passes](plugin-gpu.md)): the app owns the images and runs each pass on the GPU, or on the CPU in
bands on all cores, copying into each instance only the source rows its band reads. Lumen Glow at its
default settings on a 1920×1080 layer (the padded frame is larger), sandboxed, on a shared ten-core
machine, fastest of 15: 85 ms as one whole-frame call, 29 ms as banded passes; native 49 ms and 13 ms.

A render call has rows to write and rows to read, and they are separate: the input is never the
output, so a band writes only its own rows and nothing is rendered twice. The app asks an effect only
for the part of the layer that the frame being rendered shows (`shown` on the effect host, set by the
CPU compositor in `crates/render/src/pipeline.rs` for a top-level 2D layer whose stack has a plugin
effect), and the layer's cache key then includes that rectangle. A native band reads the input frame
and writes the output frame where they lie; a sandboxed band copies its input rows in and its own rows
out.

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

A panel describes its widgets across the sandbox boundary, and the app keeps the last description. The
app draws it with egui every frame and calls the plugin again only when something happened: an event from
one of the panel's widgets, an app event the panel subscribed to (selection, current time, active comp,
project, edits), a timer, or a new size. The state machine for one panel, `craft_plugin_host::Retained`,
holds no EffectCraft types: another Craft app reuses it with its own event source. Custom Effect Controls
rows are retained the same way and are called again on their own widget events, a new value or a new
width.

### One queue per plugin

Every interactive call (a panel, a custom control, a viewer overlay, a custom handle drag, a tool event,
a command chosen from the menu) goes to one worker thread per loaded plugin, in order, and only that
thread calls the plugin's long-lived instance for them (`effectcraft_plugin::Panels`). The UI thread posts
requests, answers the host calls that need the project, and applies replies in one place; it waits at
most 4 ms a frame for replies while it answers, never on the instance and never on a menu command, so a
slow plugin cannot stall or deadlock the app. A frame or audio a panel asks for is cut from the project on
the UI thread and rendered on the worker. A synchronous `plugin.run` (CLI, scripts) runs the command on a
thread of its own and answers its host calls until it returns.

Each request carries what it is for: the plugin's load, the project's token (`Session::project_closed`)
and a token of the surface that asked (a panel, an overlay, a tool activation). One check runs before a
request reaches the worker, before the worker calls the plugin, before a host call is answered or its
answer, frame or audio reaches the plugin, and before a reply is applied. A request whose plugin was
reloaded or turned off, whose project closed or whose surface closed is dropped with a log line; a worker
waiting for the app on such a request is released with an error. Each surface has one call in flight at a
time. A click, a checkbox toggle or a drop reaches exactly one call, so two clicks during a slow call are
two calls; pointer motion and slider values made during a call coalesce, so the backlog stays bounded. A
failed call consumes its input, and a reply that arrives after its panel was hidden is dropped and the
panel is called again when it shows. A custom control's or a custom handle's new value is set at the time
and with the undo key it was made with. A tool's project edits join the gesture's undo step as they
happen, only while nothing else changed the project in between.

A plugin has no access to egui or to the app's fonts and colours. It names theme colours, and the app
resolves them.

## What plugins cannot do yet

- Open network connections from the sandbox. A native plugin can.
- Read audio from inside an effect's render (`cx.audio`). A panel, command or task can read it. The
  render path reaches only footage items' audio (`EffectHost::audio`), not the comp mixer.
- Change the audio a layer plays.
- Register a decoder for a new footage format. A task with `pick_file` and `frame` can write an export
  itself, and a command can convert a file and import the result.
- Take over how the comp's 3D layers intersect or cast shadows. A plugin draws 3D into its own layer.

[What kinds of plugins can be built](plugin-capabilities.md) maps each kind of plugin to the calls it uses.
