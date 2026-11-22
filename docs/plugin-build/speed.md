# Plugin speed before and after the third build

Measured 2026-11-23 on an Apple M5 with 10 threads. `crates/plugin-host/examples/bench.rs` was built in release
at `e097e232` (the start of the build) and at `4b0ebda1` (the end, before the last Plasma Beam change), and
the two were run alternately four times each at a load average of 5.2 to 7.3, 15 frames per size per run. Each
figure is the smallest per-run minimum in ms. The Plasma Beam rows are from a second session of five
alternating runs at a load of 4 after its last change (`3bcbc4cb`). f32 RGBA frames, CPU only.

| Effect | Size | Native before | Native after | Sandboxed before | Sandboxed after | Sandboxed / native after |
|---|---|---|---|---|---|---|
| Probe invert | 1080p | 0.29 | 0.30 | 1.12 | 1.11 | 3.70x |
| Probe invert | 4K | 1.87 | 1.89 | 5.73 | 5.77 | 3.05x |
| Plasma Beam | 1080p | 1.67 | 1.74 | 4.54 | 3.92 | 2.25x |
| Plasma Beam | 4K | 4.44 | 4.60 | 12.71 | 11.92 | 2.59x |
| Lumen Glow | 1080p | 12.28 | 13.56 | 26.15 | 13.50 | 1.00x |
| Lumen Glow | 4K | 40.37 | 41.88 | 97.55 | 41.68 | 1.00x |
| Halo Flares | 1080p | 16.43 | 15.07 | 25.51 | 21.84 | 1.45x |
| Halo Flares | 4K | 52.66 | 49.76 | 84.32 | 78.07 | 1.57x |
| Retro CRT | 1080p | 47.23 | 12.31 | 40.72 | 17.22 | 1.40x |
| Retro CRT | 4K | 192.24 | 43.32 | 130.39 | 63.42 | 1.46x |
| Retro Neon | 1080p | 54.23 | 16.69 | 36.35 | 26.71 | 1.60x |
| Retro Neon | 4K | 217.41 | 50.52 | 106.10 | 84.42 | 1.67x |

## What changed the numbers
- **Retro, native 3.3 to 4.4 times faster, sandboxed 1.3 to 2.4 times.** Native effects that read neighbours
  render in bands on all cores, and no band renders a row twice.
- **Lumen Glow in the sandbox, about twice as fast and equal to native.** Its whole render is a plan of host
  kernels that the app runs natively; nothing is copied into the sandbox.
- **Halo, 6 to 17 % faster.** In-place rows and the `math::fast` functions.
- **Probe, unchanged.** A one-line effect is bound by copying the frame into and out of the sandbox; that
  takes a frame arena shared with the sandbox, or the helper process (see [roadmap.md](roadmap.md)).
- **Slower than at the start:** native Lumen Glow at 1080p by about 9 % (it writes two more full-size images
  between kernels than its old hand-fused code did), and native Plasma Beam by about 4 %.

## Other measurements from the build
Each was taken by the worker that made the change, with before and after builds alternating; the reports in
the build notes have the conditions.

| What | Before | After |
|---|---|---|
| Lumen Glow on the GPU, 1080p | 7.5 ms with its own shaders | 7.8 ms as a plan of host kernels |
| A GPU plugin effect that reads another layer, 1080p | 22 to 30 ms | 7.3 to 7.8 ms |
| A comp of ten static layers, next frame | 100 ms | 1.0 to 1.3 ms |
| A five-effect built-in stack at 8 bpc, cycles | 1.501 G | 1.336 G |
| One layer cache key | 9.88 µs | 4.77 µs |
| `keys.info` with 1200 selected position keys | 1.99 s | 0.0145 s |
| UI frame with three plugin panels open and idle | 0.83 ms | 0.62 to 0.66 ms |

Not measured: the app's full render path under real projects, GPU effects other than Lumen Glow, memory use.
