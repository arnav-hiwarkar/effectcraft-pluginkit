# Known issues of the plugin system

Found while the plugin system was built and reviewed, and not fixed. Each entry says where to look.
[decisions.tsv](decisions.tsv) is the log of what was decided and how each step was checked.

- **wasmtime and libloading licences.** House rule 26 wants every new dependency's licence on `standards/licences.md` in craft-contrib. wasmtime is Apache-2.0 WITH LLVM-exception, libloading is ISC. I did not check the list (it is in a storytold repo). Check before any upstream proposal.
- **`files.read` and `files.write` on Windows.** The plugin gate widens a plugin's roots to the filesystem root. On Windows that is the current drive only, so a plugin with `files.write` cannot write to another drive. Conservative, not a hole.
- **House rule 10 (git dependencies only from storytold) and rule 9 (shared code in craft-libs).** The two kit crates live in this repository so the fork builds on its own. Moving them to craft-libs is a plain move when wanted.
- **Plugin files outside the plugin's folders are not in the cache key.** An effect that reads a file through `files.read` keeps its cached frames when that file changes. The key holds only the plugin's own folders, which can be listed cheaply.
- **Possibly dead branch.** `show_result_errors` in `crates/ui-egui/src/menus/frontend.rs` still formats `{file, error}` items, a shape only the removed bundle install produced. The test near `menus/tests.rs:900` uses the removed command id `effect.installBundle` as a fixture.
- **ScriptUI panel bundles.** The removed `Install Effect Plug-in Bundle (Folder)` command also copied `.jsx` panels. Single panels still install through `file.installScriptUIPanel`.
- **SDK gap reported by the Plasma Beam author.** A whole-frame sandboxed effect cannot use more than one core.
- **`params!` cannot declare an id that ends in a Rust keyword.** The field name is the id, so `Core/type` (Plasma Beam) and `Emitter/type` (Ember) stay outside the struct and are inserted into the list by hand. A way to give a field an explicit id would remove that (`crates/plugin-sdk/src/typed.rs`).

- **Viewer tools from automation.** A plugin tool is picked only with its toolbar button. `ui.set` and the `tool.*` commands do not name plugin tools (`crates/ui-egui/src/panels/plugin_tools.rs`).
- **Viewer tools get no pen pressure.** egui reports no pressure for a pen, so `ToolEvent::pressure` is 1 while the button is down and 0 otherwise.
- **Built-in point crosshairs on an adjustment layer are drawn in layer space.** The effect works on the comp below, so a moved adjustment layer shows them off the point the effect uses. Plugin handles use comp space there (`crates/ui-egui/src/panels/effect_controls/viewer.rs`, `from_effect_space`).
- **One slow interactive call delays the plugin's other surfaces.** Panels, custom controls, overlays and tools of one plugin share its worker queue, so a panel call that runs for a second holds back that plugin's overlay for that second (`crates/plugin/src/panels.rs`).
- **Task host requests are answered by whichever session pumps first.** `effectcraft_plugin::pump` serves every task of a plugin; panel requests are routed by their project token, task requests only check that their project is still open (`crates/plugin/src/manage.rs`).
- **Two copies of the theme colour mapping.** `colour` in `crates/ui-egui/src/panels/effect_controls/plugin_overlay.rs` repeats `paint` in `panels/plugin_view.rs` (owned by the panels work in the same wave). Share one after both merge.

## From the Sol review, not fixed

- **A slow host call is not interrupted.** A file call on a stalled disk or a slow app command runs to its end on the calling thread; the sandboxed call fails when it returns past the limit. A panel's `frame` and `audio` are cut from the project on the app's thread and rendered on the plugin's worker; a task's still render on the app's thread when the task pump answers them. A file write cannot be cancelled, so it is not moved to another thread and abandoned: that let a write land after the plugin was told it failed, and stalled threads piled up. Stopping one needs the helper process (`docs/plugin-helper.md`), where the whole process can be killed. Instantiation has its own 5 s limit on top of the call limit.
- **A dangling link is also possible in the automation confinement.** `crates/automation/src/confine.rs` falls back to the parent folder when a path does not resolve. This is upstream code and affects agents too.

## Reported by the flagship plugin authors, not fixed

- **Engine: `keys.setEase` changes the side you did not set.** On a continuous key it copies the speed to the other side, and it turns off auto-Bezier so the other side falls back to a stored ease. Glide works around both. (`crates/engine/src/cmd/anim.rs`)
- **Engine: influence cannot go below 0.1 %.** A curve handle exactly on its anchor's column is stored about 6.6e-4 of the value range off.
- **CLI: `bench` does not load plugins**, so plugin effects are missing from its timings. (`bench_cmd` in `apps/effectcraft-cli/src/main.rs`)
- **GPU: on WebGPU a bad plugin shader is not contained, and plugin GPU work has no time limit there**, because the browser build cannot wait for a submission (`Clock` in `crates/gpu/src/fx_plugin.rs`).
- **GPU: a runaway plugin kernel stalls the GPU once per load.** A wgpu dispatch cannot be cancelled, so the 2 s budget is checked when a frame's plan has finished. The first frame that passes it stops the effect, which then renders on the CPU until the plugin is reloaded; that one frame still runs to its end (or until the OS GPU watchdog or the 10 s readback limit), and every other GPU effect waits for it. The error lists the plan's passes but cannot say which one was slow.

## From the Opus review, not fixed

- **Background jobs** (`camera.analyze`, `layer.sceneEditDetection`) apply their result later, outside the plugin's batch and its undo step. Plugins can start them but cannot wait for them.
- **Glide: `keys.interpolation` runs on the whole selection**, so keys on properties Glide reports as skipped still lose their continuous and roving flags. (`plugins/glide/src/apply.rs`)

## From the second build and its reviews, not fixed

- **A process of the same user that reads the cache key can still sign a compiled module.** Compiled cache entries carry an HMAC-SHA-256 under `Plugins/.compiled/key` (mode 0600, made from OS randomness), so a planted entry without the key is compiled again (`cached` in `crates/plugin-host/src/runtime/wasm.rs`). The key file is readable by every process of the same user; only the helper process with an OS sandbox would close this.
- **A native task that ignores cancel keeps its thread.** Only a sandboxed task can be interrupted.
- **Effects cannot read audio.** The render path reaches footage audio, not the comp mix.
- **Forge 3D draws only on the GPU.** Draw passes render to an `Rgba16Float` target that is created per frame and copied.
- **Ember simulates from frame 0 on every render when a parameter has an expression**, because the history cannot be hashed. It reads the masks at the current time for every step.
- **Custom Effect Controls rows cannot subscribe to app events or set timers.** They are called again on their own widget events, a new value or a new width. (`crates/plugin/src/panels.rs`)
- **`plugin dev` was not run against a live app, and double-clicking a `.craftplugin` was not tried on a built app.**
- **The 8/16 bpc quantise pass still runs after most CPU effects.** Gaussian Blur, Levels, Curves, Hue/Saturation and Glow quantise their own output (`QUANTIZES_OUTPUT` in `crates/effects`); every other built-in effect and every plugin effect is followed by a full-frame pass.
- **An animated comp re-composites every cached layer on each frame.** Only a comp where nothing animates is served whole from the layer cache.
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

- **Lumen Glow on the GPU is 25 % slower than with its own shaders.** 1080p, in-process median of 21 runs
  at load 3 to 4: 9.4 ms against 7.5 ms for the build before (`pk/kernels`). The fused pointwise loop is
  one interpreting shader (`craft_pointwise` in `crates/gpu/src/shaders/plugin_kernels.wgsl`): its register
  array indexed at run time costs about 0.4 ms (measured by replacing it), and the switch and per-op data
  reads cost more. The emission is stored at full size before the first halving, where the old shader
  thresholded each of the halving's 16 taps; running the fused program per tap instead was slower (10.0
  ms). Generating WGSL per fused loop, or allocating its values to named registers on the host, would
  remove the interpreter.
- **Native Lumen Glow on all cores takes 5 % longer in wall time** (14.4 ms against 13.7 ms at 1080p, load
  3) although it runs 3 % fewer instructions: the emission and the upsampled glow are two more
  full-size images written and read.

## Seen in the third build

## Found by the region work, not fixed

- **Only the last plugin effects of a stack render a part.** A plugin effect gets the shown part grown by
  the margins of the effects after it, and built-in effects declare a margin only for a few 3D channel
  effects (`pixel_margin` in `crates/effects/src/channel3d.rs`), so a plugin effect followed by a blur or a
  colour effect renders the whole layer. Plugin effects declare no margin to the pipeline either, so the
  first of two plugin effects renders whole. The shown part also ignores the layer's masks.
- **A light sandboxed effect is slower than with in-place frames.** Its band now has an input region and a
  separate output region in plugin memory, and the output region is zeroed before the call, so Probe
  sandboxed at 1080p went from 1.64 to 2.55 ms (minimum of three interleaved runs, load 34 to 40). Heavy
  effects gain: sandboxed Retro Neon 46.6 to 30.2 ms in the same runs.
- **A moving layer that is partly outside the frame is rendered again on every frame.** Its cache key holds
  the shown rectangle, which moves with it. A layer wholly inside the frame keeps one key.

- **A layer cache entry may be kept for a layer whose built-in effect reads another frame while an
  expression is present.** Built-in effects that read other frames are not counted as reading other times
  for the layer cache. Raised by the render fix worker; not verified. (`crates/render/src/cache.rs`)
