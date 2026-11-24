# Known issues of the plugin system

Found while the plugin system was built and reviewed, and not fixed. Each entry says where to look.
[decisions.tsv](decisions.tsv) is the log of what was decided and how each step was checked.
[roadmap.md](roadmap.md) lists what the third build did and what was put off; [speed.md](speed.md) has the
measured speed before and after.

- **wasmtime and libloading licences.** House rule 26 wants every new dependency's licence on `standards/licences.md` in craft-contrib. wasmtime is Apache-2.0 WITH LLVM-exception, libloading is ISC. I did not check the list (it is in a storytold repo). Check before any upstream proposal.
- **`files.read` and `files.write` on Windows.** The plugin gate widens a plugin's roots to the filesystem root. On Windows that is the current drive only, so a plugin with `files.write` cannot write to another drive. Conservative, not a hole.
- **House rule 10 (git dependencies only from storytold) and rule 9 (shared code in craft-libs).** The two kit crates live in this repository so the fork builds on its own. Moving them to craft-libs is a plain move when wanted.
- **Plugin files outside the plugin's folders are not in the cache key.** An effect that reads a file through `files.read` keeps its cached frames when that file changes. The key holds only the plugin's own folders, which can be listed cheaply.
- **Possibly dead branch.** `show_result_errors` in `crates/ui-egui/src/menus/frontend.rs` still formats `{file, error}` items, a shape only the removed bundle install produced. The test near `menus/tests.rs:900` uses the removed command id `effect.installBundle` as a fixture.
- **ScriptUI panel bundles.** The removed `Install Effect Plug-in Bundle (Folder)` command also copied `.jsx` panels. Single panels still install through `file.installScriptUIPanel`.

- **Viewer tools get no pen pressure.** egui reports no pressure for a pen, so `ToolEvent::pressure` is 1 while the button is down and 0 otherwise.
- **Built-in point crosshairs on an adjustment layer are drawn in layer space.** The effect works on the comp below, so a moved adjustment layer shows them off the point the effect uses. Plugin handles use comp space there (`crates/ui-egui/src/panels/effect_controls/viewer.rs`, `from_effect_space`).
- **One slow interactive call delays the plugin's other surfaces.** Panels, custom controls, overlays, tools and menu commands of one plugin share its worker queue, so a panel call that runs for a second holds back that plugin's overlay for that second (`crates/plugin/src/panels.rs`).
- **Task host requests are answered by whichever session pumps first.** `effectcraft_plugin::pump` serves every task of a plugin; panel requests are routed by their project token, task requests only check that their project is still open (`crates/plugin/src/manage.rs`).

## From the Sol review, not fixed

- **A slow host call is not interrupted.** A file call on a stalled disk or a slow app command runs to its end on the calling thread; the sandboxed call fails when it returns past the limit. A panel's `frame` and `audio` are cut from the project on the app's thread and rendered on the plugin's worker; a task's still render on the app's thread when the task pump answers them. A file write cannot be cancelled, so it is not moved to another thread and abandoned: that let a write land after the plugin was told it failed, and stalled threads piled up. Stopping one needs the helper process (`docs/plugin-helper.md`), where the whole process can be killed. Instantiation has its own 5 s limit on top of the call limit.
- **A dangling link is also possible in the automation confinement.** `crates/automation/src/confine.rs` falls back to the parent folder when a path does not resolve. This is upstream code and affects agents too.

## Reported by the flagship plugin authors, not fixed

- **Engine: `keys.setEase` changes the side you did not set.** On a continuous key it copies the speed to the other side, and it turns off auto-Bezier so the other side falls back to a stored ease. Glide works around both. (`crates/engine/src/cmd/anim.rs`)
- **Engine: influence cannot go below 0.1 %.** A curve handle exactly on its anchor's column is stored about 6.6e-4 of the value range off.
- **GPU: on WebGPU a bad plugin shader is not contained.**
- **GPU: a plugin kernel that runs for seconds stalls the GPU for as long as that frame takes.** A wgpu dispatch cannot be cancelled and plugin GPU work has no time limit of its own; the OS GPU watchdog and the 10 s readback deadline are the only limits, and every other GPU effect waits meanwhile. A time limit was removed on 2026-11-22 because a wall clock around a shared queue kept charging the wrong plugin. The cure is per-plan GPU timestamps where the device has them, or running plugin GPU work in the helper process.

## From the Opus review, not fixed

- **Background jobs** (`camera.analyze`, `layer.sceneEditDetection`) apply their result later, outside the plugin's batch and its undo step. Plugins can start them but cannot wait for them.
- **Glide: `keys.interpolation` runs on the whole selection**, so keys on properties Glide reports as skipped still lose their continuous and roving flags. (`plugins/glide/src/apply.rs`)

## From the second build and its reviews, not fixed

- **A process of the same user that reads the cache key can still sign a compiled module.** Compiled cache entries carry an HMAC-SHA-256 under `Plugins/.compiled/key` (mode 0600, made from OS randomness), so a planted entry without the key is compiled again (`cached` in craft-libs `crates/plugin-host/src/runtime/wasm.rs`). The key file is readable by every process of the same user; only the helper process with an OS sandbox would close this.
- **A native task that ignores cancel keeps its thread.** Only a sandboxed task can be interrupted.
- **A pointwise effect can leave a partly written frame.** An effect with `Layout::pointwise()` changes the layer's frame in place, so when one band fails the bands that finished keep their changes. Keeping a copy to undo them cost every frame of every light effect (native Probe 0.30 to 0.70 ms at 1080p), and native code is code the user approved. In the sandbox, holding each band in its instance and copying all of them out only after every band succeeded cost about 0.3 ms (20 %) on sandboxed Probe at 1080p, because the copies then read bands that had left the cache; swapping each band out at once and back on failure cost the same. Every other layout leaves the frame as it was.
- **Effects cannot read audio.** The render path reaches footage audio, not the comp mix.
- **Forge 3D draws only on the GPU.** Draw passes render to an `Rgba16Float` target that is created per frame and copied.
- **Ember simulates from frame 0 on every render when a parameter has an expression**, because the history cannot be hashed. It reads the masks at the current time for every step.
- **`plugin dev` was not run against a live app, and double-clicking a `.craftplugin` was not tried on a built app.**
- **The 8/16 bpc quantise pass still runs after most CPU effects.** Gaussian Blur, Levels, Curves, Hue/Saturation and Glow quantise their own output (`QUANTIZES_OUTPUT` in `crates/effects`); plugin effects quantise each band as it is written; every other built-in effect is followed by a full-frame pass.
- **An animated comp keeps only its bottom run of static 2D layers composited.** On the CPU compositor the layers below the first animated, adjustment, collapsed, 3D or matted layer are kept as one buffer; every layer from there up is composited again each frame. The GPU compositor re-composites every layer.
- **No network host call.** A sandboxed plugin cannot open a connection; a native plugin can.
- **The capability page names other companies' products as examples.** House rule 22 is about feature names; check `standards/naming.md` before this goes upstream.

## Seen while running the full Mac gate

- **`effectcraft-media` integration tests hang on this Mac, on upstream `main` too.** `cargo test -p effectcraft-media --test it` stops at 0 % CPU with several `footage::` tests unfinished (`vp9_webm_frames`, `concurrent_requests`, `cache_budget_evicts`, `mxf_mpeg2_with_pcm_audio` and others, a different set each run). A sample shows the threads parked in `effectcraft_media::pool::Inner::frame_px` and `run_decode`, waiting on a channel. One of those tests passes in 0.2 s when run alone. The same suite hangs the same way in a worktree of untouched `main` (563fea65), and this branch does not change `crates/media`. So the full `cargo xtask ci` gate cannot finish on this Mac. Not investigated further.

## Seen while trying the app by hand, not fixed

- **Work after the last auto-save is lost on a crash.** By design: the default interval is 20 minutes
  (Settings ▸ Auto-Save). Not a bug, noted because it looks like one when testing recovery.
- **Launching the app from an agent's shell can get it stopped about 30 minutes later**, with no log line
  and no crash report, because the shell's process group is cleaned up. Start it in its own session
  (`setsid` or a double fork) or from the user's own Terminal; `pluginkit/demo/start.sh` is the demo
  launcher.

## Found by the speed study, not fixed

The measurements and the ranked list of changes are in `pluginkit/reports/speed-study.md`.

- **Timeout checks cost 8 to 35 % of sandboxed compute** (Plasma Beam 4.44 → 3.23 ms with them off, Halo
  25 → 17.5 ms). They are needed to stop a plugin that hangs; the options are in the study.

## Found by the second host kernel work, not fixed

- **Lumen Glow on the GPU is still about 7 % slower than with its own shaders.** 1080p, in-process median
  of 21 runs, minimum of six alternating runs at load 5 to 6: 8.0 ms with each fused loop generated as
  straight-line WGSL (`pointwise` in craft-libs `crates/plugin-host/src/kernels/gpu.rs`), 9.7 ms with the interpreter
  it replaced; the build with its own shaders (`pk/kernels`) measured 7.5 ms at load 3 to 4 and was not
  measured again. The rest is the emission, stored at full size before the first halving. Running the
  generated program on each of the halving's 16 taps instead removed that image and made four chained
  Lumen effects 20 % faster, but one 1080p frame 1.7 ms slower, so it was not kept.
- **Native Lumen Glow at 1080p takes 5 % longer than at the start of the third build** (13.35 ms against
  12.66 ms, minimum of four alternating runs at load 4 to 6; 4K 41.6 against 41.2 ms). Pass images are now
  allocated when first used and a dead one is reused, which removed 0.6 ms of single-threaded zeroing. What
  is left: the emission and the upsampled glow are two more full-size images written and read. Reusing one
  temporary for the resample and convolve kernels instead of zeroing a new one per call was tried and not
  kept: `sample` put all zeroing at 3 % of the render before and 2.4 % after (patch in
  `pluginkit/reports/w5-lumen-temp-reuse.patch`). About 75 % of the samples are in the fused pointwise rows
  and the transfer rows (craft-libs `crates/plugin-host/src/kernels/pointwise.rs`), 19 % in the resample.
- **Native and sandboxed Lumen Glow give different pixels.** Its plan's blur taps and level gains use
  `f32::exp`, `exp2` and `log2`, which differ in the last bits between the native and the WebAssembly
  standard library (`plugins/lumen-glow/src/blur.rs`, `settings.rs`, `profile.rs`). `craft_plugin::math`
  would make them equal but changes the pixels slightly.
- **Lumen Glow with a negative Aspect on a frame 16384 wide and 1 tall is refused.** Its levels halve only
  across, and the app cannot pad such a frame, so two images per level stay half a frame each and the plan
  asks for more than 8 frames of scratch. The effect reports the refusal and leaves the frame unchanged.

## Seen in the third build

- **The test kit's band check renders a pointwise effect whole with `frame.pixels` transparent.** An effect
  that changes `frame.pixels` directly (not through `rows_in_place`, `map` or `rows`) passes in the app but
  fails `TestPlugin` renders taller than one band (`timed` in craft-libs `crates/plugin-host/src/testing.rs`). The
  whole render needs separate input so that an effect reading rows it did not declare is still caught
  (`plugins/cookbook`, `Smear`); one in-place band would not catch it.
- **A native plugin call that never returns keeps its worker thread.** The app never waits for it: its later panel calls, overlays and menu commands queue behind it, and a host request it is waiting on is released with an error when its panel, overlay or tool closes or the plugin is unloaded. A call stuck inside the plugin's own code cannot be stopped in one process; that needs the helper process (`docs/plugin-helper.md`). A sandboxed call stops at its time limit. A synchronous `plugin.run` (CLI, scripts, `engine.execute`) waits for the plugin like any other command.

## Found by the region work, not fixed

- **A built-in effect that reaches its neighbours makes the plugin effects before it render whole.** Plugin
  effects, the colour effects in `POINTWISE` (`crates/effects/src/lib.rs`), Fill without feather and Gaussian
  Blur on its sampled path (sigma under 3 grid pixels) declare their margin in `pixel_margin`. A plugin
  effect declares one only when it is pointwise: `Layout::neighbourhood(n)` promises rows, and its columns
  are whole rows, so an effect before a neighbourhood plugin effect renders whole. Every other
  built-in, a larger Gaussian Blur (its running box sums are not bit-identical on a part), Fast Box Blur and
  Glow among them, declares none. A plugin margin is read at the layer's raster scale, so a solid above 16
  million pixels (made at a lower scale) underestimates a fixed reach in pixels. The shown part also ignores
  the layer's masks.
- **A light sandboxed effect is slower than with in-place frames.** Its band now has an input region and a
  separate output region in plugin memory, and the output region is zeroed before the call, so Probe
  sandboxed at 1080p went from 1.64 to 2.55 ms (minimum of three interleaved runs, load 34 to 40). Heavy
  effects gain: sandboxed Retro Neon 46.6 to 30.2 ms in the same runs.

- **A layer cache entry may be kept for a layer whose built-in effect reads another frame while an
  expression is present.** Built-in effects that read other frames are not counted as reading other times
  for the layer cache. Raised by the render fix worker; not verified. (`crates/render/src/cache.rs`)

## From the sixth build, not fixed

- **`Camera` gained a `focus` field.** Plugin code that builds a `craft_plugin::Camera` field by field no longer compiles; built plugins still load. Plugins normally only read the camera the app sends.
- **`cx.scene()` without `.reads_scene()` is not refused.** `cx.placement`, `cx.placement_at` and `cx.scene_at` are refused without their declarations; the older `cx.scene()` is not, so an undeclared call can still show a stale cached frame.
- **An effect that reads the scene at other times is not cached in a comp with any expression.** Its cache key covers every layer's keyframes but cannot cover expression results (`cache::hash_scene`).
