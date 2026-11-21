# Known issues of the plugin system

Found while the plugin system was built and reviewed, and not fixed. Each entry says where to look.
[decisions.tsv](decisions.tsv) is the log of what was decided and how each step was checked.

- **wasmtime and libloading licences.** House rule 26 wants every new dependency's licence on `standards/licences.md` in craft-contrib. wasmtime is Apache-2.0 WITH LLVM-exception, libloading is ISC. I did not check the list (it is in a storytold repo). Check before any upstream proposal.
- **`files.read` and `files.write` on Windows.** The plugin gate widens a plugin's roots to the filesystem root. On Windows that is the current drive only, so a plugin with `files.write` cannot write to another drive. Conservative, not a hole.
- **Docs that still describe the removed plug-in system.** `docs/agents.md`, `docs/gaps.md`, `docs/target-app-parity.md`, `docs/file-format-parity.md`, `docs/scripting-parity.md`, `docs/control-protocol.md`, `docs/preferences.md`, `docs/web.md`, `docs/releasing.md`, `docs/effects-parity.md`, `docs/roadmap.md` mention effect plug-ins, helpers or the Plug-in Library. Not rewritten.
- **House rule 10 (git dependencies only from storytold) and rule 9 (shared code in craft-libs).** The two kit crates live in this repository so the fork builds on its own. Moving them to craft-libs is a plain move when wanted.
- **Dead links after the removal.** `docs/gaps.md` links to the deleted `external-helpers.md` and ADR 0001, and `docs/agents.md` line 698 links to the deleted `publishing-plugins.md` and line 779 names `effect.plugins.*`.
- **Possibly dead branch.** `show_result_errors` in `crates/ui-egui/src/menus/frontend.rs` still formats `{file, error}` items, a shape only the removed bundle install produced. The test near `menus/tests.rs:900` uses the removed command id `effect.installBundle` as a fixture.
- **ScriptUI panel bundles.** The removed `Install Effect Plug-in Bundle (Folder)` command also copied `.jsx` panels. Single panels still install through `file.installScriptUIPanel`.
- **SDK gaps reported by the Plasma Beam author.** A percent parameter is fixed at 0 to 100. A preset cannot set the visible parameter values (After Effects plugins do this). `cx.f` does not clamp to the slider range, so expression-driven values can leave it. A whole-frame sandboxed effect cannot use more than one core.

## From the Sol review, not fixed

- **Cache keys of dependent layers.** A layer that samples another layer through a layer parameter, and the comp thumbnail, do not include the sampled layer's plugin revision. Reloading a changed plugin can leave those two caches stale until something else changes. (`crates/render/src/cache/dependencies.rs`, `disk_cache.rs`)
- **Effects that read data files.** The render cache does not know when a plugin's data file changes, so an effect that reads a palette file keeps the cached frame after the file changes.
- **Late plugin start skips parameter migration.** A project opened while its plugin is disabled keeps its saved parameters. If the plugin later starts with a new parameter, existing instances lack it. (Missing values now fall back to the declared default once the adapter change lands.)
- **Time limits do not cover host work.** A plugin blocked inside a slow host call, such as a file read on a stalled disk, is not interrupted. Instantiation has its own 5 s limit on top of the call limit.
- **Native plugins can leave a partly written frame.** A native effect that changes pixels and then fails leaves those changes. Sandboxed effects do not.
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
- **GPU: a plugin kernel that loops forever can time out the GPU context.** Compile errors quote line numbers that include the prelude. The shader cache keeps entries of unloaded plugins until the GPU context is dropped. On WebGPU a bad plugin shader is not contained.
- **UI: plugin labels have no automation id**, so the control channel cannot read a panel's text. A click made just before a tab is hidden is delivered when it reopens.

## From the Opus review, not fixed

- **A batch can change which effect a path names.** The check for writing text-valued effect properties resolves `effects/#1/x` before the batch runs. A batch that reorders effects and then writes a same-named parameter of a different type is not caught. It needs two effects that share a parameter id with different types.
- **`text.fonts {rescan}`** lists the user's installed fonts to any plugin with `document.read`.
- **`layer.setText {font}`** saves the recent-fonts list to the settings file as a side effect.
- **Background jobs** (`camera.analyze`, `layer.sceneEditDetection`) apply their result later, outside the plugin's batch and its undo step. Plugins can start them but cannot wait for them.
- **Panel events are dropped when the panel call fails**, so the click that crashed a plugin is lost.
- **Glide: `keys.interpolation` runs on the whole selection**, so keys on properties Glide reports as skipped still lose their continuous and roving flags. (`plugins/glide/src/apply.rs`)
- **A plugin render error keeps Effect Controls repainting every 500 ms** until the error clears.

## Seen while running the full Mac gate

- **`effectcraft-media` integration tests hang on this Mac, on upstream `main` too.** `cargo test -p effectcraft-media --test it` stops at 0 % CPU with several `footage::` tests unfinished (`vp9_webm_frames`, `concurrent_requests`, `cache_budget_evicts`, `mxf_mpeg2_with_pcm_audio` and others, a different set each run). A sample shows the threads parked in `effectcraft_media::pool::Inner::frame_px` and `run_decode`, waiting on a channel. One of those tests passes in 0.2 s when run alone. The same suite hangs the same way in a worktree of untouched `main` (563fea65), and this branch does not change `crates/media`. So the full `cargo xtask ci` gate cannot finish on this Mac. Not investigated further.
