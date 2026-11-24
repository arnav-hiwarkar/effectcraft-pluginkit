# Plugin reference

Types and calls of the `craft-plugin` crate, the manifest, and the app commands for plugins. To learn the
workflow, read [Write a plugin](plugin-sdk.md).

## plugin.json

| Field | Meaning |
|---|---|
| `id` | Reverse-domain, lower case, at least two segments. Effect ids become `<id>.<effect>`. |
| `name`, `version` | Shown in the plugin manager. Required. |
| `author`, `description`, `license` | Optional; `cargo xtask plugin check` warns when the description or the licence is missing. |
| `entry` | `{"wasm": "plugin.wasm"}`, `{"native": {"macos-aarch64": "libx.dylib", "linux-x86_64": "libx.so", "windows-x86_64": "x.dll"}}`, or both keys together. File names only; they live in the plugin folder. A plugin with `wasm` runs sandboxed unless the user chooses Run Unsandboxed and the plugin has a native build for this machine. |
| `permissions` | Any of `document.read`, `document.write`, `files.read`, `files.write`, `clipboard`. |
| `contributes` | Data a plugin ships with no code: `{"presets": [{"effect", "name", "values"}], "luts": ["look.cube"]}`. See [Data-only plugins](#data-only-plugins). |

### Data-only plugins

A manifest with no `entry` and a non-empty `contributes` is a data-only plugin. Nothing runs, so it takes no
permissions (a manifest that lists any is refused) and installing it needs no approval: the install
approves it, and changing its files later waits for approval like any plugin.

- `presets`: `effect` is a built-in effect id (`ec.color.tint`) or a loaded plugin's effect id
  (`<plugin id>.<effect>`); `name` is unique for that effect within the plugin; `values` maps parameter
  ids to values as in `.preset()` (number, true or false, a list of 2, 3 or 4 numbers, text; a popup takes
  its option index). They appear in the effect's Presets menu and in `effect.presets` and apply with
  `effect.applyPreset`.
- `luts`: `.cube` file names in the plugin folder (no subfolders). They appear as `<plugin name> / <file>`
  in the Library menu next to Choose... of Apply Color LUT and the other LUT file parameters.

The install is refused with the reason when an effect or parameter does not exist (the message lists the
parameters it has), a value has the wrong type, a LUT file is missing, too large (256 MB) or not a valid
`.cube` file, a name is listed twice, or the manifest gives neither an `entry` nor `contributes`. A plugin
that fails the same checks at startup shows as Failed with the same message. A preset for another plugin's
effect needs that plugin installed and running; data-only plugins start last.
`cargo xtask plugin new <name> --kind data` makes one; `cargo xtask plugin <dir>` packs it.

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
    fn migrate(&self, from: u32, values: &mut BTreeMap<String, Value>); // default: leaves the values
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
| `.params([Param, …])` | Adds parameters, such as the list `params!` makes. |
| `.group(name, [Param, …])` | Adds parameters under a twirl-down that shows `name`. Their ids become `name/<id>`, see [Parameter ids](#parameter-ids). |
| `.time_dependent()` | The output changes with `cx.time` when no parameter is animated. |
| `.reads_other_frames()` | The effect calls `cx.source_at`, `cx.layer_at`, `cx.scene_at` or `cx.placement_at`. |
| `.reads_scene()` | The effect calls `cx.scene`, `cx.scene_at` or `cx.placement`. |
| `.reads_param_history()` | The effect calls `cx.at` or `cx.param_history`. A layer whose other effects or masks have expressions is then not cached, because the key cannot see what an expression returned at another time. |
| `.preset(name, [(id, Value), …])` | A named set of parameter values, see [Presets](#presets). |
| `.shader(wgsl)` | WGSL kernels for GPU passes. Each invocation may run 1 048 576 loop iterations; past that the app turns the shader off and renders on the CPU (see [Time](plugin-gpu.md#time)). See [Passes](plugin-gpu.md). |
| `.designer(panel_id)` | Effect Controls shows a button that opens this panel. |
| `.version(n)` | The effect's state version (0 when not set), see [State versions](#state-versions). |

The render cache uses the four declarations. An effect that omits one shows stale frames. The
layer's cache key already holds every keyframe and expression of the effect, not only the values at
the current time (`hash_debug(&mut h, c)` in `crates/render/src/cache.rs`, `key_any`), so a keyframe
moved at frame 12 also redraws frame 40.

### Parameter ids

A parameter id is up to 8 parts joined by `/`, at most 256 bytes; the parts before the last name twirl-down
groups and are shown as written (`Phosphor Glow/glow` shows a `Phosphor Glow` group). A part may hold any
character except `/` and control characters, must not be empty, and must not start with `#` or `@`, which
property paths read as an index (`effects/#1`) or a uid. An effect has at most 512 parameters, each with its
own id. A plugin that breaks one of these does not load, and the message names the effect, the id and the rule.

### Presets

`.preset("Sun", [("look", Value::Number(1.0)), ("Glow/brightness", Value::Number(120.0))])` names a
set of values. Ids are the full parameter ids, including the group prefix. Values have the form `cx`
reads them in, except points: a point is a fraction of the layer, like a parameter's default. A slider
value is clamped to its hard range. A preset that names an id the effect does not declare, or gives a value
of the wrong kind, stops the plugin from loading with the effect, the preset and the id, for example
`effect `glow`: preset `Wide` sets `radus`, which is not one of its parameters ["radius"]`. Layer and mask
parameters in a preset are ignored.

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
| `Param::slider(id, name, default, min..=max)` | `cx.f(id)` | number, always inside `min..=max` |
| `Param::percent(id, name, default)` | `cx.f(id)` | a number shown with `%`, not limited |
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
| `Param::gradient(id, name, &[(at, [r, g, b, a]), …])` | `cx.gradient(id)` | a `Gradient`; `.sample(t)` gives RGBA |
| `Param::curve(id, name, &[[0.0, 0.0], [1.0, 1.0]])` | `cx.curve(id)` | a `Curve`; add `.channels()` for master, red, green and blue, read with `cx.curves(id)` |

A slider has two ranges. `min..=max` is the hard range: what the user can type and what `cx.f` returns, even
when an expression or a script pushes the value outside it. `.soft(low..=high)` is what the slider under the
row shows, so a value up to 1000 can be typed while the slider covers 0.01 to 10. `.log()` spaces the slider
by ratio, which suits gains and sizes; it needs a soft (or hard) minimum above 0 and is linear otherwise.
`.range(min..=max)` gives a percent parameter a hard range and a slider (a percent without one has none).
The modifiers work in any order: `percent(...).soft(...).log().range(...)` keeps the soft range and the log
scale.
A hard range with its larger end first (`10.0..=1.0`) stops the plugin from loading with
`effect `glow`: parameter `gain` has the range 10..=1; give the smaller end first`.
`param.fit(value)` is the check the app applies to every value it delivers and the test kit applies to every
`.set`: it clamps a slider to its hard range, truncates a popup or mask index, and refuses a value of the
wrong kind with `parameter `amount` takes a number, not the text "x"`.

```rust
Param::slider("gain", "Gain", 1.0, 0.0..=1000.0).soft(0.01..=10.0).log()
Param::percent("mix", "Mix", 50.0).range(-100.0..=300.0)
```

A parameter declared without these behaves as before. `effect.list` reports `sliderMin`, `sliderMax`,
`log` and `percent` for them.

#### Gradients

A gradient parameter shows its colours as a strip in Effect Controls; clicking it (or Enter on it) opens the
app's Gradient Editor, and its stopwatch keyframes it like any property. Keyframes with the same number of
stops blend stop by stop. `Gradient { colors: Vec<(f64, [f64; 4])>, opacities: Vec<(f64, f64)> }` holds
stops at positions 0 to 1 in order; `Gradient::new(colors)` adds full opacity at both ends.
`gradient.sample(t)` clamps `t` to 0..=1, blends the two colour stops around it and the two opacity stops
around it, and returns the colour's alpha times the opacity, exactly as the app samples its own gradients.
A default needs 1 to `MAX_GRADIENT_STOPS` (64) colour stops, at most 64 opacity stops, positions in
0..=1 and finite colours, or the plugin does not load:
`effect `ramp`: gradient `ramp` has 65 colour and 2 opacity stops; give 1 to 64 colour stops and at most 64 opacity stops`.
A gradient the user edits to more stops reaches the plugin with its first 64, and the plugin's journal gets one warning naming the parameter and the stops dropped, once per changed value. In the test kit pass a
`Gradient` to `.set`, or `{"colors": [[0, [0, 0, 0, 1]], [1, [1, 1, 1, 1]]]}` to `.with` and to
`plugin look --set`.

#### Curves

A curve parameter shows the app's curve graph under its row: click adds a point, dragging moves one, dragging
one off removes it, and Reset goes back to a straight line. With `.channels()` the graph has Master, Red,
Green and Blue tabs. Each drag is one undo step. `cx.curve(id).at(x)` evaluates the monotone cubic through
the points (the spline the app's curve editors draw), extended along the end slopes outside the first and
last point; `.lut::<256>()` samples it at 256 even steps from 0 to 1 for a lookup table. `cx.curves(id)`
gives one `Curve` per channel.
The value is text: the points `x,y` separated by spaces, the channels separated by `|`
(`"0,0 0.5,0.7 1,1"`), so a preset sets it with `Value::Text`. Every value the plugin receives has 2 to
64 points per channel (`MAX_CURVE_POINTS`), each coordinate in 0 to 1 and x increasing; `fit` clamps and
sorts the points and refuses a curve with fewer or more points, or the wrong number of channels. A default
that breaks these rules stops the plugin from loading with `effect `tone`: parameter `curve`: the default
a curve has 2 to 64 points, not 1`.

#### Typed parameters

`params! { struct Name { field: Type = builder(name, …), "Group" { field: Type = builder(…), … }, … } }`
makes a struct with one field per parameter and two functions: `Name::params()`, the `Vec<Param>` for
`EffectSpec::params`, and `Name::read(cx)`, the struct filled from `cx`. The builder is any `Param` builder
above without its first argument; the id is the field name, or `Group/field` inside a group. Range builders chain after it: `gain: f64 = slider("Gain", 1.0, 0.0..=1000.0).soft(0.01..=10.0).log()`. `Name::IDS`
lists the ids. A field can state its saved id with `as`: `kind as "type": usize = popup("Type", &TYPES, 0)`
saves `type` (or `Group/type`), so an id that is a Rust keyword, or the same id in two groups
(`core_gain as "gain"` in `"Core"`, `glow_gain as "gain"` in `"Glow"`), needs no hand-written parameter.
A struct may have no fields, a group may be empty, and a trailing comma is allowed after a field or a group.
The field type must fit the builder; a `String` field with `slider(...)` stops the build with
`a slider, percent or angle parameter is read as a number, so its field cannot be String`:

| Type | Read with | For |
|---|---|---|
| `f64`, `f32` | `cx.f` | slider, percent, angle |
| `bool` | `cx.bool` | checkbox |
| `usize`, `u32` | `cx.index` | popup, mask |
| `[f64; 2]`, `[f64; 3]` | `cx.point`, `cx.point3` | point, point3 |
| `[f32; 4]` | `cx.color` | color |
| `String` | `cx.text` | text, custom |
| `Curve`, `Vec<Curve>` | `cx.curve`, `cx.curves` | curve, curve with `.channels()` |
| `LayerParam` | `{ id, connected }`; `.image(cx)` and `.image_at(cx, comp_time)` read the layer | layer |
| `Gradient` | `cx.gradient` | gradient |

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
right, bottom)` after padding each side by its own amount. Both pad as far as the app does:
`applied_padding([left, top, right, bottom], cx.size)` gives those sides. Each side is at most
`MAX_PADDING` (4096); when the padded frame would be wider or taller than 16384 or larger than 16 Mpx and
four times the frame, every side is cut by one factor, so a frame 16384 wide is not padded at all.

| Call | Returns |
|---|---|
| `cx.layer("matte")` | The layer chosen in a layer parameter, resampled onto this frame's grid. |
| `cx.layer_at("matte", comp_time)` | The same at another time. |
| `cx.source_at(layer_time)` | This effect's input at another time. Declare `.reads_other_frames()`. |
| `cx.at(layer_time)` | The same `Cx` with every parameter evaluated at another layer time, keyframes and expressions applied as the app renders that time. `time` becomes `layer_time` and `comp_time` moves by the same amount; the frame fields stay. `None` outside the app. Declare `.reads_param_history()`. |
| `cx.param_history()` | A hash of everything that sets this effect's parameters over time (values, keyframes, expression text), the same in every frame and session. `None` when a parameter has an expression, whose values at other times the hash cannot see, or outside the app. Use it to key caches of a simulation. |
| `cx.masks()` | The layer's masks as polylines in frame pixels. |
| `cx.scene()` | The comp's camera and lights, see [Scene](#scene). Declare `.reads_scene()`. |
| `cx.scene_at(comp_time)` | The same at another comp time. Declare `.reads_scene()` and `.reads_other_frames()`. |
| `cx.placement("target")` | Where the layer chosen in a layer parameter sits, see [Placement](#placement). Declare `.reads_scene()`. |
| `cx.placement_at("target", comp_time)` | The same at another comp time. Declare `.reads_scene()` and `.reads_other_frames()`. |

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
| `scene.camera` | `view` (rows of a 4x4 matrix from layer pixels to camera space, which has its origin at the camera, x right, y down and z along the view, in the same pixel units), `position` (the camera in layer pixels), `zoom` (the distance in pixels at which one unit covers one pixel), and `focus`, `Some` when depth of field is on: `distance` and `aperture` in pixels, `blur` (1 is 100 %). The app has no near or far distance. |
| `scene.lights` | Every light layer that is switched on. |
| `Scene::centred(cx.layer_size)` | A camera facing the middle of the layer that leaves the plane z = 0 where it is. Use it when `cx.scene()` is `None`. |

A `Light` has `kind` (`Ambient`, `Point`, `Spot`, `Parallel`), `position`, `direction` (the way the
light travels, unit length), `colour`, `intensity` (1 is 100 %), `cone_angle` (the full angle in
degrees) and `feather` (0 to 1). Light a surface with `colour` times `intensity`.

`cx.scene_at(comp_time)` gives the scene at another comp time, for motion blur or a camera move
between frames. It returns an error for a time that is not finite, and when the spec lacks
`.reads_scene()` or `.reads_other_frames()`: the layer cache keys such an effect on every camera,
light and layer of the comp, which it would otherwise miss. A comp with an expression on any layer
is then not cached.

### Placement

`cx.placement("target")` returns an error when no layer is chosen or the spec lacks `.reads_scene()`
(and `.reads_other_frames()` for `cx.placement_at`). A `LayerPlacement` has:

| Field or call | Meaning |
|---|---|
| `matrix` | Rows of a 4x4 matrix from the target's own pixels (x right, y down, z away from the viewer) to this layer's pixels, through the parents of both layers. It includes 3D rotation and scale. Like the scene it does not change with `cx.scale` or padding. |
| `apply(point)` | A target pixel `[x, y, z]` in this layer's pixels. |
| `opacity` | The target's own opacity, 0 to 1. |
| `size` | The target's size in its pixels; its corners are `[0, 0]` and `size`. |

The matrix maps the target's layer onto this layer's 3D space, not through the camera: to see where
it shows on screen, pass `apply(point)` to `scene.project`.

### Frame

| Field | Meaning |
|---|---|
| `frame.pixels` | The pixels this call writes: premultiplied RGBA `f32`, row 0 at the top. Values above 1 are valid. Write every pixel; a pixel left unwritten is transparent, or keeps its input with `Layout::pointwise()`. |
| `frame.width`, `frame.height` | Size of the rectangle this call writes. |
| `frame.origin` | Position of that rectangle in the full frame. Not `[0, 0]` in a band or when only part of the layer shows. |
| `frame.input` | The input this call may read (`Rows`: `width`, `height` of the full frame, `first_row`, `pixels`, `row(y)`, `at(x, y)`, in full-frame coordinates). Whole rows: the rows the call writes, plus `n` above and below for `Layout::neighbourhood(n)`, the rows `Effect::input` names with `maps_input`, every row for `Layout::whole_frame()`. In place it holds no rows until `frame.rows()` is called. |

| Method | Meaning |
|---|---|
| `frame.map(f)` | Writes each pixel as `f` of the input pixel under it. The pointwise case. |
| `frame.rows()` | `(y, input row, output row)`, `y` in the full frame, the input row cut to the columns written. |
| `frame.rows_mut()` | `(y, output row)`. |
| `frame.rows_in_place()` | `(y, row)`, each row holding the input under it, to change in place: the cheapest loop for an effect that reads only its own pixel. In place it is `rows_mut()`; when `frame.input` holds rows, each one is copied into its row first. |
| `frame.at(x, y)` | The input pixel at a full-frame position; transparent outside the input. In place, before `frame.rows()`, the pixel as `frame.pixels` holds it now. |
| `frame.drawn(rect)` | Says the call drew only inside `rect` (full-frame coordinates): the layer is transparent outside it. |
| `Frame::whole(width, height, input, pixels)` | A whole-frame call, for unit tests. |

The app asks an effect only for the part of the layer that shows in the frame being rendered (the comp
frame, or the viewer's region when it is zoomed in), grown by what the resampling onto the comp and the
later effects in the stack need, with its edges moved out to a 64-pixel grid so a layer moved a few pixels
keeps its cached render. Without padding, a layer wholly in the frame renders whole. With
`Layout::pointwise()`, `Layout::neighbourhood(n)` or `.maps_input()` that rectangle is split into bands of
rows on all cores, for sandboxed and native builds alike, and no row is written twice. A native plugin
reads the input where it lies and writes the output frame directly; a sandboxed one gets its input rows
copied in and its rows copied out. A sandboxed effect whose reach covers the frame renders in one band, so
the frame is copied in once, and its bands never stage more than 1 GiB of rows between them.

With `Layout::pointwise()`, `frame.pixels` holds the input when the call starts. When whole rows show, the
effect changes the layer's frame in place and `frame.input` holds no rows: `frame.map(f)` maps the pixels
where they lie with no copy, and `frame.rows()` first copies the band once, into memory kept for the next
call, so its input rows are the input as it was. An effect that fails in one band then leaves the bands that
finished changed. Every other case renders into a new frame that replaces the layer's frame only when
every call succeeded, so an effect that fails, in any band, leaves the layer as it was.

An effect that draws without reading its input (a generator: a shape, text, particles) adds
`.ignores_input()` to its layout and reports the rectangle it drew with `frame.drawn(rect)`. The contract:

- With `ignores_input`, `frame.input` holds no rows and `frame.pixels` starts with unspecified contents,
  not the input and not zeros. Write every pixel inside the rectangle you report; nothing outside it.
- Everything outside the reported rectangle is transparent. The app crops the layer to the rectangle (the
  union of what each band reported, clipped to the band and the frame), so nothing clears the rest of the
  frame. A rectangle that reaches past the frame is clipped to it; one wholly outside leaves the layer
  empty. A call that reports nothing drew its whole rectangle, as before.
- A sandboxed plugin then has nothing copied in and only the reported rows copied out. On a 3840x2160
  layer with a 200x200 sprite a sandboxed render drops from 6.3 to 0.42 ms (89 M to 12.5 M instructions),
  a native one from 1.28 to 0.14 ms (`examples/gen_measure.rs` in craft-plugin-host).
- `frame.drawn` also works without `ignores_input`, with the same meaning: outside the rectangle the
  layer is transparent.
- An app older than these two calls ignores `ignores_input` and refuses a render that reports a
  rectangle, so a plugin that uses them needs an app that has them.
- Bands that report different rectangles are cropped to their union; where a band reported nothing the
  layer is transparent.

A plugin built before frames had separate input (it sends no `separate_input` in its description) still
runs: each band gets whole rows with `n` rows above and below in `frame.pixels`, the app copies them for it
and keeps the band's own rows.

### Layout

| Constructor | Meaning |
|---|---|
| `Layout::whole_frame()` | One call gets the whole frame. |
| `Layout::pointwise()` | Each output pixel depends only on the same input pixel. |
| `Layout::neighbourhood(n)` | Each output row depends on input rows at most `n` away. |
| `.padded(n)` | Grow the frame by `n` pixels on each side first, as far as `applied_padding` allows (each side at most 4096). |
| `.padded_sides(left, top, right, bottom)` | Grow each side by its own amount. |
| `.maps_input()` | The app asks `Effect::input` which input rectangle an output rectangle needs, once per band. |
| `.ignores_input()` | The effect reads no input: nothing is copied in, and `frame.pixels` starts unspecified. Pair it with `frame.drawn(rect)` (see Frame). |

`Rect { left, top, right, bottom }` is in frame pixels, `right` and `bottom` exclusive, and may reach past
the frame; the app clips it. `Rect::of_size`, `grown(n)`, `clipped(size)`, `union`, `intersection`, `contains`, `width`,
`height` and `is_empty` help build one.
| `.passes(plan)` | Run these passes on the GPU when the GPU renderer is active, and otherwise on the CPU through `Effect::pass`, in bands on all cores. |
| `.gpu(plan)` | Run these passes when the GPU renderer is active; `render` is the CPU path. |

`PlanPass::reach(n)` gives a pass the number of source rows above and below the matching source row that
an output row reads. Without it the pass runs as one call on the CPU. `PlanPass::aux_reach(n)` sets the
reach of `aux` alone; without it `aux` uses the pass's reach.

### Host kernels

Passes the app computes itself, on the CPU natively and on the GPU with its own WGSL, within 2/255 of
each other. See [Host kernels](plugin-gpu.md#host-kernels) for what each computes.

| Builder | Fields and limits |
|---|---|
| `PlanPass::blur(src, out, radius)`, `PlanPass::box_blur(..)` | `HostKernel::Blur { shape: BlurShape::Gaussian | Box, radius: [f32; 2] }`, each axis 0 to `MAX_BLUR_RADIUS` (512). `src` and `out` the same size. |
| `PlanPass::resample(src, out, filter)` | `HostKernel::Resample { filter: Filter::Bilinear | Bicubic, scale: Option<[f32; 2]> }`. `scale` is source pixels per output pixel, above 0 and at most 64; `None` takes it from the sizes. |
| `PlanPass::blend_images(base, top, out, mode, amount)` | `HostKernel::Blend { mode: BlendMode, amount: f32 }`, `top` as `aux`. All three the same size; `amount` finite. `BlendMode::ScreenHdr` is `base + top * (1 - clamp(base, 0, 1))`: Screen for a base in 0 to 1, an add below 0, the base kept above 1. |
| `PlanPass::curves(src, out, table)` | `HostKernel::Curves { table: Vec<Pixel> }`, 2 to `MAX_CURVE_POINTS` (4096) entries. |
| `PlanPass::convolve(src, out, taps)` | `HostKernel::Convolve { taps: Taps::Separable { x, y } | Taps::Full { width, weights } }`. Separable: odd counts up to `MAX_TAPS` (1025). Full: odd sides up to `MAX_FULL_SIDE` (31). |
| `PlanPass::warp(src, displacement, out, amount)` | `HostKernel::Warp { amount: [f32; 2] }`, displacement as `aux`, each axis at most `MAX_WARP` (4096) pixels. |
| `PlanPass::matrix(src, out, rows)` | `HostKernel::Matrix { rows: [[f32; 4]; 4] }`, finite. |
| `PlanPass::to_linear(src, out)`, `PlanPass::to_srgb(src, out)` | `HostKernel::ToLinear`, `HostKernel::ToSrgb`. `src` and `out` the same size. |
| `PlanPass::transfer(src, out, to, negatives)` | `HostKernel::Transfer { to: Encoding::Linear | Srgb, negatives: Negatives::Mirror | Linear }`. `Mirror` is `to_linear` or `to_srgb`; `Linear` continues the curve's straight segment below zero (`x / 12.92` decoding, `x * 12.92` encoding). `src` and `out` the same size. |
| `PlanPass::brightness_curve(src, out, measure, curve, range)` | `HostKernel::BrightnessCurve { measure: Measure::MaxChannel | Luma, curve: Vec<f32>, range: f32 }`, 2 to `MAX_CURVE_POINTS` (4096) entries, `range` above 0. |
| `PlanPass::dither(src, out, seed, origin)` | `HostKernel::Dither { seed: u32, origin: [i64; 2] }`. |
| `PlanPass::light_alpha(src, out)` | `HostKernel::LightAlpha`. |
| `PlanPass::dilate(src, out, radius)`, `PlanPass::erode(..)` | `HostKernel::Dilate { radius: [u32; 2] }`, `HostKernel::Erode { .. }`, each axis 0 to `MAX_MORPHOLOGY_RADIUS` (512). `src` and `out` the same size. |
| `PlanPass::directional_blur(src, out, angle, length)` | `HostKernel::DirectionalBlur { angle: f32, length: f32 }`, `angle` in degrees, `length` 0 to `MAX_DIRECTIONAL_LENGTH` (1024) pixels. `src` and `out` the same size. |
| `PlanPass::bilateral(src, out, radius, tolerance)` | `HostKernel::Bilateral { radius: u32, tolerance: f32 }`, `radius` 0 to `MAX_BILATERAL_RADIUS` (32), `tolerance` above 0. `src` and `out` the same size. |
| `PlanPass::noise(out, scale, octaves, seed, origin)` | `HostKernel::Noise { scale: f32, octaves: u32, seed: u32, origin: [i64; 2] }`, `scale` above 0 pixels, 1 to `MAX_NOISE_OCTAVES` (12) octaves. Its `src` is `PlanImage::Source` and is not read. |
| `PlanPass::hsl_range(src, out, hue, saturation, lightness, softness)` | `HostKernel::HslRange { hue: [f32; 2], saturation: [f32; 2], lightness: [f32; 2], softness: f32 }`, `hue` 0 to 360 degrees, the others 0 to 1. `src` and `out` the same size. |
| `PlanPass::splat(src, out, dabs)` | `HostKernel::Splat { dabs: Vec<Dab> }`, `Dab { at: [f32; 2], radius: f32, colour: [f32; 3], opacity: f32, hardness: f32 }`: at most `MAX_DABS` (65536), `radius` 0 to `MAX_DAB_RADIUS` (256), colour 0 or more, `opacity` and `hardness` 0 to 1, and together covering at most `MAX_DAB_TILES` (2²¹) tiles of 16×16 pixels. `src` and `out` the same size. |
| `PlanPass::host(kernel, src, out)` | Any `HostKernel`. |

Every value must be finite. `kernel.check(src_size, aux_size, out_size)` is the check `plan.validate`
runs for one pass. `kernel.pointwise()` is true for the kernels that may write the image
they read and that the app fuses into one loop.

`craft_plugin::histogram(pixels)` returns `[[u32; 256]; 4]`: for red, green, blue and alpha, how many of
the pixels have each straight (unpremultiplied) value, rounded to 255ths and clamped to 0 to 1; a
transparent pixel counts as 0 in every channel. It runs in the plugin, not as a plan pass: a plan's
images go to the app and never back to the plugin during the render, so a host histogram would have no
reader. Call it in `render` or `pass` on the frame you were given, or in a task on frames it reads.

`plan.validate(frame)` checks a plan against the limits in [Passes](plugin-gpu.md#limits) for a padded
frame of that size and returns the first rule it breaks. The app runs this same check before it runs a
plan on the GPU or the CPU, and reports the reason as the effect's error. `plan.check(frame)` also
refuses a plan the CPU cannot run (one with draw passes, buffers, textures or layers).

## Projects

A saved project records `"plugins": [{"id", "version", "digest"?}]`, one entry for each plugin with an
effect in the project. The entry comes from the loaded plugin's `plugin.json`; while the plugin is
missing, the entry saved before is kept for as long as one of its effects is in the project, so saving
again changes neither the record nor the effect's values. `digest` is not written yet. A project
without the list opens as before.

| Where | What happens |
|---|---|
| `file.open` | Result `plugins`: `[{id, version, installed, status}]` for each recorded plugin that is `missing` (`installed: null`) or a `differentVersion`, plus one notification naming them. |
| `renderQueue.render`, Watch Folder | Refused with an error naming the plugins of the project's effects that are not loaded, unless `{"withoutMissingPlugins": true}` (CLI `render --without-missing-plugins`); the effects are then skipped. |
| Preview, Effect Controls | The effect is skipped and its header shows a warning: "Plug-in `<id>` is not installed". |

### State versions

Each saved instance of an effect records the effect's state version (`"state_version"` on its group,
left out at 0). When a project saved with an older version opens, or the plugin starts while it is open,
the app calls `Effect::migrate(from, values)` once per instance before anything else reads it. `values` holds
the instance's saved static values by full parameter id, ids the effect no longer declares included, in the
form `cx` reads them, except points, which are in layer pixels; layer and mask values are not passed.
Change, add or remove entries: a removed id whose value reappears under a new id moves there with its
keyframes and expression, a changed value replaces the static value, a removed id is deleted. The instance
is then stamped with the current version. An effect that never calls `.version` is never asked.

An instance saved by a newer version of the plugin than the one installed is left exactly as saved,
unknown parameters included, and saving writes it back unchanged; opening it shows a notification
naming the layer, the effect and both versions (and `file.importProject` lists it in its warnings). A migration
that returns an error, or a value the parameter cannot hold, also leaves the instance unchanged and is
reported the same way. Layer and project [plugin data](#plugin-data) are not versioned; keep a version
in the data itself.

### Plugin data

A plugin keeps its own bytes in the project and on layers: `host::data(place, key)` returns
`Option<Vec<u8>>`, `host::set_data(place, key, bytes)` writes, `host::remove_data(place, key)` removes.
`place` is `Place::Project`, `Place::Layer(id)` (a layer id from `layer.list` or the selection) or, inside an
effect's `render`, `Place::OwnLayer` (`cx.layer_data(key)`). Each plugin sees only its own data.

| Rule | Value |
|---|---|
| Permission | reading `document.read`, writing `document.write` |
| Keys per place | 64 (`MAX_DATA_KEYS`), 1 to 256 bytes each |
| One value | 256 KiB (`MAX_DATA_VALUE`) |
| One plugin in one project | 4 MiB, keys and values over every place (`MAX_DATA_PER_PLUGIN`) |
| Undo | Each write is an undo step; all the edits of one command or one panel call are one step, named after the command |
| Effects | Read `Place::OwnLayer` only, never write. The layer's data is in its cache key, so a changed value redraws the effect |
| Saved | `plugin_data` on the project and on each layer, per plugin id, values in base64. Kept byte for byte, and saved again unchanged, while the plugin is missing |

A write past a limit, to a layer that does not exist, or from an effect is refused with a message
that says what to change.

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

`craft_plugin::math::fast` has cheaper forms for loops that call several of these per pixel: shorter
polynomials, a one-step range reduction and no guards, so fewer instructions. Their errors are still far below
one level of a 16-bit channel.

| Function | Valid range | Largest error |
|---|---|---|
| `fast::sin(x)`, `fast::cos(x)` | any; the error grows with `abs(x)` | 2e-6 absolute within 20 radians, 7e-6 within 100, 6e-5 within 1000 |
| `fast::exp(x)` | `-87..=88`, clamped | 9e-6 relative within `-20..=20`, 1.2e-5 over the range |
| `fast::exp2(x)` | `-126..=126`, clamped | 3.3e-6 relative |
| `fast::ln(x)` | normal `x > 0`; zero gives `-127 * LN_2`, a negative `x` gives `ln(-x)`, a subnormal one a wrong value near `-88` | 1.2e-7 times `max(1, abs(result))` |

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
tree on screen stays until the new one arrives. A failed call shows its error above the last tree; the
events it carried are not delivered again. Events of a hidden tab are dropped, and so is a reply that
arrives after its tab was hidden: the panel is called again when it shows. A click, a checkbox toggle or
a drop reaches one call, so two clicks on one button during a slow call give two calls in which
`button` returns `true`. Pointer motion, slider, number, text and colour events made during a call
coalesce into the latest one; presses, releases and keys stay in order. Pending `edit` events join their
layer lists. A reply, a host call or an edit made for a project that has since closed, or by a load of
the plugin that was reloaded or turned off, is dropped.

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

### Custom parameter rows

The row `Effect::controls` draws for a `Param::custom` is kept and called again like a panel, on the same
worker, so `ui.on`, `ui.every` and `ui.after` work in it as they do in a panel. Each row has its own
subscriptions and timers. It is also called again when its value changes. A row the app does not draw
(its layer is not shown in Effect Controls, or Effect Controls is closed) waits like a hidden tab. A row
built with an older SDK subscribes to nothing and arms no timer.

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
`number_in`, `text_edit`, `popup`, `color`, `separator`, `space`, and the containers `row`, `column` and `group`.
Value widgets take `&mut` state and return `true` on the frame the user changed it.
`ui.number_in(text, &mut value, speed, min..=max, decimals)` is `number` with a range the value stays in and a
fixed number of decimals. A value that starts outside the range is clamped quietly; `number_in` returns `true`
only when the user edited it.

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
`polygon` (filled; concave shapes fill correctly, up to 2048 points, and more points are filled as a convex
shape; repeated points count once, and an outline that crosses itself draws nothing), `image` and `text`, in canvas-local pixels. A pointer event has a phase of
`Hover`, `Down`, `Drag` or `Up`, the `modifiers` held (`shift`, `alt`, `command`) and the `scroll` delta. A drag keeps reporting positions after the pointer leaves the
canvas. Colours are theme tokens (`Paint::Text`, `Paint::Accent`, `Paint::Weak`, `Paint::Surface`, …)
or `Paint::Rgba`.

Two buttons with the same label in the same container are told apart by order. Put widgets that come
and go inside their own `row`, `column` or `group` so the others keep their identity.

## On-viewer controls

`Effect::overlay` runs while the effect is selected and **View ▸ Show Layer Controls** is on. Its `cx`
is in layer pixels: `scale` 1, `offset` 0, `size` the layer size. On an adjustment layer the effect
works on the comp below, so `size` is the comp size and the handles are drawn in comp pixels. It runs in
the long-lived instance, on the plugin's worker thread like a panel; the last overlay stays on screen
until the new one arrives.
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

**Caps.** One overlay, from an effect or a tool, keeps its first 4096 shapes and 256 handles, 4096 points
per `Line` or `Polygon` and 512 characters per `Text`, in the order drawn. The rest is dropped and the app
logs one warning naming the plugin and the count. Draw a coarser overlay instead of a dense one.

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
name. Every drag of one handle is one undo step; on a keyframed parameter it sets the key at the time the
drag was made. A `custom` handle's `drag` runs on the worker; the value it leaves is set when it
arrives, also when it is the value the drag started from, and a drag of a reloaded plugin stops. The
release sends the pointer's last position, and drags waiting behind a slow call merge only within one
handle and one gesture. The focused handle moves 1 pixel per arrow key, 10 with Shift, each press one undo step. A point
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
away. `event` runs in the long-lived instance, on the plugin's worker thread, so the tool keeps its state
between events. Events arrive in order; moves made while `event` was busy coalesce into the latest. The
overlay it draws starts in `Space::Comp` and stays on screen until the next reply.

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

**From automation.** A tool is named `<plugin id>/<tool id>`. `ui.set {"tool": "plugin/tool"}`
and the command `tool.plugin/tool` (through `engine.execute` or `ui.menu.invoke`) pick it, like the built-in tools,
and any built-in tool picked afterwards puts it away. An unknown name is an error that lists the loaded plugin tools.
`ui.click`, `ui.drag` and `ui.scroll` over the viewer then reach the tool as `Down`, `Move`, `Up` and `Scroll` events.

`Move` comes while the pointer is over the viewer or the button is held. `Scroll` and `Key` come while the
pointer is over the viewer and no text field has the keyboard. The project changes a tool makes with
`host::command` between `Down` and `Up` are one undo step named after the tool. An edit made elsewhere in
the meantime stays its own step, and the tool's later edits start a new one. Putting the tool away
during a gesture ends it.

## Host calls

All are in `craft_plugin::host`.

| Call | Permission |
|---|---|
| `command(id, params)` for a command that only reads | `document.read` |
| `command(id, params)` for a command that edits the project | `document.write` |
| `command(…)` that names a file, such as `file.saveCopy` | `files.read` and `files.write`, as well |
| `read_file(Root::Bundle, path)`, `list_files(Root::Bundle, path)`; `read_file` returns a `FileError { kind, message }` whose `kind` is `NotFound`, `PermissionDenied` or `Other`, and `?` turns it into an `Error`. The app sends the kind as its own field, so a file or folder name never changes it | None |
| `read_file`, `write_file`, `list_files` with `Root::Data` | None |
| `read_file(Root::User, path)`, `list_files(Root::User, path)` | `files.read` |
| `write_file(Root::User, path, bytes)` | `files.write` |
| `log(level, message)`, `info(message)` | None |
| `notify(message)`: shows a short message to the user | None |
| `start(task, args)`: starts a task, returns its id | None |
| `task(id)`: `TaskState { status: Running, Done, Failed or Cancelled, progress, message, result, error }` | None |
| `cancel(id)` | None |
| `frame(FrameRequest { comp, layer, time, scale, ids, source })`: the comp (the active one when `comp` is `None`), or one layer with its effects, its track matte and the comp's cameras and lights (every other layer is hidden; the layer itself shows even when its video switch is off, as through a layer parameter), as a premultiplied RGBA f32 `Image`. Refused inside an effect's `layout` or `render`. `scale` is above 0 and at most 4. With `source`, the named layer's source before its masks, effects and transform, in layer pixels (the source's size, or the comp's for a layer without one), at the layer's time for comp time `time`. With `ids`, `comp` and `layer` are ids, never names. | `document.read` |
| `audio(AudioRequest { layer, start, duration, rate })`: interleaved f32 samples of the active comp's mix, or of one layer, with the channel count (2). Rates 1000 to 192000 Hz, at most 2^25 sample frames per call. Refused inside an effect. | `document.read` |
| `pick_file(PickFile { save, title, extensions })`: the app's file dialog, `Some(path)` or `None`. The chosen path is then readable, and writable when `save` is set, through `Root::User` for the rest of the session. The host holds the identity of the picked file: once it is replaced, for example by a link, reading or writing it fails with `is no longer the file the user picked`, and the plugin should ask the user to pick it again. A new save target is created in the folder the user picked it in, even if that folder is moved afterwards. A path inside the app's own plugin folders is refused (`that folder belongs to the app`). Panels and commands only. Refused in headless runs. | None |
| `upload_image(key, width, height, rgba8)`: stores an image for `ui.image` and `c.image` | None |
| `remove_image(key)`: removes an uploaded image and frees its share of the 256 MiB image budget | None |
| `clipboard()`: the text on the system clipboard | `clipboard` |
| `set_clipboard(text)`: replaces the text on the system clipboard | `clipboard` |
| `data(place, key)`: the plugin's bytes at the project or a layer (see Plugin data) | `document.read` |
| `set_data(place, key, bytes)`, `remove_data(place, key)` | `document.write` |

`craft_plugin::log!(level, "format {args}")` calls `host::log` with the file and line in front of the
message; `level` is `info` (the default when left out), `warn` or `error`.

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

A sandboxed plugin that passes a limit or panics is stopped. A panic is reported as `the plugin panicked at
<file>:<line>:<column>: <message>`: a native plugin returns it as the call's error, and every plugin writes it
to its log before it stops. The next call starts a new instance, and
panel state is lost. A host call is not interrupted: a file call on a stalled disk, or a slow app command, runs to
its end, and the plugin is stopped when it returns past the limit. A write the plugin is told failed for that
reason has still landed.

## App commands

| Command | Parameters | Does |
|---|---|---|
| `plugin.list` | | Every plugin with its state, permissions, effects, panels, and commands |
| `plugin.install` | `path` | Installs a `.craftplugin` file or a bundle folder into the plugin folder. The plugin waits for approval. |
| `plugin.approve` | `id` | Grants the permissions the plugin asks for, for exactly the installed files, and starts it. Also turns back on a plugin turned off after the app closed unexpectedly |
| `plugin.setSandbox` | `id`, `sandboxed` | For a plugin with a WebAssembly and a native build: `false` runs the native build for this version, `true` runs the sandboxed one |
| `plugin.disable` | `id` | Stops the plugin and keeps it installed |
| `plugin.uninstall` | `id` | Removes the plugin and its approval |
| `plugin.load` | `path` | Runs a bundle until the app quits, with the permissions it asks for |
| `plugin.reload` | | Stops and restarts every plugin |
| `plugin.run` | `plugin`, `command` | Runs a plugin command |
| `plugin.startTask` | `plugin`, `task`, `args` | Starts a plugin task, returns its id |
| `plugin.tasks` | `wait`, `timeout` (600 s) | Every plugin task with status, progress, message, result and error. With `wait`, answers the tasks' app calls until none runs. |
| `plugin.cancelTask` | `plugin`, `id` | Cancels a task |
| `plugin.log` | `id`, `after` (0), `load` | `{load, missed, lines}`: the plugin's log lines, panics and failed calls after sequence number `after`, as `lines: [{seq, level, message}]`. The log keeps the last 512 lines and at most 64 KiB; a message is cut at 4 KiB and newlines and control characters in it are escaped (`\n`, `\u{1b}`), so one message is one row. `missed` counts lines dropped before you read them. A failed call reads `render of effect `glow` failed: …`. Each load of the plugin has its own `load` number and starts its numbers again; pass the `load` you last saw, and a cursor from an earlier load reads the new load from its start. |
| `window.plugins` | | Opens the plugin manager |
| `window.pluginPanel` | `plugin`, `panel` | Opens a plugin panel |

A script cannot run `plugin.install`, `plugin.approve`, `plugin.setSandbox`, `plugin.disable`, `plugin.uninstall`, `plugin.load` or `plugin.reload`, directly or inside `engine.batch`: the command fails with a message that sends the user to Window ▸ Plug-ins ▸ Manage Plug-ins. The UI, CLI and control channel still run them.

`keys.info {keys?}` describes the selected keyframes, and plugins with `document.read` use it. Each key has
its zero-based `index` among its property's keys (the first key is 0) and the property's `keyCount`. For a spatial property, `segmentLength`
is the length in pixels of the path from this key to the next (`null` on the last key) and `pathLength` the
length of the whole path. `dims` is the number of dimensions the layer shows (2 for scale on a 2D layer) and
`easeDims` the number `keys.setEase` works on (3); `inEase` and `outEase` have `easeDims` entries.

An automation agent confined to folders cannot run any of these except `plugin.list`.

Installed plugins live in `Plugins/` in the settings folder. `Plugins/state.json` records each approval with
the version and digest of the files approved; `plugin.list` gives the state `needs_approval` with an `error`
saying why when files changed or permissions grew, and `quarantined` for a plugin that ran unsandboxed when
the app last closed unexpectedly. `Plugins/.runs/` holds the markers of running apps. `--safe-mode` or
`EFFECTCRAFT_SAFE_MODE=1` starts the app with no plugins and no `EFFECTCRAFT_PLUGIN_PATH` bundles.
`Plugins/.data/<id>/` is a plugin's private data folder.

## Testing

`craft-plugin-host = { git = "https://github.com/arnav-hiwarkar/craft-libs-plugin-beta", rev = "…", features = ["testing"] }` as a
dev-dependency gives
`craft_plugin_host::testing`. It runs the plugin compiled into the test binary through the same host code
the app uses, with the permissions `plugin.json` asks for. Commands the plugin runs are recorded, not run.

| Call | Does |
|---|---|
| `TestPlugin::of(plugin)` | Runs the plugin that the function `plugin` (the one given to `export!`) makes, with the `plugin.json` of the crate under test. One test binary runs one plugin. |
| `assert_frame!(plugin.render("tint").set("amount", 100.0), "name")` | Renders and compares with `tests/frames/<name>.png`. By default each straight (not premultiplied) RGBA channel may differ by one level of 255; `.tolerance(0)` on the render asks for an exact match and `.tolerance(n)` allows `n` levels. `<name>` is a plain name of letters, digits, `-` and `_`; a stored frame that is a link is refused. A missing frame is stored, except when `CI` is set: then the test fails and writes the rendered frame to `target/frames/<name>.actual.png`. A frame of another size fails the same way. A different one fails, writes `<name>.actual.png` and `<name>.diff.png` under the plugin's `target/frames/` and prints their paths. Only `UPDATE_FRAMES=1` stores every rendered frame; any other value is ignored. |
| `plugin.render("tint").time_check()` | Renders at the set time and half a second later; an effect whose frames differ without `.time_dependent()` on its `EffectSpec` is an error naming that call. `cargo xtask plugin check` runs it for every effect. |
| `plugin.render(effect)` | A `Render` builder: `.set(id, value)`, `.with(json)`, `.on(Pattern)`, `.input(Picture)`, `.size(w, h)` (320x180 by default), `.layer(param, Picture)`, `.mask(points, closed)`, `.time(s)`, `.scale(f)`, `.float()`, `.tolerance(levels)`. A `.set` value passes through `Param::fit` as in the app: clamped to the hard range, and an error when its kind is wrong. |
| `render.run()`, `render.timed(rounds)` | The output `Picture` (padded by the layout's padding), and the best time. When the layout or a pass of its plan has a reach, the render in bands is also compared with a whole render, and a difference is an error naming the first pixel. |
| `render.layout()`, `render.overlay()`, `render.source()` | The effect's `Layout`, its `Overlay` (in layer pixels), and the input picture. |
| `plugin.panel(id)` | A `PanelTest`: `.click(label)`, `.send(label, UiEventKind)`, `.app_event(AppEvent)`, `.timer(name)`, `.redraw()`, `.texts()`, `.widgets()`. |
| `plugin.migrate(effect, from, values)` | Calls the effect's `migrate` with saved values and returns what it made of them. |
| `plugin.command(id)`, `plugin.task(id, args)`, `plugin.tool(id, event)` | Runs a command; runs a task to its end (60 s at most) and returns its result; sends a `ToolEvent` (`testing::pointer(kind, comp)` makes one) and returns the overlay. |
| `plugin.commands()`, `plugin.answer(command, json)`, `plugin.notes()`, `plugin.logs()` | The commands run so far with their parameters; what a command returns (null by default); the `notify` messages; the log. |
| `plugin.reply(command, \|params\| result)` | What a command returns, `Ok(json)` or `Err(message)`, made from the parameters it was called with. |
| `plugin.start(id, args)`, `plugin.cancel(task)`, `plugin.wait(task)` | Starts a task and returns its id; cancels it as the Cancel button does; waits for its result. A cancelled task gives `Err("task <id> was cancelled")`. |
| `plugin.frames(\|request\| result)` | The pictures `host::frame` returns (`Ok(picture)` or `Err(message)`), for example a sequence made from `request.time`. The request is checked first as the app checks it. |
| `plugin.data(place, key)`, `plugin.set_data(place, key, bytes)` | The plugin data a task or command wrote, and data to give it, under the app's key, size and count limits. In a render, `Place::OwnLayer` is `Place::Layer(RENDER_LAYER)`. |
| `Pattern` | `Photo`, `Gradient`, `Checker`, `Shapes`, `Text`, `Empty`; `.picture(w, h)`. |
| `Picture` | `{ width, height, pixels }` in premultiplied RGBA f32; `open(path)`, `save(path)`, `at(x, y)`, `rgba8()`. |

`host::frame` in a test returns the photo pattern at 320x180 times the requested scale unless `plugin.frames` says otherwise; layer parameters
are connected only when the render gives them a picture; `cx.scene()` is a centred camera; `cx.at` returns
the same parameters at every time.

## Developer commands

| Command | Does |
|---|---|
| `cargo xtask plugin new <name> [--kind effect\|gpu-effect\|panel\|tool\|task]` | Writes `plugins/<name>` from a template that builds and passes its tests |
| `cargo xtask plugin <dir> [--native] [--out DIR]` | Builds the bundle and the `.craftplugin` package |
| `cargo xtask plugin dev <dir> [--bridge PORT]` | Rebuilds on every save, loads the bundle into the app on the control port (9877) with `plugin.load`, and prints compile errors and the plugin's log as they happen |
| `cargo xtask plugin render <dir> …` | Renders one effect to a PNG; see [Write a plugin](plugin-sdk.md#check-render-and-benchmark) |
| `cargo xtask plugin bench <dir> [--rounds N]` | Sandboxed and native times per effect at 1080p and 4K, the ratio and the load average |
| `cargo xtask plugin check <dir>` | Reports problems with the fix for each; exits non-zero on errors |
