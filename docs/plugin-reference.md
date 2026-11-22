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
| `permissions` | Any of `document.read`, `document.write`, `files.read`, `files.write`, `clipboard`. |

## Plugin

`Plugin::new()` starts an empty plugin. `craft_plugin::export!(function)` exports the plugin that `function`
returns.

| Method | Adds |
|---|---|
| `.effect(value)` | An effect. `value` implements `Effect`. |
| `.panel(id, title, value)` | A panel. `value` implements `Panel`. |
| `.command(id, title, function)` | A command. `function` is `Fn() -> Result<()>`. |
| `.task(id, title, function)` | A long-running task. `function` is `Fn(&Task) -> Result<serde_json::Value>`. |
| `.tool(id, name, value)` | A viewer tool in the toolbar. `value` implements `Tool`. |

### Task

| Method | Returns |
|---|---|
| `task.args()` | The JSON the caller passed to `host::start` |
| `task.progress(fraction, message)` | `Err` once the user cancelled, so `?` ends the task |

A task runs on its own plugin instance on a background thread with no time limit. Its `command`,
`notify`, `frame` and `audio` calls are answered by the app's main thread, once per UI frame (in the
CLI: after each `run` step, while `render` waits, in the `plugin.tasks` wait loop, and before `exec` or
`run` saves and exits, which waits up to 600 s for running tasks); its log, file, cache and task calls are answered on its own
thread. A sandboxed task that ignores cancel is stopped 0.5 s after the cancel. A native task can only
be asked, through `progress`. A plugin runs at most 4 tasks at once. Disabling, reloading or removing a
plugin cancels its tasks. A task belongs to the project that was open when it started: when that project
closes (a new project or another one is opened) the task is cancelled, and a request it makes after that
fails with `the project this task started in was closed`, as does starting another task from it. Such a task ends as
cancelled even if it returns a result. The last 16 finished tasks of a plugin stay listed.

## Effect

```rust
pub trait Effect: Send + Sync + 'static {
    fn spec(&self) -> EffectSpec;
    fn layout(&self, cx: &Cx) -> Layout;                       // default: Layout::whole_frame()
    fn render(&self, cx: &Cx, frame: &mut Frame) -> Result<()>; // default: writes the input unchanged
    fn input(&self, cx: &Cx, out: Rect) -> Rect;                // default: out; asked only with Layout::maps_input
    fn controls(&self, instance: u64, param: &str, value: &mut String, ui: &mut Ui); // default: draws nothing
    fn pass(&self, cx: &Cx, pass: &Pass, out: &mut Frame) -> Result<()>; // default: an error naming the kernel
    fn overlay(&self, cx: &Cx, overlay: &mut Overlay);          // default: draws nothing
    fn drag(&self, cx: &Cx, key: &str, to: [f64; 2], value: &mut String); // default: leaves the value
}
```

`pass` computes one band of one pass of a plan on the CPU. `Pass` holds `kernel`, `f`, `u`, `src` and
`aux` (`Rows`: `width`, `height`, `first_row`, `pixels`, `at(x, y)`, `row(y)` in whole-image coordinates)
and `out_size`. See [Passes](plugin-gpu.md).

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
| `.reads_param_history()` | The effect calls `cx.at` or `cx.param_history`. A layer whose other effects or masks have expressions is then not cached, because the key cannot see what an expression returned at another time. |
| `.preset(name, [(id, Value), …])` | A named set of parameter values, see [Presets](#presets). |
| `.shader(wgsl)` | WGSL kernels for GPU passes. See [Passes](plugin-gpu.md). |
| `.designer(panel_id)` | Effect Controls shows a button that opens this panel. |

The render cache uses the four declarations. An effect that omits one shows stale frames. The
layer's cache key already holds every keyframe and expression of the effect, not only the values at
the current time (`hash_debug(&mut h, c)` in `crates/render/src/cache.rs`, `key_any`), so a keyframe
moved at frame 12 also redraws frame 40.

### Presets

`.preset("Sun", [("look", Value::Number(1.0)), ("Glow/brightness", Value::Number(120.0))])` names a
set of values. Ids are the full parameter ids, including the group prefix. Values have the form `cx`
reads them in, except points: a point is a fraction of the layer, like a parameter's default. Ids
the effect does not declare, and layer and mask parameters, are ignored.

A preset writes the absolute value of every parameter it names, so the controls show them, and leaves the
parameters it does not name as the user set them. A look that names `look` and the counts of its elements
(`("Rays/count", Value::Number(14.0))`) therefore does not reset the element controls.

The engine lists them with `effect.presets {layer?, effect}` → `{presets: [name]}` and applies one
with `effect.applyPreset {layer?, effect, preset}`. Applying sets every value at the current layer
time in one undo step: a keyframed parameter gets a keyframe, a static one changes its value.
Panels, commands, the CLI and automation all use these two commands.

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

A new version of a plugin may add parameters. Effects already applied in a project gain them with their
defaults when the project opens, or when the plugin starts while the project is open. The second is one undo step,
`Update Plug-in Effects`, and marks the project changed.

### Cx

`Cx` dereferences to these fields.

| Field | Meaning |
|---|---|
| `cx.size` | Full frame size in pixels. |
| `cx.scale` | Frame pixels per layer pixel (0.5 at half resolution). |
| `cx.offset` | Frame pixel where layer pixel (0, 0) sits. Padding moves it. |
| `cx.layer_size` | Layer size in layer pixels. |
| `cx.time`, `cx.comp_time`, `cx.frame_rate` | Seconds and frames per second. |
| `cx.instance` | Identifies this applied effect: different for every effect on every layer, and the same across frames, sessions and reopening the project. It is built from the layer id and the effect's property-group uid. `cx.recall` and `cx.remember` key on it, and it makes a good random seed. |
| `cx.float` | The project is 32 bpc. When false, the app rounds the result to 8 or 16 bpc. |
| `cx.linear` | The working colour space is linear. When false, pixel values are display-encoded. |

`cx.px(id)` is `cx.f(id) * cx.scale`. In `layout`, `cx` describes the frame before padding.
`cx.padded(n)` is the context `render` gets after `n` pixels of padding, `cx.padded_sides(left, top,
right, bottom)` after padding each side by its own amount.

| Call | Returns |
|---|---|
| `cx.layer("matte")` | The layer chosen in a layer parameter, resampled onto this frame's grid. |
| `cx.layer_at("matte", comp_time)` | The same at another time. |
| `cx.source_at(layer_time)` | This effect's input at another time. Declare `.reads_other_frames()`. |
| `cx.at(layer_time)` | The same `Cx` with every parameter evaluated at another layer time, keyframes and expressions applied as the app renders that time. `time` becomes `layer_time` and `comp_time` moves by the same amount; the frame fields stay. `None` outside the app. Declare `.reads_param_history()`. |
| `cx.param_history()` | A hash of everything that sets this effect's parameters over time (values, keyframes, expression text), the same in every frame and session. `None` when a parameter has an expression, whose values at other times the hash cannot see, or outside the app. Use it to key caches of a simulation. |
| `cx.masks()` | The layer's masks as polylines in frame pixels. |
| `cx.scene()` | The comp's camera and lights, see [Scene](#scene). Declare `.reads_scene()`. |

These calls also work in `layout`, for example to size the padding from the masks.

`cx.layer`, `cx.layer_at`, and `cx.source_at` return `Option<Image>`. `Image` has `width`, `height`,
`pixels()`, `at(x, y)`, and bilinear `sample(x, y)`.

The image is exactly `cx.size` and lies on the frame's own pixel grid: its pixel (x, y) is frame
pixel (x, y), at the same `cx.scale` and `cx.offset`. In `render` that is the padded frame; in
`layout` it is the frame before padding. Another layer is centred on this layer, not stretched, and
what lies outside it is transparent. `cx.layer` and `cx.layer_at` give the other layer as its layer
parameter's source choice says (by default with its masks and effects). `cx.source_at` gives this
layer at that layer time through its masks and the effects above this one, in the same space as the
frame `render` gets.

### Scene

`cx.scene()` returns `None` when the app cannot tell, for example in a render with no comp. Every
position and direction is in the layer's own pixels: x to the right, y down, z away from the viewer,
with the layer on the plane z = 0. They do not change with `cx.scale` or padding.

| Field or call | Meaning |
|---|---|
| `scene.projection` | Rows X, Y and W of a 3x4 matrix from a layer point `[x, y, z, 1]` (layer pixels, z away from the viewer) to where it shows on the layer: layer pixel `X / W`, `Y / W`. Turn that into a frame pixel with `* cx.scale + cx.offset`. |
| `scene.project(point)` | That layer pixel, or `None` behind the camera. |
| `scene.view_projection(cx)` | The same camera as a column-major 4x4 clip matrix for a draw pass on a frame of `cx.size`. It lands a point on the pixel `project` gives, times `cx.scale`, plus `cx.offset`. Depth is 1 at `Scene::NEAR` (one layer pixel from the camera) and falls towards 0 with distance, which is what the depth buffer expects. |
| `scene.camera_layer` | The camera is a camera layer, not the comp's default view. |
| `scene.camera` | `view` (rows of a 4x4 matrix from layer pixels to camera space, which has its origin at the camera, x right, y down and z along the view, in the same pixel units), `position` (the camera in layer pixels), and `zoom` (the distance in pixels at which one unit covers one pixel). The app has no near or far distance. |
| `scene.lights` | Every light layer that is switched on. |
| `Scene::centred(cx.layer_size)` | A camera facing the middle of the layer that leaves the plane z = 0 where it is. Use it when `cx.scene()` is `None`. |

A `Light` has `kind` (`Ambient`, `Point`, `Spot`, `Parallel`), `position`, `direction` (the way the
light travels, unit length), `colour`, `intensity` (1 is 100 %), `cone_angle` (the full angle in
degrees) and `feather` (0 to 1). Light a surface with `colour` times `intensity`.

### Frame

| Field | Meaning |
|---|---|
| `frame.pixels` | The pixels this call writes: premultiplied RGBA `f32`, row 0 at the top. Values above 1 are valid. Write every pixel; a pixel left unwritten is transparent. |
| `frame.width`, `frame.height` | Size of the rectangle this call writes. |
| `frame.origin` | Position of that rectangle in the full frame. Not `[0, 0]` in a band or when only part of the layer shows. |
| `frame.input` | The input this call may read (`Rows`: `width`, `height` of the full frame, `first_row`, `pixels`, `row(y)`, `at(x, y)`, in full-frame coordinates). Whole rows: the rows the call writes, plus `n` above and below for `Layout::neighbourhood(n)`, the rows `Effect::input` names with `maps_input`, every row for `Layout::whole_frame()`. |

| Method | Meaning |
|---|---|
| `frame.map(f)` | Writes each pixel as `f` of the input pixel under it. The pointwise case. |
| `frame.rows()` | `(y, input row, output row)`, `y` in the full frame, the input row cut to the columns written. |
| `frame.rows_mut()` | `(y, output row)`. |
| `frame.at(x, y)` | The input pixel at a full-frame position; transparent outside the input. |
| `Frame::whole(width, height, input, pixels)` | A whole-frame call, for unit tests. |

The app asks an effect only for the part of the layer that shows in the frame being rendered (the comp
frame, or the viewer's region when it is zoomed in), grown by what the resampling onto the comp and the
later effects in the stack need. Without padding, a layer wholly in the frame renders whole. With
`Layout::pointwise()`, `Layout::neighbourhood(n)` or `.maps_input()` that rectangle is split into bands of
rows on all cores, for sandboxed and native builds alike, and no row is written twice. A native plugin
reads the input where it lies and writes the output frame directly; a sandboxed one gets its input rows
copied in and its rows copied out. The rendered bands go into a new frame that replaces the layer's frame
only when every call succeeded, so an effect that fails, in any band, leaves the layer as it was.

A plugin built before frames had separate input (it sends no `separate_input` in its description) still
runs: each band gets whole rows with `n` rows above and below in `frame.pixels`, the app copies them for it
and keeps the band's own rows.

### Layout

| Constructor | Meaning |
|---|---|
| `Layout::whole_frame()` | One call gets the whole frame. |
| `Layout::pointwise()` | Each output pixel depends only on the same input pixel. |
| `Layout::neighbourhood(n)` | Each output row depends on input rows at most `n` away. |
| `.padded(n)` | Grow the frame by `n` pixels on each side first. The app caps each side at 4096. |
| `.padded_sides(left, top, right, bottom)` | Grow each side by its own amount. |
| `.maps_input()` | The app asks `Effect::input` which input rectangle an output rectangle needs, once per band. |

`Rect { left, top, right, bottom }` is in frame pixels, `right` and `bottom` exclusive, and may reach past
the frame; the app clips it. `Rect::of_size`, `grown(n)`, `clipped(size)`, `union`, `contains`, `width`,
`height` and `is_empty` help build one.
| `.passes(plan)` | Run these passes on the GPU when the GPU renderer is active, and otherwise on the CPU through `Effect::pass`, in bands on all cores. |
| `.gpu(plan)` | Run these passes when the GPU renderer is active; `render` is the CPU path. |

`GpuPass::reach(n)` gives a pass the number of source rows above and below the matching source row that
an output row reads. Without it the pass runs as one call on the CPU. `GpuPass::aux_reach(n)` sets the
reach of `aux` alone; without it `aux` uses the pass's reach.

### Host kernels

Passes the app computes itself, on the CPU natively and on the GPU with its own WGSL, within 2/255 of
each other. See [Host kernels](plugin-gpu.md#host-kernels) for what each computes.

| Builder | Fields and limits |
|---|---|
| `GpuPass::blur(src, out, radius)`, `GpuPass::box_blur(..)` | `HostKernel::Blur { shape: BlurShape::Gaussian | Box, radius: [f32; 2] }`, each axis 0 to `MAX_BLUR_RADIUS` (512). `src` and `out` the same size. |
| `GpuPass::resample(src, out, filter)` | `HostKernel::Resample { filter: Filter::Bilinear | Bicubic, scale: Option<[f32; 2]> }`. `scale` is source pixels per output pixel, above 0 and at most 64; `None` takes it from the sizes. |
| `GpuPass::blend_images(base, top, out, mode, amount)` | `HostKernel::Blend { mode: BlendMode, amount: f32 }`, `top` as `aux`. All three the same size; `amount` finite. |
| `GpuPass::curves(src, out, table)` | `HostKernel::Curves { table: Vec<Pixel> }`, 2 to `MAX_CURVE_POINTS` (4096) entries. |
| `GpuPass::convolve(src, out, taps)` | `HostKernel::Convolve { taps: Taps::Separable { x, y } | Taps::Full { width, weights } }`. Separable: odd counts up to `MAX_TAPS` (1025). Full: odd sides up to `MAX_FULL_SIDE` (31). |
| `GpuPass::warp(src, displacement, out, amount)` | `HostKernel::Warp { amount: [f32; 2] }`, displacement as `aux`, each axis at most `MAX_WARP` (4096) pixels. |
| `GpuPass::matrix(src, out, rows)` | `HostKernel::Matrix { rows: [[f32; 4]; 4] }`, finite. |
| `GpuPass::host(kernel, src, out)` | Any `HostKernel`. |

Every value must be finite. `kernel.check(src_size, aux_size, out_size)` is the check `plan.validate`
runs for one pass.

`plan.validate(frame)` checks a plan against the limits in [Passes](plugin-gpu.md#limits) for a padded
frame of that size and returns the first rule it breaks. The app runs this same check before it runs a
plan on the GPU or the CPU, and reports the reason as the effect's error. `plan.check(frame)` also
refuses a plan the CPU cannot run (one with draw passes, buffers, textures or layers).

## Fast maths

`craft_plugin::math` holds `#[inline(always)]` `f32` functions with no tables and no library calls. Sandboxed and
native plugins compute identical bits. Each error is the largest found by a test that sweeps the range against
`std` in `f64`.

| Function | Valid range | Largest error |
|---|---|---|
| `exp2(x)` | `-126..=126`, clamped | 3e-7 relative |
| `exp(x)` | `-87..=88`, clamped | 5e-7 relative |
| `log2(x)`, `ln(x)` | finite `x > 0`; zero, negative and subnormal give `-126` and `-87.3` | 1.1e-7 and 1.2e-7 times `max(1, abs(result))` |
| `pow(x, y)` | finite `x >= 0`, result in `1e-37..=1e37`; `pow(0, y)` is `0` for `y > 0`; a positive `x` below `1.2e-38` behaves as `1.2e-38` | 2e-5 relative |
| `sin(x)`, `cos(x)`, `sin_cos(x)` | `abs(x) <= 1000` | 2e-6 absolute (3e-7 within 20 radians) |
| `atan2(y, x)` | finite; the sign of a zero counts; `atan2(0, 0)` is `0`; a NaN argument gives a finite angle | 5e-6 absolute |
| `hypot(x, y)` | the larger of `abs(x)`, `abs(y)` is `0` or in `1e-19..=1e18` (below `1e-22` the result is `0`) | 1.5e-7 relative |
| `smoothstep(e0, e1, x)` | `e0 != e1` | exact |
| `lerp(a, b, t)` | any | exact |
| `saturate(x)` | any; NaN stays NaN | exact |
| `srgb_to_linear(v)`, `linear_to_srgb(v)` | any; negatives mirror around 0, values above 1 continue the curve; `srgb_to_linear` saturates at `8.5e37` past `6e15` | 3e-6 absolute over `-2..=2` (`srgb_to_linear` 3e-6 relative up to `1e15`) |

## Panel

```rust
pub trait Panel: Send + 'static {
    fn ui(&mut self, ui: &mut Ui);
}
```

Panels and commands of one plugin run in the same long-lived instance, so a `static` is shared between
them. Effects do not see it: they run in pooled instances.

The app keeps the widget tree a panel returned and draws it every frame. It calls `ui` again, on a worker
thread, only for: an event from one of the panel's widgets, an app event the panel subscribed to, a timer
of the panel, or a new panel size. Host calls that need the project are answered on the UI thread. The
tree on screen stays until the new one arrives. A failed call shows its error above the last tree, and
the events it carried are delivered again with the next call. Events of a hidden tab are dropped.

### App events

| Call | Returns |
|---|---|
| `ui.on(Topic::Selection)` | `AppEvent::Selection { layers, properties, keyframes }`: selected layer ids, `[layer, property uid]` pairs and the number of selected keyframes |
| `ui.on(Topic::Time)` | `AppEvent::Time { comp, time }`: the current time of the active comp in seconds |
| `ui.on(Topic::Comp)` | `AppEvent::Comp { comp }`: the active comp, or `None` |
| `ui.on(Topic::Project)` | `AppEvent::Project { path }`: a project was opened, created, closed or saved under a new path |
| `ui.on(Topic::Edit)` | `AppEvent::Edit { layers }`: the project changed; `layers` are the active comp's layers that changed, were added or removed |

`ui.on` subscribes for the call's reply and returns the event when it arrived in this call. Each kind is
delivered at most once per UI frame with its latest contents.

### Timers

| Call | Does |
|---|---|
| `ui.every(name, ms) -> bool` | Calls the panel every `ms` while the panel's replies ask for it; `true` when it fired for this call |
| `ui.after(name, ms) -> bool` | Calls the panel once, `ms` after the reply that armed it; `true` when it fired. Asked for again in the call it fired in, it arms again |

`ms` is at least 16 (`MIN_TIMER_MS`). Timers wait while the tab is hidden and start over when it shows.
A timer the next reply does not ask for is stopped.

### Keyboard, pointer and drops

| Field or call | Holds |
|---|---|
| `c.pointers` | Every pointer event over the canvas since the last call, in order |
| `c.pointer` | The last of `c.pointers` |
| `c.keys` | Every key pressed while the canvas had the keyboard focus: `Key { name, modifiers }`. Names are egui key names: `Left`, `Right`, `Up`, `Down`, `Enter`, `Escape`, `Delete`, `Backspace`, `Space`, `A` to `Z`, `0` to `9`, `F1` … |
| `ui.drop_target(key, \|ui\| …) -> Option<&Dropped>` | `Dropped::Files { paths }`, `Dropped::Items { ids }` (project items) or `Dropped::Data { plugin, data }`. A dropped file is readable through `Root::User` as if the user had picked it, with no `files.read`; its folder stays closed and it is not writable |
| `ui.drag_source(key, drag, \|ui\| …)` | Lets the user drag its contents. `Drag::File { root, path }` drags a file from `Root::Bundle` or `Root::Data` (or `Root::User` with `files.read`); `Drag::Data { data }` drags text to a plugin drop target |

A panel's widgets have the automation ids `plugin.<plugin>.<panel>.<widget id>`, and its labels
`plugin.<plugin>.<panel>.label.<n>` in drawing order, so the control channel can read a panel's text.

Every widget takes the focus with Tab. A canvas also takes it on a click, keeps the arrow keys while it
has it, and shows a focus ring.

A reply of an older SDK is a bare widget list. The app redraws such a panel every frame, on the worker
thread, and does not send it key or drop events.

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
`polygon` (convex, filled), `image` and `text`, in canvas-local pixels. A pointer event has a phase of
`Hover`, `Down`, `Drag` or `Up`, the `modifiers` held (`shift`, `alt`, `command`) and the `scroll` delta. A drag keeps reporting positions after the pointer leaves the
canvas. Colours are theme tokens (`Paint::Text`, `Paint::Accent`, `Paint::Weak`, `Paint::Surface`, …)
or `Paint::Rgba`.

Two buttons with the same label in the same container are told apart by order. Put widgets that come
and go inside their own `row`, `column` or `group` so the others keep their identity.

## On-viewer controls

`Effect::overlay` runs while the effect is selected and **View ▸ Show Layer Controls** is on. Its `cx`
is in layer pixels: `scale` 1, `offset` 0, `size` the layer size. It runs in the long-lived instance.
The app keeps the overlay and calls `overlay` again only when the request changes: a value, the time,
or the effect being selected.

Marks are drawn in the overlay's current space (`overlay.space(Space::Layer)`, the default, or
`Space::Comp`) with the current paint (`overlay.paint(Paint::Accent)`, the default):

| Method | Draws |
|---|---|
| `line(&[p, …])` | A line through the points, 1.5 points wide |
| `circle(centre, radius)` | A circle outline; the radius is in the space's pixels |
| `rectangle(min, max)` | A rectangle outline |
| `text(at, text)` | Text at 12 points, its top left at `at` |
| `shape(Shape)` | Any canvas shape (`Line`, `Bezier`, `Rect`, `Circle`, `Polygon`, `Text`); widths and text sizes are screen points, `Image` is not drawn |

Handles are in layer pixels. Each is bound to what a drag changes:

| Method | Handle | A drag sets |
|---|---|---|
| `point(cx, param)` | A crosshair at the point | The point parameter to the handle's new place |
| `angle(param, centre, at)` | A square at `at` | The angle parameter, by the turn around `centre` in degrees (clockwise on screen) |
| `radius(param, centre, pixels, unit)` | A ring of `pixels` and a square on it | The parameter to the distance from `centre` divided by `unit` (pixels per unit, above 0) |
| `rect(cx, min, max)` | The rectangle, a square at its centre and two corner crosshairs | Both point parameters, moved together; each corner alone |
| `custom(param, key, at)` | A square at `at` | The text of the custom parameter, to what `Effect::drag(cx, key, to, value)` writes |
| `handle(at, Bind)` | Any of the above, from its `Bind` | |

`named(text)` names the last handle for its tooltip and screen readers; the default is the parameter's
name. Every drag of one handle is one undo step; on a keyframed parameter it sets the key at the current
time. The focused handle moves 1 pixel per arrow key, 10 with Shift, each press one undo step. A point
parameter bound by no handle keeps the built-in crosshair.

## Viewer tools

```rust
pub trait Tool: Send + 'static {
    fn icon(&self) -> Vec<Shape>;   // default: a circle; shapes in a 16 × 16 box
    fn cursor(&self) -> Cursor;     // default: Cursor::Crosshair (also Arrow, Move, Grab, Text)
    fn event(&mut self, event: &ToolEvent, overlay: &mut Overlay) -> Result<()>;
}
```

The tool's button follows the toolbar's tools; clicking it again, or picking another tool, puts it
away. `event` runs in the long-lived instance, so the tool keeps its state between events. The overlay
it draws starts in `Space::Comp` and stays on screen until the next event.

| `ToolEvent` field | Holds |
|---|---|
| `kind` | `Down`, `Move`, `Up` (primary button), `Scroll`, `Key` |
| `comp` | The pointer in comp pixels |
| `layer`, `layer_id` | The pointer in the first selected layer's pixels, and that layer's id, or `None` |
| `modifiers` | `shift`, `alt`, `command` |
| `pressure` | 1 while the button is down, else 0 |
| `scroll` | The wheel delta of a `Scroll` |
| `key` | The key of a `Key`, such as `Escape`, `Enter`, `A` |
| `zoom` | Screen points per comp pixel |
| `time` | The comp time in seconds |

`Move` comes while the pointer is over the viewer or the button is held. `Scroll` and `Key` come while the
pointer is over the viewer and no text field has the keyboard. The project changes a tool makes with
`host::command` between `Down` and `Up` are one undo step named after the tool.

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
| `start(task, args)`: starts a task, returns its id | None |
| `task(id)`: `TaskState { status: Running, Done, Failed or Cancelled, progress, message, result, error }` | None |
| `cancel(id)` | None |
| `frame(FrameRequest { comp, layer, time, scale })`: the comp (the active one when `comp` is `None`), or one layer with its effects, its track matte and the comp's cameras and lights (every other layer is hidden; the layer itself shows even when its video switch is off, as through a layer parameter), as a premultiplied RGBA f32 `Image`. Refused inside an effect's `layout` or `render`. `scale` is above 0 and at most 4. | `document.read` |
| `audio(AudioRequest { layer, start, duration, rate })`: interleaved f32 samples of the active comp's mix, or of one layer, with the channel count (2). Rates 1000 to 192000 Hz, at most 2^25 sample frames per call. Refused inside an effect. | `document.read` |
| `pick_file(PickFile { save, title, extensions })`: the app's file dialog, `Some(path)` or `None`. The chosen path is then readable, and writable when `save` is set, through `Root::User` for the rest of the session. The host holds the identity of the picked file: once it is replaced, for example by a link, reading or writing it fails with `is no longer the file the user picked`, and the plugin should ask the user to pick it again. A new save target is created in the folder the user picked it in, even if that folder is moved afterwards. A path inside the app's own plugin folders is refused (`that folder belongs to the app`). Panels and commands only. Refused in headless runs. | None |
| `upload_image(key, width, height, rgba8)`: stores an image for `ui.image` and `c.image` | None |
| `remove_image(key)`: removes an uploaded image and frees its share of the 256 MiB image budget | None |
| `clipboard()`: the text on the system clipboard | `clipboard` |
| `set_clipboard(text)`: replaces the text on the system clipboard | `clipboard` |

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
| `task` | None; cancel stops it |
| Running tasks per plugin | 4 |
| Memory of one sandboxed instance | 3 GiB |
| Padding | 4096 pixels |

A sandboxed plugin that passes a limit or panics is stopped. The next call starts a new instance, and
panel state is lost. A host call is not interrupted: a file call on a stalled disk, or a slow app command, runs to
its end, and the plugin is stopped when it returns past the limit. A write the plugin is told failed for that
reason has still landed.

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
| `plugin.startTask` | `plugin`, `task`, `args` | Starts a plugin task, returns its id |
| `plugin.tasks` | `wait`, `timeout` (600 s) | Every plugin task with status, progress, message, result and error. With `wait`, answers the tasks' app calls until none runs. |
| `plugin.cancelTask` | `plugin`, `id` | Cancels a task |
| `window.plugins` | | Opens the plugin manager |
| `window.pluginPanel` | `plugin`, `panel` | Opens a plugin panel |

An automation agent confined to folders cannot run any of these except `plugin.list`.

Installed plugins live in `Plugins/` in the settings folder. `Plugins/state.json` records each approval.
`Plugins/.data/<id>/` is a plugin's private data folder.
