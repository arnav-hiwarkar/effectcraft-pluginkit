# Known issues of the plugin system

Found while the plugin system was built and reviewed, and not fixed. Each entry says where to look.
[decisions.tsv](decisions.tsv) is the log of what was decided and how each step was checked.

- **wasmtime and libloading licences.** House rule 26 wants every new dependency's licence on `standards/licences.md` in craft-contrib. wasmtime is Apache-2.0 WITH LLVM-exception, libloading is ISC. I did not check the list (it is in a storytold repo). Check before any upstream proposal.
- **`files.read` and `files.write` on Windows.** The plugin gate widens a plugin's roots to the filesystem root. On Windows that is the current drive only, so a plugin with `files.write` cannot write to another drive. Conservative, not a hole.
- **House rule 10 (git dependencies only from storytold) and rule 9 (shared code in craft-libs).** The two kit crates live in this repository so the fork builds on its own. Moving them to craft-libs is a plain move when wanted.
- **Plugin files outside the plugin's folders are not in the cache key.** An effect that reads a file through `files.read` keeps its cached frames when that file changes. The key holds only the plugin's own folders, which can be listed cheaply.
- **Possibly dead branch.** `show_result_errors` in `crates/ui-egui/src/menus/frontend.rs` still formats `{file, error}` items, a shape only the removed bundle install produced. The test near `menus/tests.rs:900` uses the removed command id `effect.installBundle` as a fixture.
- **ScriptUI panel bundles.** The removed `Install Effect Plug-in Bundle (Folder)` command also copied `.jsx` panels. Single panels still install through `file.installScriptUIPanel`.
- **SDK gaps reported by the Plasma Beam author.** A percent parameter is fixed at 0 to 100. A preset cannot set the visible parameter values (After Effects plugins do this). `cx.f` does not clamp to the slider range, so expression-driven values can leave it. A whole-frame sandboxed effect cannot use more than one core.

## From the Sol review, not fixed

- **Late plugin start skips parameter migration.** A project opened while its plugin is disabled keeps its saved parameters. If the plugin later starts with a new parameter, existing instances lack it. The render uses the declared default for a missing value (`wire_params` in `crates/plugin/src/effects.rs`), but the parameter has no property, so it cannot be shown or keyframed until the effect is applied again. Migrating needs a hook when a plugin starts late.
- **A slow host call is not interrupted.** The call fails when the host call returns past the limit, but the app cannot stop its own file read on a stalled disk. Instantiation has its own 5 s limit on top of the call limit. Interrupting needs host calls that run on another thread.
- **Reloading leaks the old effect descriptions.** Each reload leaks a few kilobytes of strings, because the effect registry hands out `'static` references.
- **A dangling link is also possible in the automation confinement.** `crates/automation/src/confine.rs` falls back to the parent folder when a path does not resolve. This is upstream code and affects agents too.

## Reported by the flagship plugin authors, not fixed

- **Engine: `keys.setEase` changes the side you did not set.** On a continuous key it copies the speed to the other side, and it turns off auto-Bezier so the other side falls back to a stored ease. Glide works around both. (`crates/engine/src/cmd/anim.rs`)
- **Engine: no path length for spatial properties.** Converting a curve to a speed along a motion path needs the path's length. Glide samples `prop.get` 129 times in a batch. `keys.info` could return it.
- **Engine: influence cannot go below 0.1 %.** A curve handle exactly on its anchor's column is stored about 6.6e-4 of the value range off.
- **Engine: `keys.info` gaps.** It does not give a key's index in its property, and reports 2 dimensions for scale on a 2D layer while `keys.setEase` works on 3.
- **CLI: `bench` does not load plugins**, so plugin effects are missing from its timings. (`bench_cmd` in `apps/effectcraft-cli/src/main.rs`)
- **SDK: sliders have one range.** No logarithmic mode and no separate soft range. `ui.number` has no range or decimals.
- **SDK: padding is one number for all four sides.** A horizontal streak pays for vertical padding too.
- **SDK: pointer events carry no modifier keys**, and only the last pointer event per frame reaches a canvas.
- **SDK: `read_file` cannot tell a missing file from a failed read.**
- **GPU: on WebGPU a bad plugin shader is not contained, and plugin GPU work has no time limit there**, because the browser build cannot wait for a submission (`Clock` in `crates/gpu/src/fx_plugin.rs`). Natively one band of rows is the smallest piece, so a kernel that is very slow on every pixel of one row still runs that band to the end before the limit stops the plan.
- **GPU: a plan stopped by the time limit is reported only in the log.** The effect falls back to the CPU and Effect Controls does not say why (`plugin_stop` in `crates/gpu/src/context.rs`).
- **UI: plugin labels have no automation id**, so the control channel cannot read a panel's text. A click made just before a tab is hidden is delivered when it reopens.

## From the Opus review, not fixed

- **Some queries need `document.write`.** The gate reads `journal` from the command table, and commands declared with `cmd!` are journaled even when they only read, such as `text.fonts` and `text.fontFeatures`. `crates/plugin/src/gate/census.tsv` shows each decision. Too strict, not a hole; the fix is in the engine's command table.
- **Background jobs** (`camera.analyze`, `layer.sceneEditDetection`) apply their result later, outside the plugin's batch and its undo step. Plugins can start them but cannot wait for them.
- **Panel events are dropped when the panel call fails**, so the click that crashed a plugin is lost.
- **Glide: `keys.interpolation` runs on the whole selection**, so keys on properties Glide reports as skipped still lose their continuous and roving flags. (`plugins/glide/src/apply.rs`)
- **A plugin render error keeps Effect Controls repainting every 500 ms** until the error clears.

## From the second build and its reviews, not fixed

- **A process of the same user that reads the cache key can still sign a compiled module.** Compiled cache entries carry an HMAC-SHA-256 under `Plugins/.compiled/key` (mode 0600, made from OS randomness), so a planted entry without the key is compiled again (`cached` in `crates/plugin-host/src/runtime/wasm.rs`). The key file is readable by every process of the same user; only the helper process with an OS sandbox would close this.
- **A native task that ignores cancel keeps its thread.** Only a sandboxed task can be interrupted.
- **In the CLI a task's document requests are answered only inside `plugin.tasks {wait}`.**
- **Effects cannot read audio.** The render path reaches footage audio, not the comp mix.
- **Plugin panels and custom controls cannot be fully used from the keyboard.** A canvas cannot take focus and custom controls get no key events. House rule 21 asks for this. (`crates/ui-egui/src/panels/plugin_view.rs`, Opus finding 8)
- **One `reach` covers a pass's `src` and `aux`.**
- **Lumen Glow has no CPU fallback for a level that needs more than 48 floats**; its `render` returns an error there.
- **Forge 3D draws only on the GPU.** Draw passes render to an `Rgba16Float` target that is created per frame and copied.
- **Ember simulates from frame 0 on every render when a parameter has an expression**, because the history cannot be hashed. It reads the masks at the current time for every step.
- **A Halo preset sets the look and resets the element controls.** It does not write absolute slider values.
- **Curves keeps the selected channel per effect, not per instance of the effect**, unless the fix for Opus finding 7 landed (see the log).
- **`polygon` on a canvas fills convex shapes only.**
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

- **A large reach repeats most of a band.** A `Layout::neighbourhood(n)` band renders its own rows plus `n`
  above and below. Retro Neon has `n = 67` at 1080p, so each of ten 108-row bands renders 242 rows. Fewer,
  taller bands, or a pass plan, would repeat less. (`Plugin::render` in `crates/plugin-host/src/plugin.rs`)
- **Timeout checks cost 8 to 35 % of sandboxed compute** (Plasma Beam 4.44 → 3.23 ms with them off, Halo
  25 → 17.5 ms). They are needed to stop a plugin that hangs; the options are in the study.
- **A light native effect pays for a copy of the frame.** Every render now writes a new frame that replaces
  the old one only when all bands succeed. A native band copies its input rows into that frame and renders
  there, so native Probe went from 0.54 to 1.23 ms and native Plasma Beam from 2.23 to 2.60 ms at 1080p. The
  cure is a frame with separate rows to read and rows to write, which the region-of-interest work brings.
