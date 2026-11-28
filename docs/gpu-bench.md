# Measuring the GPU plugin path

The plugin kit counts the work on its side (`craft-extensions` `docs/kit/measure.md`): plugin
calls, bytes into and out of plugin memory, lowered plan steps and WGSL builds. The uploads,
readbacks and dispatches happen here, in `effectcraft-gpu`, so they are counted here too, and the
`gpu-bench` job in `.github/workflows/pluginkit.yml` fails when one rises or a frame leaves the CPU render.

## What is counted

`GpuContext::counters()` returns a `GpuCounters` snapshot and `GpuContext::reset_counters()` zeroes
it. Every context counts its own work with relaxed atomics, bumped once per call, never per pixel.

| Counter | Counted at |
|---|---|
| `uploads`, `upload_bytes` | `GpuContext::upload_image` (layers, footage, effect inputs), plugin textures, Cryptomatte planes |
| `buffer_uploads`, `buffer_upload_bytes` | `GpuContext::buffer_init` for storage, vertex and index buffers (kernel tables, plugin data) |
| `uniform_buffers`, `uniform_bytes` | `GpuContext::buffer_init` with `UNIFORM` usage: one per dispatch or draw today |
| `readbacks`, `readback_bytes` | `Enc::read_texture_with`, `Enc::read_keyed`, particle state, YUV export planes |
| `loop_flag_reads` | `Enc::read_loop_flag`: the 4-byte loop-guard flag of a plugin effect |
| `dispatches`, `draws` | `Enc::dispatch`, the display mips, Advanced 3D, particles, YUV; plugin and Advanced 3D draws |
| `compute_pipelines`, `render_pipelines`, `shader_modules` | every pipeline and WGSL module built: built-in families, plugin kernels, host kernels, plugin draws, Advanced 3D, particles, YUV |
| `plugin_passes` | each pass of a plugin GPU plan recorded (`fx_plugin::record`) |
| `plugin_layers_copied` | layer inputs of a plugin pass fitted through a pixel copy |

## The bench

`cargo run --release -p effectcraft-plugin --example gpu_bench -- [--json out.json] [--check bench/gpu-baseline.json]`

It builds the kit's probe plugin for `wasm32-unknown-unknown`, loads it into a headless session and
renders two 1920×1080, 8 bpc comps with a layer cache, as the app does:

| Case | Stack on one solid |
|---|---|
| `mixed-stack-1080p` | Fractal Noise (CPU built-in), probe `gpu-invert` (plugin WGSL), probe `host-kernels` kernel 14 (matrix and light alpha fused into one host-kernel step), probe `invert` (plugin CPU), Levels (GPU built-in) |
| `gpu-stack-1080p` | probe `gpu-invert`, probe `host-kernels` kernel 14, Levels: the all-GPU control |

Levels' gamma is keyframed, so every frame renders. Each case renders one warm-up frame, then three
warm frames. The counters are reset before each warm frame; the highest count of each counter over
the three is reported (a note says when they differed). Each warm frame is compared with the CPU
render of the same frame and must be within 1/255; the digest is an FNV-1a hash of the three GPU
frames' f32 bits, printed as information only. A case with no dispatch, or with no plugin GPU pass, fails: the GPU
declined the frame or a plugin effect fell back to the CPU.

The job runs on `ubuntu-24.04` with Mesa's lavapipe, so it never times anything; timings come from
the owner's Mac. The counters do not depend on the build profile, so the job runs the release build.

## Reading the job and updating the baseline

The job prints a table of counters per warm frame, the measured JSON, then the result. It fails
when a counter is higher than `bench/gpu-baseline.json`, the CPU and GPU frames differ by more than
1/255, a case runs no plugin GPU pass, or a case is missing on either side. A pixel digest that differs
from the baseline only prints a note: Mesa updates on the runner change the raw GPU floats. A lower counter passes with
a note: lower the baseline in the same change to keep the gain. A case marked `"pending": true`
passes and only prints. The artifact `gpu-bench-out` holds `gpu-bench-out.json`, in the
baseline's shape (`worst` is ignored by the check). To accept a run:

1. `gh run download <id> -n gpu-bench-out -R arnav-hiwarkar/effectcraft-pluginkit`
2. Copy `gpu-bench-out.json` to `bench/gpu-baseline.json`, or only the cases you mean to change.
3. Commit with the reason: which change moved the count and why the new number is right.
