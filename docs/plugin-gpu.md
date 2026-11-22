# Passes: one pipeline for the GPU and the CPU

An effect that needs the whole frame (a wide glow, a pyramid blur, anything with several steps) describes
its work as a plan: scratch images plus a short list of passes, each naming a kernel, the images it reads
and the image it writes. The app owns the images and runs the passes in order. A GPU pass can
also draw triangles, with a depth buffer, see [Draw passes](#draw-passes); a plan with draw passes, buffers,
textures or layers runs only on the GPU.

- On the GPU the kernels are WGSL compute shaders. The app compiles them once, runs the passes on its own
  textures, and never copies the frame into the plugin's sandbox.
- On the CPU the kernels are the effect's `pass` method. The app splits each pass into horizontal bands,
  runs them on all cores, and gives each sandboxed instance only the source rows its band reads.
- A pass can instead name a [host kernel](#host-kernels): a blur, resample, blend, curve, convolution,
  warp, colour matrix, sRGB transfer, brightness curve, dither or light alpha that the app runs itself,
  natively on the CPU and with its own WGSL on the GPU.

The GPU runs the plan when GPU Acceleration is on (`--gpu` on the command line). Otherwise, and whenever
the GPU declines, the app runs the plan on the CPU if the layout says it can (`Layout::passes`), and calls
the effect's `render` if not (`Layout::gpu`). Keep the WGSL kernels and the CPU code computing the same
result: the same frame can render either way, for example the viewer on the GPU and an export on the CPU.

## The Rust side

Three things make up a pass pipeline:

1. `layout()` returns the plan for this frame: `Layout::...passes(plan)` for a plan that runs on the GPU
   or the CPU, `Layout::...gpu(plan)` for a plan only the GPU runs.
2. `EffectSpec::shader(wgsl)` hands the app the kernels' WGSL source for the GPU.
3. `Effect::pass` computes one band of one pass on the CPU. See [Passes on the CPU](#passes-on-the-cpu).

`layout()` runs for every frame, so a plan can depend on parameters and frame size. Return a layout
without a plan to render this frame with `render`.

```rust
let mut plan = GpuPlan::new();
let half = plan.scratch([w, h]);
let plan = plan
    .pass(GpuPass::new("down", GpuImage::Source, half))
    .pass(GpuPass::new("up", GpuImage::Source, GpuImage::Output).aux(half).floats(&[strength]));
Layout::whole_frame().gpu(plan)
```

A pass reads `src` and, optionally, `aux`, and writes `out`. Each names one of:

| Image | What it is |
|---|---|
| `GpuImage::Source` | The effect's input. Read only. |
| `GpuImage::Output` | The effect's result, the size of the frame. The last pass to write it gives the result. |
| `plan.scratch([w, h])` | A working image of that size, declared on the plan. |
| `plan.texture(key, [w, h])` | A picture the plugin stored with `host::cache_put(key, bytes)`: RGBA, four little-endian `f32` for each pixel, row 0 at the top. Read only. |
| `plan.layer(param)` | The layer chosen in layer parameter `param`, resampled onto the padded frame: the same pixels `cx.layer(param)` gives `render`. The size of the frame. Transparent when no layer is chosen or it has nothing to show. Read only. The app binds the texture it already keeps for that layer and resamples it on the GPU, so an unchanged layer is neither rendered nor uploaded again. |

`layout` runs before the frame is padded, so there `cx.size`, `cx.offset` and point parameters describe
the frame without padding. The passes run on the padded frame. Build the plan from `cx.padded(padding)`,
which is the context `render` gets: its `size` includes the padding and its `offset` and points are
moved by it. Read sizes in the kernel with `out_size()` and `src_size()` instead of assuming them.

Values reach a pass through `.floats(&[..])` (up to 96) and `.uints(&[..])` (up to 16), on the GPU and the CPU alike. A kernel reads
value `i` with `pass_f(i)` or `pass_u(i)`; values the pass did not give read as 0. Scale distances by
`cx.scale` as the CPU render does.

### Limits

The app checks every plan with `GpuPlan::validate` before running it, on the GPU and on the CPU, and a
plan that fails does not run: the effect reports the reason. The GPU then checks only what depends on the
device and the stored bytes (image sizes the GPU holds, buffer and texture byte counts, indices):

- 1 to 128 passes;
- at most 32 scratch images, each at least 1×1 and at most twice the padded frame in each axis, and
  together at most 8 times the padded frame's memory, counting the images the passes work in: one
  for each size that a blur or separable convolve writes, one for each size of a resample's
  intermediate (the output's width by the source's height), and for each size a draw writes a colour
  target of half an image and, with depth, a depth buffer of a quarter;
- a pass reads only `Source` or an image an earlier pass wrote, and never the image it writes unless it
  is a [pointwise host kernel](#host-kernels);
- no pass writes `Source` or a texture, and some pass writes `Output`;
- at most 96 floats and 16 uints per pass;
- the limits of each [host kernel](#host-kernels);
- at most 16 textures and layers together, each texture as many bytes as its size needs, together at
  most 16 777 216 pixels (a layer counts as one padded frame);
- the limits on draw passes below.

### Time

A plugin's GPU work has no time limit of its own. A GPU dispatch cannot be cancelled, so a kernel that runs
for seconds stalls the GPU for as long as that frame takes, and every other effect waits for it. The
operating system's GPU watchdog and the app's 10 second readback deadline are the only limits.

## Draw passes

A draw pass rasterises triangles into its `out` image instead of running a compute kernel.

```rust
let mut plan = GpuPlan::new();
let mesh = plan.buffer(GpuData::Cache("my-mesh/vertices".into()));
let triangles = plan.buffer(GpuData::Cache("my-mesh/indices".into()));
let copies = plan.buffer(GpuData::Floats(instance_matrices));
let draw = GpuPass::draw("mesh", GpuImage::Output)
    .vertices(mesh, &[3, 3, 2])
    .indices(triangles)
    .instances(copies, &[4, 4, 4, 4])
    .depth()
    .clear()
    .blend(Blend::Over)
    .floats(&view_projection);
```

A buffer is one of:

| Data | What it is |
|---|---|
| `GpuData::Cache(key)` | Bytes the plugin stored with `host::cache_put(key, bytes)`: little-endian `f32` for vertices and instances, `u32` for indices. Store a mesh once and name it in every frame's plan; the app uploads it to the GPU again only when the bytes change. The cache drops its least recently used entries when it is full, so check with `host::cache_has(key)` while building the plan and store the bytes again when it returns `false`. |
| `GpuData::Floats(values)` | Values sent with the plan. Right for a few hundred numbers that change each frame. |
| `GpuData::Indices(values)` | Indices sent with the plan. |

| Call | Meaning |
|---|---|
| `.vertices(buffer, &[3, 3, 2])` | The vertex buffer and how many floats each attribute has. The attributes arrive at `@location(0)`, `@location(1)` and so on. |
| `.indices(buffer)` | Draw indexed triangles. Without it, every three vertices are a triangle. |
| `.instances(buffer, &[4, 4, 4, 4])` | Draw one copy for each entry of this buffer. Its attributes take the locations after the vertex attributes. |
| `.count(n)` | Draw only the first `n` indices, or vertices when there are no indices. |
| `.depth()` | Test and write the plan's depth buffer. A fragment is kept when its depth is greater than or equal to what is there, so nearer means larger: 1 at the near plane and 0 far away. `Scene::view_projection` gives this depth. |
| `.blend(Blend::Over)` | How a fragment meets what is there: `Replace` (the default), `Over` or `Add`. Colours are premultiplied. |
| `.cull(Cull::Back)` | Skip triangles facing away. Counter-clockwise triangles in clip space face front. The default draws both sides. |
| `.clear()` | Start from a transparent image. Without it the pass draws over what `out` holds. |
| `.src(image)`, `.aux(image)` | Images the shaders can read, as in a compute pass. `src` is `Source` unless you set it. |

A draw pass with the kernel name `mesh` runs the entry points `vs_mesh` and `fs_mesh` of the effect's WGSL.
They read `P`, `src` and `aux` through the same prelude functions as a compute kernel (`pass_f`, `pass_u`,
`texel`, `sample_uv`, `src_size`, `aux_size`). They cannot use `out`, `out_size`, `inside_out`, `out_uv` or
`store`; a fragment shader returns its colour at `@location(0)`.

```wgsl
struct MeshVertex {
    @builtin(position) clip: vec4<f32>,
    @location(0) normal: vec3<f32>,
}

@vertex
fn vs_mesh(@location(0) position: vec3<f32>, @location(1) normal: vec3<f32>,
           @location(3) m0: vec4<f32>, @location(4) m1: vec4<f32>, @location(5) m2: vec4<f32>, @location(6) m3: vec4<f32>) -> MeshVertex {
    let model = mat4x4<f32>(m0, m1, m2, m3);
    let camera = mat4x4<f32>(
        vec4<f32>(pass_f(0u), pass_f(1u), pass_f(2u), pass_f(3u)), vec4<f32>(pass_f(4u), pass_f(5u), pass_f(6u), pass_f(7u)),
        vec4<f32>(pass_f(8u), pass_f(9u), pass_f(10u), pass_f(11u)), vec4<f32>(pass_f(12u), pass_f(13u), pass_f(14u), pass_f(15u)));
    var v: MeshVertex;
    v.clip = camera * model * vec4<f32>(position, 1.0);
    v.normal = (model * vec4<f32>(normal, 0.0)).xyz;
    return v;
}

@fragment
fn fs_mesh(v: MeshVertex) -> @location(0) vec4<f32> {
    let lit = max(dot(normalize(v.normal), vec3<f32>(0.0, 0.0, -1.0)), 0.0);
    return vec4<f32>(vec3<f32>(lit), 1.0);
}
```

A shader may leave out an attribute the layout declares, as this one leaves out the texture coordinate at
location 2.

The plan has one depth buffer. The first pass with `.depth()` clears it, and every pass that uses it must
draw into images of one size. Draw passes render in 16-bit float and the result is copied into the working
image, so a colour above 65504 is clipped. Passes that follow each other into the same image, without
`.clear()`, share one render target with no copy in between.

To match the comp's 3D layers, declare `.reads_scene()` and pass `cx.scene()?.view_projection(cx)` to the
vertex shader. Model positions are then layer pixels, see [Scene](plugin-reference.md#scene).

The app checks draw passes with the rest of the plan:

- at most 64 buffers, each a whole number of 4-byte values, together at most 256 MiB;
- a buffer has 1 to 8 attributes of 1 to 4 floats, and holds one or more whole vertices or instances;
- every index a pass draws is below its vertex count, and `count` is within the buffer;
- every buffer and texture a pass names is declared, and its cache key holds bytes.

When a plan with draw passes is declined, the app calls `render`. An effect that cannot draw the same
picture on the CPU should leave the frame as it is.

## Passes on the CPU

`Layout::...passes(plan)` marks the plan as one the CPU can run. Give each pass a reach and implement
`Effect::pass`:

```rust
fn pass(&self, cx: &Cx, pass: &Pass, out: &mut Frame) -> Result<()> {
    match pass.kernel {
        "down" => {
            for (y, row) in out.rows_mut() {
                for (x, pixel) in row.iter_mut().enumerate() {
                    let (x, y) = (2 * x as isize, 2 * y as isize);
                    let block = [pass.src.at(x, y), pass.src.at(x + 1, y), pass.src.at(x, y + 1), pass.src.at(x + 1, y + 1)];
                    *pixel = std::array::from_fn(|c| block.iter().map(|p| p[c]).sum::<f32>() * 0.25);
                }
            }
            Ok(())
        }
        other => Err(format!("no kernel `{other}`").into()),
    }
}
```

| In `pass` | Meaning |
|---|---|
| `pass.kernel`, `pass.f`, `pass.u` | The pass's kernel name and numbers, as given in the plan. |
| `pass.src`, `pass.aux` | The images the pass reads (`Rows`). `width` and `height` are the whole image; `at(x, y)` and `row(y)` take whole-image coordinates. A pixel outside the image, or in a row this call was not given, reads as transparent. |
| `out` | The band to fill. Write every pixel: what `out` holds before the call is not defined. `out.rows_mut()` yields `(y, row)` with `y` in the whole output image; `out.origin[1]` is the band's first row. |
| `pass.out_size` | The whole output image's width and height. |

`GpuPass::reach(n)` says how many source rows above and below the matching source row an output row
reads. `reach(0)` is pointwise. The source and the output of a pass may differ in size: the matching
source row of output row `y` is `y * src_height / out_height`, rounded down, and the same reach applies
to `aux` with its own height. `GpuPass::aux_reach(n)` gives `aux` a reach of its own, for example
`.reach(0).aux_reach(2)` for a pass that reads its source pointwise and a half-size `aux` two rows
around. A band gets the source rows matching its first and last output rows, widened by the reach. Rows outside that read as transparent, so a reach that is too small shows as dark lines at
band edges; when sizes are odd the matching row can be one less than `2 * y` or `y / 2` suggests, so
allow one row more. A pass without a reach needs its whole source and runs as one call on one core.

A plugin pass never reads the image it writes: the plan is refused. A pass that returns an error fails the
render, and the layer keeps its frame. A native plugin runs the same bands directly on the app's images with no copies. The default
`pass` returns an error naming the kernel, so an effect with `Layout::gpu` plans needs none.

## Host kernels

A host kernel pass names a standard step instead of plugin code. The app runs it on its own images: on
the CPU natively on all cores with nothing copied into the sandbox, on the GPU with the app's WGSL. Both
give the same result within 2/255, so a plan of host kernels needs no `pass` method and no WGSL of its
own. Mix host kernel passes and plugin passes freely in one plan.

| Builder | What it computes |
|---|---|
| `GpuPass::blur(src, out, [rx, ry])` | Gaussian blur reaching `rx` and `ry` pixels (standard deviation a third of the radius), 0 to 512. |
| `GpuPass::box_blur(src, out, [rx, ry])` | Box blur of that radius; a fractional radius weighs the outer pixels by its fraction. |
| `GpuPass::resample(src, out, Filter::Bilinear)` | `src` scaled to the size of `out`, `Filter::Bilinear` or `Filter::Bicubic` (Catmull-Rom). Shrinking widens the filter, so every source pixel counts. |
| `GpuPass::blend_images(base, top, out, mode, amount)` | `top` over `base` in a `BlendMode`, mixed by `amount` (0 keeps `base`). |
| `GpuPass::curves(src, out, &table)` | A lookup table per channel: entry `i` of `n` holds the output for input `i / (n - 1)`, red, green, blue and alpha, 2 to 4096 entries. |
| `GpuPass::convolve(src, out, Taps::Separable { x, y })` | A separable kernel, odd tap counts up to 1025, centred on the pixel. |
| `GpuPass::convolve(src, out, Taps::Full { width, weights })` | A full kernel, odd sides up to 31, row by row. |
| `GpuPass::warp(src, displacement, out, [ax, ay])` | Each pixel read from `src` at its position plus `ax` times the red and `ay` times the green of `displacement`, bilinear. |
| `GpuPass::matrix(src, out, rows)` | A 4×4 colour matrix: output channel `i` is `rows[i]` dotted with the input pixel. |
| `GpuPass::to_linear(src, out)`, `GpuPass::to_srgb(src, out)` | The sRGB transfer curve on red, green and blue as they are (no division by alpha), alpha kept. Negative values mirror around zero and values above 1 continue the curve. |
| `GpuPass::brightness_curve(src, out, measure, curve, range)` | Each pixel's colour scaled so that its brightness `m` becomes `curve(m)`, hue and alpha kept. `measure` is `Measure::MaxChannel` (the largest of red, green and blue) or `Measure::Luma` (Rec. 709). Entry `i` of `n` holds the output for `m = range * i / (n - 1)`; past `range` the last segment continues. A pixel with no brightness becomes black. Threshold, soft knee and highlight or shadow selection are curves. |
| `GpuPass::dither(src, out, seed, origin)` | Triangular noise of at most one 8-bit level on red, green and blue, the same for a seed and a pixel position minus `origin` on the CPU and the GPU. Values at or beyond 0 and 1 are left exact. |
| `GpuPass::light_alpha(src, out)` | Alpha raised to the brightest colour channel, at most 1, so light added onto transparent pixels stays visible as valid premultiplied colour. |
| `GpuPass::host(HostKernel::..., src, out)` | Any kernel with all its fields, for example `HostKernel::Resample { filter, scale: Some([2.0, 2.0]) }` to fix the source pixels per output pixel instead of taking it from the sizes. |

Rules shared by all of them:

- Pixels outside an image read as transparent, as for plugin passes.
- `blur`, `convolve`, `curves`, `matrix`, `blend_images` and `warp` write an image of their source's size,
  and `blend_images` and `warp` an `aux` of that size too. `resample` maps any size to any size, shrinking
  at most 64 times in one pass.
- The `BlendMode`s are `Over` (premultiplied over), `Add`, `Subtract`, `Multiply`, `Screen`, `Overlay`,
  `Darken`, `Lighten` and `Difference`. All but `Over` work channel by channel, alpha included.
- `curves` works on straight colour: it divides by alpha, looks the colour up, and multiplies by the
  alpha it looked up. Inputs outside 0 to 1 take the end entries. `matrix` works on the premultiplied
  values as they are.
- A pass's `floats`, `uints` and `reach` are ignored; the app knows what each kernel reads.
- `blend_images`, `curves`, `matrix`, `to_linear`, `to_srgb`, `brightness_curve`, `dither` and
  `light_alpha` are pointwise: each output pixel depends only on the same pixel of its inputs. A pointwise
  pass may write the image it reads, as in `GpuPass::matrix(work, work, rows)`.
- Consecutive pointwise passes of one size run as one loop over the frame on the CPU and one shader on the
  GPU. An image they write that no later pass reads is never stored. The loop ends before a pass that would
  need a third stored image, and after a pass whose image a later, non-fused pass reads. On the GPU a
  `resample` whose image only the next such loop reads is sampled inside that loop's shader instead of
  being stored.

`plan.validate` checks each kernel's fields and names the pass and the field it refuses:
`pass 2 (blur): `radius` is [600.0, 2.0]; each axis must be 0 to 512`.

An effect with no WGSL runs a plan on the GPU too when that plan is made only of host kernels. The app
decides per frame: at settings where the layout gives no plan, or a plan naming the effect's own kernels,
the frame renders on the CPU. Once the effect has given such a plan (at the default settings when the
plugin loads, or at any later frame), the app offers it to the GPU; until then it renders on the CPU, so
when the defaults give no such plan, the first frame at settings that do renders on the CPU. An effect whose plans name its own kernels gives WGSL with `.shader(..)`.

Lumen Glow (`plugins/lumen-glow`) is made only of host kernels and has no WGSL and no `pass` method:
`to_linear` and a `brightness_curve` soft knee, `resample` passes down a pyramid, a separable `convolve`,
`matrix` and `blend_images` per level, and one fused composite of `to_linear`, `matrix`,
`brightness_curve`, `blend_images`, `to_srgb`, `dither` and `light_alpha`.

## The WGSL side

The app compiles your WGSL after a prelude that declares the bindings every kernel gets:

| Name | Binding | Meaning |
|---|---|---|
| `src` | `texture_2d<f32>` | The pass's `src` image. |
| `aux` | `texture_2d<f32>` | The pass's `aux` image; a 1×1 transparent image when the pass has none. |
| `out` | `texture_storage_2d<rgba32float, write>` | The pass's output image. |
| `P` | uniform | The pass's values; read them with `pass_f` / `pass_u`. |
| `data` | storage buffer | Reserved for the app. |

Do not declare these again, and declare no other bindings: the shader then fails to compile.

Pixels are premultiplied RGBA, 32-bit float, in the project's working colour space: the same values
your CPU `render` reads from `frame.input`. Pixel `(0, 0)` is the top-left corner.

Each pass runs its kernel over `out` in 16×16 workgroups. Declare `@compute @workgroup_size(16, 16)`,
and return early when `inside_out(id.xy)` is false. A pixel of `out` the kernel does not write is
transparent.

These prelude functions are stable:

| Function | Returns |
|---|---|
| `out_size()`, `src_size()`, `aux_size()` | The image's size in pixels, `vec2<u32>`. |
| `inside_out(p)` | Whether pixel `p` lies inside `out`. |
| `pass_f(i)`, `pass_u(i)` | Value `i` of the pass's floats (0 to 95) or uints (0 to 15). |
| `texel(t, p)` | Pixel `p: vec2<i32>` of `src` or `aux`; transparent outside the image. |
| `out_uv(p)` | The centre of output pixel `p` in 0..1 coordinates. |
| `sample_uv(t, uv)` | `t` sampled bilinearly at 0..1 coordinates, edge pixels repeated. |
| `store(p, colour)` | Writes `colour` to output pixel `p`. |

0..1 coordinates name the same spot in images of any size, so `sample_uv(aux, out_uv(id.xy))` reads a
half-size scratch image under the current output pixel.

The prelude declares other helpers that may change, with names that start with `craft_`. Prefix your own
function names, for example with the effect's name, so they cannot collide with them. The host kernels are
compiled in a module of their own, so a plain pass cannot name one; use the [host kernel](#host-kernels)
builders instead.

When the WGSL fails to compile, or a kernel a pass names does not exist, the app logs the error once and
renders the effect on the CPU until the plugin is reloaded. The log quotes line numbers of your source,
as `wgsl:LINE:COLUMN`; a line of the prelude shows as `prelude LINE`.

## Example: tint

One pass, with the source as `src` and the output as `out`.

```rust
use craft_plugin::prelude::*;

const KERNELS: &str = r#"
@compute @workgroup_size(16, 16)
fn tint_apply(@builtin(global_invocation_id) id: vec3<u32>) {
    if (!inside_out(id.xy)) { return; }
    let c = texel(src, vec2<i32>(id.xy));
    let tint = vec3<f32>(pass_f(0u), pass_f(1u), pass_f(2u));
    store(id.xy, vec4<f32>(c.rgb * tint, c.a));
}
"#;

struct Tint;

impl Effect for Tint {
    fn spec(&self) -> EffectSpec {
        EffectSpec::new("tint", "Tint", "Color Correction").param(Param::color("color", "Color", [1.0, 0.8, 0.6, 1.0])).shader(KERNELS)
    }

    fn layout(&self, cx: &Cx) -> Layout {
        let [r, g, b, _] = cx.color("color");
        Layout::pointwise().gpu(GpuPlan::new().pass(GpuPass::new("tint_apply", GpuImage::Source, GpuImage::Output).floats(&[r, g, b])))
    }

    fn render(&self, cx: &Cx, frame: &mut Frame) -> Result<()> {
        let [r, g, b, _] = cx.color("color");
        frame.map(|p| [p[0] * r, p[1] * g, p[2] * b, p[3]]);
        Ok(())
    }
}
```

## Example: soften through a half-size image

The first pass averages each 2×2 block of the frame into a half-size scratch image. The second blends
the frame towards that image, sampled back up to full size. Padding gives the softened edge room.

```rust
use craft_plugin::prelude::*;

const KERNELS: &str = r#"
@compute @workgroup_size(16, 16)
fn soften_down(@builtin(global_invocation_id) id: vec3<u32>) {
    if (!inside_out(id.xy)) { return; }
    let p = vec2<i32>(id.xy) * 2;
    let sum = texel(src, p) + texel(src, p + vec2<i32>(1, 0)) + texel(src, p + vec2<i32>(0, 1)) + texel(src, p + vec2<i32>(1, 1));
    store(id.xy, sum * 0.25);
}

@compute @workgroup_size(16, 16)
fn soften_up(@builtin(global_invocation_id) id: vec3<u32>) {
    if (!inside_out(id.xy)) { return; }
    let c = texel(src, vec2<i32>(id.xy));
    let soft = sample_uv(aux, out_uv(id.xy));
    store(id.xy, c + (soft - c) * pass_f(0u));
}
"#;

const PADDING: u32 = 4;

struct Soften;

impl Effect for Soften {
    fn spec(&self) -> EffectSpec {
        EffectSpec::new("soften", "Soften", "Blur & Sharpen").param(Param::slider("strength", "Strength", 0.75, 0.0..=1.0)).shader(KERNELS)
    }

    fn layout(&self, cx: &Cx) -> Layout {
        let frame = [cx.size[0] + 2 * PADDING, cx.size[1] + 2 * PADDING];
        let mut plan = GpuPlan::new();
        let half = plan.scratch([frame[0].div_ceil(2), frame[1].div_ceil(2)]);
        let plan = plan
            .pass(GpuPass::new("soften_down", GpuImage::Source, half))
            .pass(GpuPass::new("soften_up", GpuImage::Source, GpuImage::Output).aux(half).floats(&[cx.f("strength") as f32]));
        Layout::whole_frame().padded(PADDING).gpu(plan)
    }

    fn render(&self, cx: &Cx, frame: &mut Frame) -> Result<()> {
        let (w, h, input) = (frame.input.width, frame.input.height, frame.input);
        let (hw, hh) = (w.div_ceil(2), h.div_ceil(2));
        let at = |x: usize, y: usize| input.at(x as isize, y as isize);
        let mut half = vec![[0.0f32; 4]; hw * hh];
        for y in 0..hh {
            for x in 0..hw {
                let block = [at(2 * x, 2 * y), at(2 * x + 1, 2 * y), at(2 * x, 2 * y + 1), at(2 * x + 1, 2 * y + 1)];
                half[y * hw + x] = std::array::from_fn(|c| (block[0][c] + block[1][c] + block[2][c] + block[3][c]) * 0.25);
            }
        }
        let clamped = |x: i64, y: i64| half[y.clamp(0, hh as i64 - 1) as usize * hw + x.clamp(0, hw as i64 - 1) as usize];
        let strength = cx.f("strength") as f32;
        let x0 = frame.origin[0];
        for (y, read, row) in frame.rows() {
            for (x, (pixel, source)) in (x0..).zip(row.iter_mut().zip(read)) {
                let fx = (x as f32 + 0.5) / w as f32 * hw as f32 - 0.5;
                let fy = (y as f32 + 0.5) / h as f32 * hh as f32 - 0.5;
                let (tx, ty) = (fx - fx.floor(), fy - fy.floor());
                let (xi, yi) = (fx.floor() as i64, fy.floor() as i64);
                let (a, b, c, d) = (clamped(xi, yi), clamped(xi + 1, yi), clamped(xi, yi + 1), clamped(xi + 1, yi + 1));
                for k in 0..4 {
                    let top = a[k] + (b[k] - a[k]) * tx;
                    let bottom = c[k] + (d[k] - c[k]) * tx;
                    pixel[k] = source[k] + (top + (bottom - top) * ty - source[k]) * strength;
                }
            }
        }
        Ok(())
    }
}
```

`render` gets the padded frame too. There `cx.size` already includes the padding, so `frame.input.width`
equals `cx.size[0]`. A whole-frame effect still reads every input row, but writes only the rectangle that
shows, which may be smaller than the frame.

The probe plugin in `crates/plugin-host/tests/probe` carries both effects with CPU renders that match
their kernels, and `crates/plugin/tests/it/gpu.rs` checks that the two paths agree. `plugins/forge-3d` is a
whole effect built on draw passes: indexed meshes from the cache, instancing, depth, and the comp's camera
and lights.
