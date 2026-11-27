# Plugin system: what the third build did and what is left for later

Written 2026-11-23, at the end of the third build. The log of every decision and how it was checked is
[decisions.tsv](decisions.tsv); what is known to be wrong or missing is [known-issues.md](known-issues.md); the
measured speed before and after is [speed.md](speed.md).

## Built in the third build

| Area | What a plugin author or user gets |
|---|---|
| Speed | Native effects that read neighbours run on all cores. An effect that reads no other pixels changes the frame in place. Bands never render a row twice. Only the part of a layer that shows in the comp is rendered. |
| Host kernels | Blur, resample, blend, curves, convolve, warp, colour matrix, transfer curves, brightness curve, dither and light alpha run in the app, natively on the CPU and as its own WGSL on the GPU, from one plan. Consecutive pointwise kernels are fused. Lumen Glow has no rendering code of its own. |
| Fast maths | `craft_plugin::math` and `math::fast`: inline functions with tested error bounds, the same bits natively and in the sandbox. |
| Panels | The app keeps a panel's last widget tree and calls the plugin only on an event, a subscribed app event, a timer or a resize, on the plugin's own worker thread. Keyboard use, every pointer event, clipboard behind a permission, drag and drop. |
| On the viewer | Effects draw overlays and handles bound to parameters; plugins add viewer tools. |
| Safety | Picked files are checked by file identity, a dropped file is a pick, tasks belong to their project, the compiled-module cache is authenticated, a failed plan leaves nothing behind, one identity check for every interactive request. |
| Authoring | `plugin new --kind`, `params!` typed parameters, the test kit with stored frames, `plugin render`, `bench`, `check` and `dev`, a quickstart that was run from a fresh clone and a cookbook whose every snippet is compiled. |
| App | Auto-saves keep the open comps and their origin; a recovered project saves over its original. Five built-in effects quantise as they write. A comp in which nothing animates serves frames from memory. Layer cache keys hash values. Four built-in effects that change with time are now declared so. |

## Plugin UI programme (2026-11-26/27)

Plan, contract and decisions: `pluginkit/PROGRAM-UI.md`; owner summary: `pluginkit/reports/ui-lead.md`.
- Built: panel places, sizes, icons and help; floating windows; status items; tool option bars; numbered
  panel instances; virtualised lists, tables and trees with keyboard work, rename, reorder and sort; icon
  buttons, toggles, radios, text areas, rich text, gaps, menus and context menus, split panes, curve, gradient
  and colour editors drawn by the app, app-rendered frame previews; commands in any menu, context menus and the
  toolbar, with enabled and checked state; panel keys and panel menus; one undo step per panel drag; right-click
  and double-click on canvases and tools; pen pressure, tilt, eraser and rotation on macOS; worker lanes so a slow
  panel call does not hold back overlays and tools; a UI level so newer events reach only plugins that know them.
- The framework painter (`craft-plugin-ui-framework`) lives on craft-libs `pluginkit-next`; the same 36
  conformance cases pass on both painters. The EffectCraft port checklist is in `reports/ui-lead.md`.
- Left: pen data on Windows, Linux and the web; a sandboxed command still waits for a running panel call;
  `known-issues.md` lists the rest.

## Deferred by the owner, for a later run

| Unit | Why it waits | What exists |
|---|---|---|
| Plugin helper process with an OS sandbox | The largest unit. It pays off once other people's plugins exist: a crash or hang is ended by killing the helper, native plugins become safe, and sandboxed code needs no timeout checks (8 to 35 % of its compute). | The design and its measurements: [plugin-helper.md](../plugin-helper.md). Several known issues name it as their cure. |
| Plugins in the comp's 3D scene | Large and uncertain; only Forge-style plugins need it. | Nothing yet. |
| Model inference (`ai.run`) | The app has no general model runner; it means a heavy new dependency. | Nothing yet. |
| Importers and Render Queue output modules | Large; no current plugin needs it. | Nothing yet. |
| A C and C++ header | Matters when someone ports an existing After Effects plugin. | The wire protocol is one JSON call each way and costs under 0.2 % of render time, so a header can wrap it as it is. |
| Audio effects, and audio inside a render | Nothing depends on them yet. | A written brief (kept with the build notes). |
| Services between plugins, localisation of plugin text | Nothing depends on them yet. | Nothing yet. |
| Network access from the sandbox | Decided: not for now. | A native plugin can; a sandboxed one cannot. |

## Decisions the owner made

- **Native plugins are sandboxed by default once the helper process exists** (decided 2026-11-23). Every native
  plugin runs in its own helper with the OS rules in `docs/plugin-helper.md`: no network, no other programs, files
  only in its own folders unless it holds `files.read` or `files.write`. A native plugin that needs more asks for
  the `system` permission; its helper runs without OS rules, still keeps crash and hang isolation, and the install
  approval says it has full access to the computer. Why: a sandbox can be loosened later without breaking plugins
  but not added later; `system` keeps every capability; the rules cost about 4.5 ms per helper start and nothing
  per render; none of the nine reference plugins needs `system`. With it, `cargo xtask plugin check` and the dev
  log name the `system` permission when a call is refused by the OS.

## Removed again during the build, and why

- **A time limit on plugin GPU work.** Built in wave 1, repaired three times, removed: every review found a new
  way for it to stop the wrong plugin, and a correct limit needs per-plan GPU timestamps.
- **A new frame for every render.** It guaranteed a failed effect could not leave a half-processed frame, and
  it made every light native effect 20 to 130 % slower. Effects that read no other pixels render in place again.
- **Abandoning a slow file call at the time limit.** A file write cannot be cancelled, so the abandoned write
  could land after the plugin was told it failed.

## Later: plugin manager, distribution and a store (owner, 2026-11-24)

After the plugins and the shared base work as the owner wants on EffectCraft. Kept from the old platform's study
(`pluginkit/reports/w8-old-system-study.md`, section 3(b) and (c)):
- a signed catalogue index (ed25519 over the exact bytes, each row pinning sha256 and size) with a kill list;
- versions installed side by side: one-step rollback, optional pins, an update that widens access waits for approval;
- a manifest fit to publish: `apps`, `requires`, licence, source, a reason per permission; release-record and submission-check tools;
- origin tracking (file or catalogue; a change asks again);
- Problems and Updates in the manager, over a toolkit-free manager model in craft-libs that every app draws;
- for all apps: per-app contracts, a shared command vocabulary, the helper process, brushes, translated plugin text, a web runtime where an app's plugins run in the browser, JavaScript plugins only if script authors are a target.

Not taken (study section 4): the wasmi interpreter and fuel, the 100-string permission ladder, a per-frame GPU time
budget, typed binary calls, mapping frames into plugin memory, the Adobe and Figma compatibility layers as kit work,
install from GitHub, and store polish.
