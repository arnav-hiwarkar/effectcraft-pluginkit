# GPU effects

A plugin effect can run on the GPU. The plugin ships WGSL compute kernels and, for each frame, returns
a plan: a short list of passes, each naming a kernel, the images it reads and the image it writes. The
app compiles the WGSL once, runs the passes on its own textures, and never copies the frame into the
plugin's sandbox.

The GPU path runs when GPU Acceleration is on (`--gpu` on the command line). Otherwise, and whenever
the GPU declines, the app calls the effect's CPU `render`. Keep `render` and the kernels computing the
same result: the same frame can render either way, for example the viewer on the GPU and an export on
the CPU.

## The Rust side

Two things turn on the GPU path:

1. `EffectSpec::shader(wgsl)` hands the app the kernels' WGSL source.
2. `layout()` returns `Layout::...gpu(plan)` with the plan for this frame.

`layout()` runs for every frame, so a plan can depend on parameters and frame size. Return a layout
without `.gpu(..)` to render this frame on the CPU.

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

The frame the passes see is the padded frame: `cx.size` plus `2 * padding` in each axis when the layout
has `.padded(padding)`. Read sizes in the kernel with `out_size()` and `src_size()` instead of assuming
them.

Values reach a pass through `.floats(&[..])` (up to 48) and `.uints(&[..])` (up to 16). A kernel reads
value `i` with `pass_f(i)` or `pass_u(i)`; values the pass did not give read as 0. Scale distances by
`cx.scale` as the CPU render does.

### Limits

The app checks every plan before running it and renders the frame on the CPU when one fails:

- 1 to 64 passes;
- at most 16 scratch images, each at least 1×1 and at most twice the padded frame in each axis;
- a pass reads only `Source` or an image an earlier pass wrote, never the image it writes;
- no pass writes `Source`, and some pass writes `Output`;
- at most 48 floats and 16 uints per pass.

## The WGSL side

The app compiles your WGSL after a prelude that declares the bindings every kernel gets:

| Name | Binding | Meaning |
|---|---|---|
| `src` | `texture_2d<f32>` | The pass's `src` image. |
| `aux` | `texture_2d<f32>` | The pass's `aux` image; a 1×1 transparent image when the pass has none. |
| `out` | `texture_storage_2d<rgba32float, write>` | The pass's output image. |
| `P` | uniform | The pass's values; read them with `pass_f` / `pass_u`. |
| `data` | storage buffer | Reserved. |

Do not declare these again, and declare no other bindings: the shader then fails to compile.

Pixels are premultiplied RGBA, 32-bit float, in the project's working colour space: the same values
your CPU `render` sees in `frame.pixels`. Pixel `(0, 0)` is the top-left corner.

Each pass runs its kernel over `out` in 16×16 workgroups. Declare `@compute @workgroup_size(16, 16)`,
return early when `inside_out(id.xy)` is false, and write every pixel of `out`: a pixel the kernel
skips holds undefined values.

These prelude functions are stable:

| Function | Returns |
|---|---|
| `out_size()`, `src_size()`, `aux_size()` | The image's size in pixels, `vec2<u32>`. |
| `inside_out(p)` | Whether pixel `p` lies inside `out`. |
| `pass_f(i)`, `pass_u(i)` | Value `i` of the pass's floats or uints. |
| `texel(t, p)` | Pixel `p: vec2<i32>` of `src` or `aux`; transparent outside the image. |
| `out_uv(p)` | The centre of output pixel `p` in 0..1 coordinates. |
| `sample_uv(t, uv)` | `t` sampled bilinearly at 0..1 coordinates, edge pixels repeated. |
| `store(p, colour)` | Writes `colour` to output pixel `p`. |

0..1 coordinates name the same spot in images of any size, so `sample_uv(aux, out_uv(id.xy))` reads a
half-size scratch image under the current output pixel.

The prelude declares other helpers that may change. Prefix your own function names, for example with
the effect's name, so they cannot collide with them.

When the WGSL fails to compile, or a kernel a pass names does not exist, the app logs the error once and
renders the effect on the CPU until the plugin is reloaded. The log quotes line numbers in the prelude
and your source together.

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
        for pixel in frame.pixels.iter_mut() {
            pixel[0] *= r;
            pixel[1] *= g;
            pixel[2] *= b;
        }
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
        let (w, h) = (frame.width, frame.height);
        let (hw, hh) = (w.div_ceil(2), h.div_ceil(2));
        let at = |x: usize, y: usize| if x < w && y < h { frame.pixels[y * w + x] } else { [0.0; 4] };
        let mut half = vec![[0.0f32; 4]; hw * hh];
        for y in 0..hh {
            for x in 0..hw {
                let block = [at(2 * x, 2 * y), at(2 * x + 1, 2 * y), at(2 * x, 2 * y + 1), at(2 * x + 1, 2 * y + 1)];
                half[y * hw + x] = std::array::from_fn(|c| (block[0][c] + block[1][c] + block[2][c] + block[3][c]) * 0.25);
            }
        }
        let clamped = |x: i64, y: i64| half[y.clamp(0, hh as i64 - 1) as usize * hw + x.clamp(0, hw as i64 - 1) as usize];
        let strength = cx.f("strength") as f32;
        for y in 0..h {
            for x in 0..w {
                let fx = (x as f32 + 0.5) / w as f32 * hw as f32 - 0.5;
                let fy = (y as f32 + 0.5) / h as f32 * hh as f32 - 0.5;
                let (tx, ty) = (fx - fx.floor(), fy - fy.floor());
                let (xi, yi) = (fx.floor() as i64, fy.floor() as i64);
                let (a, b, c, d) = (clamped(xi, yi), clamped(xi + 1, yi), clamped(xi, yi + 1), clamped(xi + 1, yi + 1));
                let pixel = &mut frame.pixels[y * w + x];
                for k in 0..4 {
                    let top = a[k] + (b[k] - a[k]) * tx;
                    let bottom = c[k] + (d[k] - c[k]) * tx;
                    pixel[k] += (top + (bottom - top) * ty - pixel[k]) * strength;
                }
            }
        }
        Ok(())
    }
}
```

`render` gets the padded frame too: `frame.width` is `cx.size[0] + 2 * PADDING`.

The probe plugin in `crates/plugin-host/tests/probe` carries both effects with CPU renders that match
their kernels, and `crates/plugin/tests/it/gpu.rs` checks that the two paths agree.
