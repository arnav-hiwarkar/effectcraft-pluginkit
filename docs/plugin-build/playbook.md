# Plugin build playbook

How the Craft Plugin Kit was built and changed, written for the next build and for the step where every
ArtCraft app moves to one plugin system. It keeps what held up, with the evidence. The full log is
[decisions.tsv](decisions.tsv) ("row N" below is its Nth entry, header not counted), measured speed is in [speed.md](speed.md),
open problems are in [known-issues.md](known-issues.md). How to give another app plugins is in
`crates/plugin-host/ADAPTING.md` in craft-libs.

## Principles that held

**The SDK grows by addition.** A new host ability is one `HostCall` variant and one `Host` method with a
default body. A new thing the app asks a plugin is one request variant and one trait method with a default.
New fields are `Option` or serde-default. The binary interface stayed at version 1 through six builds and
about forty units. The one break was a public struct a plugin builds by literal: adding `Camera::focus`
broke plugins that write `Camera { .. }` (w6-placement-scene). Give new public data types a constructor or
`#[non_exhaustive]` from the start.

**Old plugins must keep loading, and a test proves it.** `crates/plugin/tests/it/legacy/legacy.wasm` is a
plugin built against an early SDK and checked in. Every wire or ABI change runs the tests that load it.
Grep is not enough: the w6 id check passed every grep and still refused Retro, because the `params!`
`"Group Name" { … }` form makes ids with spaces. Only `plugins/e2e.sh` caught it (w6-param-ids).

**One queue per plugin for interactive calls.** Panels and the viewer were built in parallel and each
called the instance its own way. A review found 21 defects, 13 of them that one seam: a UI deadlock, stale
replies acting on a new project, lost or repeated input (rows 72-73). The fix was one worker thread per
plugin for every panel, control, overlay, drag, tool and menu call, one identity (load, project token,
surface token) checked at every hand-off, and a UI thread that never waits on a plugin. A menu command
later moved onto the same queue and the special `settle` wait was deleted (row 94).

**App-independent code at the lowest shared layer.** The kit started inside EffectCraft as two crates with
no EffectCraft types (row 2), which made the later move to craft-libs a pure move (w7-libs-move). The panel
renderer moved out of `ui-egui` into `craft-plugin-ui` with everything app-shaped passed through a `Host`
trait: theme, translation, textures, drag payloads, even polygon triangulation (w7-plugin-ui-crate). Move
code once, as a move, never as a copy kept in two places.

**Measure before building speed work.** The speed study (`reports/speed-study.md`) ranked nine changes by
measured gain before any was built, and three ideas were dropped on its numbers. A community PR claimed
6:01 → 3:02 for a "generator fast path", but that figure was three changes on top of four other PRs with no
command (w6-pr-study); the plan made a measuring unit the gate for building it. A Lumen scratch reuse that
was correct but gained under 3 % was reverted (w5-lumen).

**Subtract before adding.** The best fixes deleted mechanisms that kept producing defects: the GPU time
limit (net 199 lines fewer, row 103), derived group ids and their migration (net 133 fewer,
w6-param-ids-relax), the source-text scans in `plugin check` (row 100), the `settle` wait. The counter
example: the fix for the interactive seam was asked for less code and `panels.rs` grew from 446 to 754
lines (rows 82, 101). When a brief asks for less code, check the line count at merge.

**Tests that fail without the change.** Every new test is mutated once: break the code it guards and see it
fail. Mutation runs found a time-dependence test that could not fail (it rendered without a layer cache,
w6-time-check) and a refusal test that a `let _ = x?;` mutation did not break. Tests compare literal values;
a speed change is bit-identical to the old output; a pixel change is compared with a literal tolerance.

## The design in one page

**Two runtimes, one SDK.** The same plugin source builds as sandboxed WebAssembly (Wasmtime with
Cranelift, in process) or as an approved native library. One JSON call each way (`craft_plugin_call`,
`host_call`); pixels go by pointer and length, never as JSON. JSON costs under 0.2 % of render time.
Sandboxed: one import, epoch time limit per call, memory, table and instance limits, a fresh instance after
a trap, an authenticated compiled-module cache. Native: every host call passes the same permission gate,
approval is per version.

**Layouts tell the host how to cut the work.** `pointwise` (reads only its own pixel: rendered in place),
neighbourhood with a `reach` (bands with separate input rows and output rows), `passes` (a plan of images
and passes the host runs, on CPU in bands or on GPU), `maps_input` (reads input at other positions, so no
region shrinking), and padding on four sides with a cap. A band writes only its own rows; nothing renders
twice.

**Host kernels and fusion.** A plan names built-in kernels (blur, resample, blend, curves, convolve, warp,
colour matrix, transfer curves, brightness curve, dither, light alpha). The host runs them natively with
SIMD on the CPU and as its own WGSL on the GPU. Consecutive pointwise kernels fuse into one row loop on the
CPU and one generated shader on the GPU. A sandboxed plugin whose render is all host kernels runs at native
speed with no copies.

**Region of interest.** The compositor asks each effect only for the part of the layer the frame shows,
grown by each later effect's margin (`reach` plus padding over scale), and snapped outwards to a 64-pixel
grid. The rectangle is part of the layer's cache key.

**Dependence flags and cache keys.** An effect declares `time_dependent`, `reads_scene`,
`reads_other_frames` and layer inputs. Keys hash parameter values (not their `Debug` text), the plugin's
revision (version, module, files in its folders), the shown rectangle, and the scene where declared. The
test kit's time check renders at two times and names a missing `.time_dependent()`. When a key cannot
cover an input (track mattes, expressions with other-time reads, a missing file), the cache declines.

**Panels, viewer, tasks.** Panels are retained widget trees: the app draws the last tree each frame and
calls the plugin only on a widget event, a subscribed app event, a timer or a resize. Effects return
overlays and handles bound to parameters; a drag is one undo step. Tools get viewer pointer and key events.
Overlays are capped (4096 shapes, 256 handles, 4096 points, 512 characters). Tasks run on their own thread
and instance with progress and cancel, bound to the project they started in.

**Permissions.** `document.read`, `document.write`, `files.read`, `files.write`, `clipboard`. The app's gate
allows only document command families, maps undoable commands to write, and needs both file permissions for
any command that names a file. A census file records the decision for every allowed command and a test
fails when one is added. Scripts cannot install, approve, load or un-sandbox plugins. No network
permission exists.

**Helper process (designed, deferred).** Plugins in a separate process under the OS sandbox. Measured:
socket round trip 2.9 µs minimum, 4.9 µs median; sandboxed start 8.7 ms, restart 9.1 ms; 4.8 to 7.7 MB per
helper; frames in shared memory stay mapped with no page faults after warm-up. Lanes use Unix sockets
(they report a dead helper as end of file and carry file descriptors). It would contain crashes and hangs,
end slow host calls by killing the process, and remove the epoch checks (8 to 35 % of sandboxed compute).
Design and numbers: [plugin-helper.md](../plugin-helper.md).

**Sandbox by default (owner decision, row 120).** Once the helper exists, native plugins run under the OS
rules too. The `system` permission opts one out and keeps crash and hang isolation. A sandbox can be
loosened later without breaking plugins, but not added later.

## Techniques and algorithms that paid off

| Technique | Number (before → after) | Where measured |
|---|---|---|
| Band native neighbourhood effects on all cores | Retro CRT native 1080p 47.2 → 12.3 ms, Neon 54.2 → 16.7 ms | speed.md |
| A frame with separate rows to read and rows to write, so no band renders its reach rows | a 108-row Retro band rendered 242 rows before | row 53 |
| Render pointwise effects in place | a new frame per render made native Probe 0.30 → 0.70 ms | row 97 |
| A generator says it ignores its input and reports the rectangle it drew; the layer is cropped to it | sandboxed 4K sprite 6.33 → 0.42 ms, 89 M → 12.5 M instructions; native 1.28 → 0.14 ms | w8-generator-path |
| Host kernel plans run natively for sandboxed plugins | Lumen sandboxed 1080p 26.2 → 13.5 ms, equal to native | speed.md |
| Fused pointwise kernels, generated WGSL per fuse (not a WGSL interpreter) | Lumen GPU 8.88 → 7.80 ms | row 103 |
| Fast inline maths in the SDK (`math`, `math::fast`) | Plasma sandboxed 2.40x → 1.85x native | row 50 |
| Plain copies instead of nested parallel copies | 10 to 25 % on copy-bound effects | speed-study |
| Region of interest with margins through a stack | 4K plugin plus colour effect at 400 %: 29.2 → 11.4 ms; 7.5 M → 0.5 M plugin pixels | w5-margins |
| Snap the shown part to a 64-pixel grid | moving half-outside Neon layer 73.3 → 10.3 ms a frame | w5-moving |
| Keep the composite of the bottom run of static layers | 106.5 → 16.3 ms a frame; 26.4 G → 5.1 G instructions | w5-composite |
| A comp where nothing animates serves its frame from memory | 100 → 1.0-1.3 ms | speed.md |
| Quantise inside bands, in cache-sized chunks (~2048 pixels) | built-in five-effect stack 1.50 G → 1.34 G cycles; plugins: ~5 % native, none sandboxed | rows 59, w5-quantise |
| Build large buffers at their exact size | peak memory over 30 region renders 619 → 260 MB | w5-regionleak |
| Hash values for cache keys, not `Debug` text | one layer key 9.88 → 4.77 µs | speed.md |
| GPU layer inputs bound from the compositor's buffer | 22-30 → 7.3-7.8 ms | speed.md |
| One readback and upload per run of CPU effects in a GPU chain | per effect before | row 54 |
| Retained panels | 0 plugin calls in 100 idle frames; UI frame 0.83 → 0.62-0.66 ms | row 70, speed.md |

The exact-size lesson in full: `flat_map(..).collect()` grows a `Vec` by doubling, and macOS malloc kept
each freed 22 MB block resident ("MALLOC_LARGE (empty)" in `vmmap`), so memory grew by one region per
render. `MallocStackLogging` hides it. Use `Vec::with_capacity` and `extend_from_slice` per row.

Still open: a light sandboxed effect is bound by copying the frame in and out (Probe 3.7x native). Only a
frame arena mapped into the sandbox or the helper process removes that.

## Things that failed or were removed, and why

- **A time limit on plugin GPU work.** Built, repaired three times, reviewed four times; each review found
  a new way for it to stop the wrong plugin, because a wall clock around a shared GPU queue charges whoever
  waits. Removed (row 96). A correct limit needs per-plan GPU timestamps, or the helper.
- **Row banding of GPU compute passes.** It broke kernels that store to another row and dispatched the
  whole grid once per band. Removed in favour of one timed submission per plan (row 57).
- **A new frame for every render.** Bought a clean frame on a rare failure for 20 to 130 % on every light
  native effect. Reversed: pointwise renders in place again (row 98).
- **All-or-nothing sandbox bands by copying out only after every band succeeded.** The delayed copies read
  memory that had left the cache: about 20 % on light effects. Reversed (row 105). A band is copied out
  while it is still in cache.
- **Abandoning a slow file call at the time limit.** A write cannot be cancelled, so the abandoned write
  landed after the plugin was told it failed, and stalled threads piled up. Removed (row 74). Only killing
  a process stops it.
- **Strict id characters.** `[A-Za-z0-9_-]` refused real plugins (Retro) and needed derived ids plus a
  migration. Relaxed to "any character except `/` and control characters, not starting with `#` or `@`",
  with depth, length, uniqueness and count caps kept (w6-param-ids-relax).
- **Source-text scans in `plugin check`.** They refused good plugins and missed real cases. Replaced by the
  app's own load validation plus one render per effect (row 100).
- **Lumen scratch reuse.** Correct, under 3 % gain. Reverted, patch kept (w5-lumen).
- **A static frame cache that grows its key for every edge case.** It declines instead (matted comps, a
  missing file). A wrong hit shows a wrong picture; a decline only costs speed (row 76).
- **Light names in the scene.** Dropped to keep `Light` `Copy` (w6-placement-scene).
- **Not worth doing, measured or reasoned:** Cranelift inlining, relaxed or hand-written SIMD in plugins,
  a new wire protocol, remapping frames per render (2.85 ms against 1.95 ms for copying on macOS), NaN
  canonicalisation, a params digest (keys already hash values), a group-key prefix (groups are scoped
  inside the effect), several in-process plugin processes (bands already use all cores).

## Checklists

**A new SDK feature**
- One `HostCall` or request variant, one handler, a default trait body. New fields optional.
- The common case fits in ten lines of plugin code. Errors say what went wrong and what to change.
- A test that calls it the way an author would (probe plugin or a reference plugin).
- A section in `docs/plugin-reference.md` and a worked example in `docs/plugin-sdk.md`.
- A UI feature also gets a headless `egui_kittest` test with real pointer and key events.
- The legacy plugin test passes. A new public struct gets a constructor or `#[non_exhaustive]`.
- If the feature reads something new (time, scene, another layer), it needs a declaration, and the gate
  refuses the call without it.

**A speed change**
- Write the claim and the cheapest check that fails if it is false.
- Keep a copy of the before binary. Run before and after alternately, at least four rounds each, and take
  the minimum. Note `uptime` with every number.
- Above load 10, use instruction counts (`/usr/bin/time -l`, `RAYON_NUM_THREADS=1`), but count with all
  threads for band-level changes, and use alternating wall times for cache-locality changes (the work is
  the same, only the timing moves).
- Use a private `TMPDIR` so the compiled-module cache is not shared.
- Output is bit-identical, checked with `cmp` on rendered frames against the base commit, early.
- Measure the whole loop, not one function alone.

**A safety change**
- What can a hostile plugin do with it: allocate without bound, recurse without bound, hold a lock, stall
  the UI thread, act after its project closed or after reload, reach a file or command it was not granted?
- Cap every count and size a plugin controls, at load where possible (ids, params, stops, points, tables,
  overlay shapes, scratch bytes after expansion).
- Is the act the user's? Scripts and plugins must not approve, install or widen permissions.
- Every request carries load, project and surface identity, checked at each hand-off.
- Word external reviews as correctness reviews; some reviewers stop on attack wording.

**A cache-affecting change**
- List what the output depends on and what the key covers. Anything missing is either added or makes the
  cache decline.
- Test through a real layer cache: render, change the input, render again, compare with a fresh render.
  A test without `renderer.cache` cannot fail.
- Process-wide counters that feed keys make parallel tests race; serialise those tests.

**A change to what the SDK accepts at load**
- Run `plugins/e2e.sh`, the reference plugins' own tests and the legacy plugin test.
- Check every `params!` form, including `"Group Name" { … }`, and `plugins/new-plugin-check.sh`.

## How we ran the build

**Roles.** One coordinator (the main chat) writes briefs, reads every diff, merges and runs gates. Workers
are started with `delegate_task`, one unit each, one worktree each (`bin/wt NAME`, branch `pk/NAME`;
craft-libs units use `git worktree add ../wt/libs-NAME`). A brief names the files the unit owns, the claim,
the check that proves it, a size budget in changed lines, and numeric targets for speed work. The worker
writes a report under 300 words to `reports/<unit>.md`, ending with lessons. `AGENT-RULES.md` and
`LESSONS.md` carry rules and lessons between agents; lessons are folded in after each wave.

**Budgets and models.** At most five compile-heavy workers at once, `-j 4` (later `-j 3`). Opus 5.5 medium
by default; high (or xhigh) only for units named hard (region of interest, the interactive queue, kernels);
Sonnet for mechanical work (renames, deletions, small parameter gaps). Review findings that take a few
lines are fixed by the coordinator, not by a new worker.

**Merging.** Workers rebase onto `pluginkit` and rerun their checks last, so the tested tree is the merged
tree. Merge one at a time with `git merge --no-ff pk/NAME`, run the kit tests after each, and run the
combined tree when a branch was a merge behind. Delete the worktree's `target` once merged.

**Reviews.** One correctness review per wave of the merged diff by a different model family (GPT-6.2 Sol
high), split in two when the diff is large. Every finding needs a concrete input sequence. Findings go to
fix units with their own briefs. Each wave's reviews found real high-severity defects; none was refuted.

**The Mac gate**, run in the main checkout with `CI=true` so the test kit refuses to store a missing frame:
`cargo fmt --check --all`; `plugins/e2e.sh`; `plugins/new-plugin-check.sh`; kit and xtask tests; render,
gpu and effects tests; engine lib tests; ui-egui lib and integration tests; clippy with all targets and
`-D warnings` on the touched crates; `cargo xtask wasm` (web build); and before a push the asset, layers,
lock-patches and pdf-deps checks. Nothing merges while it runs. The full `cargo xtask ci` cannot finish on
the Mac (the media suite hangs on upstream `main` too).

**Linux CI** runs once per phase end, by hand, on GitHub (`.github/workflows/pluginkit.yml`: a `kit` job
for the craft-libs crates at the pinned rev and a `linux` job for the adapter, lint, web build, e2e and
templates). No CI on PRs. The runner's clippy is newer than the Mac's and finds lints the Mac cannot; keep
`--keep-going` on the lint step so one run lists them all. A failed run did not wake a PR watch; read it
with `gh run view`.

**What went wrong, and the rule that came from it.**
- *Disk full* (121 MB free of 926 GB): every worktree kept tens of GB of `target`. Rule: delete a merged
  worktree's `target`; workers stop and report on ENOSPC; after it, link errors mean damaged build files
  (`cargo clean -p` the named crates).
- *Toolchain broken*: a worker ran `rustup` and the shared stable toolchain lost `cargo` and `clippy`.
  Repaired by copying the files back. Rule: never run `rustup`; report a missing target instead.
- *App launches*: a worker opened and killed the windowed app seven times in eight minutes on the owner's
  screen. Rule: no worker starts the windowed app or uses `osascript`; UI claims are proven with kittests;
  the coordinator runs one scripted windowed session per wave through the control channel only
  (`windowed-checks.md`).
- *Stopped workers*: a worker stopped by the disk filling, or by an outage, cannot be resumed with its
  context. Restart a fresh worker in the same worktree and put what the first one learned in its brief.
- *Copying `target` while building*: starting cargo before `bin/wt` finished copying gave "colliding
  StableCrateId" errors. Wait for `bin/wt` to print its path.
- *Shared state between worktrees*: never share `CARGO_TARGET_DIR`; cargo's hashes ignore the checkout path.

## What to look at first

**A plugin is slow**
- `CRAFT_PLUGINS=<effectcraft>/plugins cargo run --release -p craft-plugin-host --example bench` in
  craft-libs, native against sandboxed. A large ratio on a light effect is the copy; on a heavy one, maths
  that calls functions per pixel (use `craft_plugin::math`) or a missing host kernel.
- `effectcraft-cli bench --project P --n 30` (add `--roi` for a region, `--play 30` for consecutive frames
  with the cache); `plugins/bench-projects.sh` builds three projects.
- `sample <pid> 6` on the running bench or CLI shows the hot function.
- Check the layout: a native effect with a reach that is not banded, a `maps_input` that turns off the
  region, `frame.rows()` where `rows_in_place` or `map` would do (it copies its band).

**Memory grows**
- `vmmap -summary <pid>` on a stopped process (`kill -STOP`); look for "MALLOC_LARGE (empty)". Then look
  for buffers built by `collect()` or a full-frame `vec!` per call.

**A plugin is wrong**
- The test kit: stored frames (`assert_frame!`, `UPDATE_FRAMES=1` to store), the band check (renders in
  bands and whole and compares, which catches a reach that is too small), and the time check.
- `cargo xtask plugin check` runs the app's load validation and one render per effect.
- `plugin dev` streams the plugin log with file and line for warnings and panics; the journal keeps it.
- Compare CPU and GPU paths; on Metal, divisions near 1e-36 and denormals differ.

**A plugin is stale**
- Is the dependence declared (`time_dependent`, `reads_scene`, `reads_other_frames`)? The time check names
  a missing time flag.
- What does the cache key cover? A file read through `files.read` outside the plugin's folders is not in
  it. Test through a `LayerCache`.
- After a reload, the plugin's revision changes every key that runs it; if not, look at the revision hash.
