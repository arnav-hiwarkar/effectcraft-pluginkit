# Plugin reference

Types and calls of the `craft-plugin` crate, the manifest, and the app commands for plugins. To learn the
workflow, read [Write a plugin](plugin-sdk.md).

## plugin.json

| Field | Meaning |
|---|---|
| `id` | Reverse-domain, lower case, at least two segments. Effect ids become `<id>.<effect>`. |
| `name`, `version` | Shown in the plugin manager. Required. |
| `author`, `description` | Optional. |
| `entry` | `{"wasm": "plugin.wasm"}`, `{"native": {"macos-aarch64": "libx.dylib", "linux-x86_64": "libx.so", "windows-x86_64": "x.dll"}}`, or both keys together. File names only; they live in the plugin folder. A plugin with `wasm` runs sandboxed unless the user chooses Run Unsandboxed and the plugin has a native build for this machine. |
| `permissions` | Any of `document.read`, `document.write`, `files.read`, `files.write`. |

## Plugin

`Plugin::new()` starts an empty plugin. `craft_plugin::export!(function)` exports the plugin that `function`
returns.

| Method | Adds |
|---|---|
| `.effect(value)` | An effect. `value` implements `Effect`. |
| `.panel(id, title, value)` | A panel. `value` implements `Panel`. |
| `.command(id, title, function)` | A command. `function` is `Fn() -> Result<()>`. |

## Effect

```rust
pub trait Effect: Send + Sync + 'static {
    fn spec(&self) -> EffectSpec;
    fn layout(&self, cx: &Cx) -> Layout;                       // default: Layout::whole_frame()
    fn render(&self, cx: &Cx, frame: &mut Frame) -> Result<()>; // default: leaves the frame unchanged
    fn controls(&self, param: &str, value: &mut String, ui: &mut Ui); // default: draws nothing
}
```

`render` takes `&self` and can run on several threads at once. A sandboxed plugin runs each call in a
pooled instance, so values stored outside `cx` are not shared between calls.

### EffectSpec

`EffectSpec::new(id, name, category)`. The effect's full id is `<plugin id>.<id>`.

| Method | Meaning |
|---|---|
| `.param(Param)` | Adds a parameter. |
| `.group(name, [Param, …])` | Adds parameters under a twirl-down. Their ids become `name/<id>`. |
| `.time_dependent()` | The output changes with `cx.time` when no parameter is animated. |
| `.reads_other_frames()` | The effect calls `cx.source_at` or `cx.layer_at`. |
| `.reads_scene()` | The effect calls `cx.scene`. |
| `.shader(wgsl)` | WGSL kernels for GPU passes. See [GPU effects](plugin-gpu.md). |
| `.designer(panel_id)` | Effect Controls shows a button that opens this panel. |

The render cache uses the three declarations. An effect that omits one shows stale frames.

### Parameters

| Builder | Read in `render` with | Value |
|---|---|---|
| `Param::slider(id, name, default, min..=max)` | `cx.f(id)` | number |
| `Param::percent(id, name, default)` | `cx.f(id)` | 0 to 100 |
| `Param::angle(id, name, default)` | `cx.f(id)` | degrees |
| `Param::checkbox(id, name, default)` | `cx.bool(id)` | |
| `Param::popup(id, name, &["A", "B"], default)` | `cx.index(id)` | option index |
| `Param::point(id, name, [0.5, 0.5])` | `cx.point(id)` | frame pixels; the default is a fraction of the layer |
| `Param::point3(id, name, [0.5, 0.5, 0.0])` | `cx.point3(id)` | frame pixels |
| `Param::color(id, name, [r, g, b, a])` | `cx.color(id)` | straight RGBA, 0 to 1 |
| `Param::text(id, name, default)` | `cx.text(id)` | |
| `Param::custom(id, name, default)` | `cx.text(id)` | text the plugin owns; `controls` draws its row |
| `Param::layer(id, name)` | `cx.layer(id)` | another layer's pixels |
| `Param::mask(id, name)` | `cx.index(id)` | index into `cx.masks()` |

### Cx

`Cx` dereferences to these fields.

| Field | Meaning |
|---|---|
| `cx.size` | Full frame size in pixels. |
| `cx.scale` | Frame pixels per layer pixel (0.5 at half resolution). |
| `cx.offset` | Frame pixel where layer pixel (0, 0) sits. Padding moves it. |
| `cx.layer_size` | Layer size in layer pixels. |
| `cx.time`, `cx.comp_time`, `cx.frame_rate` | Seconds and frames per second. |
| `cx.instance` | Stable per effect instance. Use it as a random seed. |
| `cx.float` | The project is 32 bpc. When false, the app rounds the result to 8 or 16 bpc. |
| `cx.linear` | The working colour space is linear. When false, pixel values are display-encoded. |

`cx.px(id)` is `cx.f(id) * cx.scale`. In `layout`, `cx` describes the frame before padding.
`cx.padded(n)` is the context `render` gets after `n` pixels of padding.

| Call | Returns |
|---|---|
| `cx.layer("matte")` | The layer chosen in a layer parameter, resampled onto this frame's grid. |
| `cx.layer_at("matte", comp_time)` | The same at another time. |
| `cx.source_at(layer_time)` | This effect's input at another time. Declare `.reads_other_frames()`. |
| `cx.masks()` | The layer's masks as polylines in frame pixels. |
| `cx.scene()` | The comp's camera and lights, see [Scene](#scene). Declare `.reads_scene()`. |

These calls also work in `layout`, for example to size the padding from the masks.

`cx.layer`, `cx.layer_at`, and `cx.source_at` return `Option<Image>`. `Image` has `width`, `height`,
`pixels()`, `at(x, y)`, and bilinear `sample(x, y)`.

### Scene

`cx.scene()` returns `None` when the app cannot tell, for example in a render with no comp. Every
position and direction is in the layer's own pixels: x to the right, y down, z away from the viewer,
with the layer on the plane z = 0. They do not change with `cx.scale` or padding.

| Field or call | Meaning |
|---|---|
| `scene.projection` | Rows X, Y and W of the camera: a layer point `[x, y, z, 1]` shows at layer pixel `X / W`, `Y / W`. |
| `scene.project(point)` | That layer pixel, or `None` behind the camera. |
| `scene.view_projection(cx)` | The same camera as a column-major 4x4 clip matrix for a draw pass on a frame of `cx.size`. It lands a point on the pixel `project` gives, times `cx.scale`, plus `cx.offset`. Depth is 1 at `Scene::NEAR` (one layer pixel from the camera) and falls towards 0 with distance, which is what the depth buffer expects. |
| `scene.camera_layer` | The camera is a camera layer, not the comp's default view. |
| `scene.camera` | `view` (rows of the layer-to-camera matrix; camera space has z along the view), `position`, and `zoom` (the distance in pixels at which one unit covers one pixel). The app has no near or far distance. |
| `scene.lights` | Every light layer that is switched on. |
| `Scene::centred(cx.layer_size)` | A camera facing the middle of the layer that leaves the plane z = 0 where it is. Use it when `cx.scene()` is `None`. |

A `Light` has `kind` (`Ambient`, `Point`, `Spot`, `Parallel`), `position`, `direction` (the way the
light travels, unit length), `colour`, `intensity` (1 is 100 %), `cone_angle` (the full angle in
degrees) and `feather` (0 to 1). Light a surface with `colour` times `intensity`.

### Frame

| Field | Meaning |
|---|---|
| `frame.pixels` | Premultiplied RGBA `f32`, row 0 at the top. Values above 1 are valid. |
| `frame.width`, `frame.height` | Size of this call's pixels. |
| `frame.origin` | Position of this call's first pixel in the full frame. Not `[0, 0]` in a band. |

`frame.rows_mut()` yields `(y, row)` with `y` measured in the full frame. `frame.at(x, y)` takes
coordinates within this call's pixels, so subtract `frame.origin` from a full-frame position first.

### Layout

| Constructor | Meaning |
|---|---|
| `Layout::whole_frame()` | One call gets the whole frame. |
| `Layout::pointwise()` | Each output pixel depends only on the same input pixel. |
| `Layout::neighbourhood(n)` | Each output row depends on input rows at most `n` away. |
| `.padded(n)` | Grow the frame by `n` pixels on each side first. The app caps `n` at 4096. |
| `.gpu(plan)` | Run these GPU passes when the GPU renderer is active. |

## Panel

```rust
pub trait Panel: Send + 'static {
    fn ui(&mut self, ui: &mut Ui);
}
```

Panels and commands of one plugin run in the same long-lived instance, so a `static` is shared between
them. Effects do not see it: they run in pooled instances.

Widgets: `label`, `heading`, `weak`, `mono`, `error`, `button`, `button_enabled`, `checkbox`, `slider`, `number`,
`text_edit`, `popup`, `color`, `separator`, `space`, and the containers `row`, `column` and `group`.
Value widgets take `&mut` state and return `true` on the frame the user changed it.

| Widget | Shows |
|---|---|
| `ui.image(key, [w, h])` | An uploaded image. Returns `true` when clicked. |
| `ui.scroll(key, height, \|ui\| …)` | Its contents in a vertical scroll area at most `height` tall. |
| `ui.collapsing(title, open_by_default, \|ui\| …)` | A section with a twirl-down. |
| `ui.tabs(&mut selected, &["A", "B"])` | A row of tabs. Returns `true` when another is picked. |
| `ui.progress(fraction, text)` | A progress bar, `fraction` from 0 to 1. |
| `ui.tooltip(text)` | A tooltip on the widget just before it. |
| `ui.grid(columns, \|ui\| …)` | Its contents in rows of `columns` cells, such as thumbnails. |

`ui.keys()` is the text typed while the pointer is over the panel and no text field has the keyboard.

`ui.canvas(key, [width, height], |c| …)` gives a drawing surface with `line`, `bezier`, `rect`, `circle`,
`polygon` (convex, filled), `image` and `text`, in canvas-local pixels. `c.pointer` is the pointer over
the canvas this frame, with a phase of `Hover`, `Down`, `Drag` or `Up`, the `modifiers` held (`shift`,
`alt`, `command`) and the `scroll` delta. A drag keeps reporting positions after the pointer leaves the
canvas. Colours are theme tokens (`Paint::Text`, `Paint::Accent`, `Paint::Weak`, `Paint::Surface`, …)
or `Paint::Rgba`.

Two buttons with the same label in the same container are told apart by order. Put widgets that come
and go inside their own `row`, `column` or `group` so the others keep their identity.

## Host calls

All are in `craft_plugin::host`.

| Call | Permission |
|---|---|
| `command(id, params)` for a command that only reads | `document.read` |
| `command(id, params)` for a command that edits the project | `document.write` |
| `command(…)` that names a file, such as `file.saveCopy` | `files.read` and `files.write`, as well |
| `read_file(Root::Bundle, path)`, `list_files(Root::Bundle, path)` | None |
| `read_file`, `write_file`, `list_files` with `Root::Data` | None |
| `read_file(Root::User, path)`, `list_files(Root::User, path)` | `files.read` |
| `write_file(Root::User, path, bytes)` | `files.write` |
| `log(level, message)`, `info(message)` | None |
| `notify(message)`: shows a short message to the user | None |
| `upload_image(key, width, height, rgba8)`: stores an image for `ui.image` and `c.image` | None |

A plugin may run the document commands (`layer.*`, `comp.*`, `prop.*`, `keys.*`, `effect.*`, `mask.*`,
`text.*`, `shape.*`, and the other families in `DOCUMENT_FAMILIES` in `crates/plugin/src/gate.rs`) and the
commands that name a file. Every other command is refused for every plugin: scripts, settings, windows,
help, render jobs, and plugin management. A refused or denied call returns an `Err` with the reason.

## Limits

| Limit | Value |
|---|---|
| `layout`, `panel` and `controls` call | 2 s |
| Uploaded images of one plugin | 256 MiB in all, 8192 pixels on a side |
| `render` and `command` call | 60 s |
| Memory of one sandboxed instance | 3 GiB |
| Padding | 4096 pixels |

A sandboxed plugin that passes a limit or panics is stopped. The next call starts a new instance, and
panel state is lost.

## App commands

| Command | Parameters | Does |
|---|---|---|
| `plugin.list` | | Every plugin with its state, permissions, effects, panels, and commands |
| `plugin.install` | `path` | Installs a `.craftplugin` file or a bundle folder into the plugin folder. The plugin waits for approval. |
| `plugin.approve` | `id` | Grants the permissions the plugin asks for and starts it |
| `plugin.setSandbox` | `id`, `sandboxed` | For a plugin with a WebAssembly and a native build: `false` runs the native build for this version, `true` runs the sandboxed one |
| `plugin.disable` | `id` | Stops the plugin and keeps it installed |
| `plugin.uninstall` | `id` | Removes the plugin and its approval |
| `plugin.load` | `path` | Runs a bundle until the app quits, with the permissions it asks for |
| `plugin.reload` | | Stops and restarts every plugin |
| `plugin.run` | `plugin`, `command` | Runs a plugin command |
| `window.plugins` | | Opens the plugin manager |
| `window.pluginPanel` | `plugin`, `panel` | Opens a plugin panel |

An automation agent confined to folders cannot run any of these except `plugin.list`.

Installed plugins live in `Plugins/` in the settings folder. `Plugins/state.json` records each approval.
`Plugins/.data/<id>/` is a plugin's private data folder.
