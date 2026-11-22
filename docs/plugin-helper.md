# Plugin helper process

The design for running plugins in a separate, sandboxed helper process, for the agent who builds it.
The numbers come from a throwaway prototype, `pluginkit/kit/helper-proto` (its own Cargo project;
`cargo build --release --offline`, then `target/release/helper-proto [rtt|idle|frames|helper|probe|fdpass]`),
with the raw output in its `run-*.txt` files.

Machine: Apple M5, 10 cores, Darwin 25.5. Load average 26 to 34 for every run (`uptime` before and
after each; other agents were building). Minimums and medians hold; p99s are scheduler noise.

## Why

Native plugins run at full speed inside an OS sandbox, and a crash ends only the helper. The app ends
a hang by killing the helper, so WebAssembly there needs no epoch checks (8 to 35 % of sandboxed
compute, `reports/speed-study.md`). Frames live in shared memory that stays mapped in both processes.

## What was measured

### Round trip of one request and reply (µs, best of 3 rounds of 20,000 after 1,000 warm-up)

Each child ran under the strict profile below. "Host call" means the helper calls the app during a
render and waits for the answer: 2,000 calls, timed in the helper.

| Transport | Bytes | min | p50 | p99 | Host call min | p50 | p99 |
|---|---|---|---|---|---|---|---|
| Unix socket (`socketpair`) | 200 | 2.88 | 4.92 | 12.1 | 2.96 | 6.46 | 10.9 |
| | 4096 | 3.38 | 8.21 | 17.4 | 3.50 | 4.54 | 13.3 |
| Pipe pair | 200 | 2.79 | 5.54 | 12.0 | 2.96 | 5.50 | 8.6 |
| | 4096 | 3.25 | 7.54 | 17.3 | 3.62 | 6.88 | 17.0 |
| Shared memory + POSIX named semaphore | 200 | 2.25 | 6.42 | 16.4 | 3.38 | 5.25 | 14.2 |
| | 4096 | 2.50 | 5.96 | 20.3 | 2.46 | 5.75 | 11.3 |
| Shared memory + `os_sync_wait_on_address` (SHARED) | 200 | 0.71 | 1.17 | 14.9 | 0.71 | 6.04 | 15.5 |
| | 4096 | 0.96 | 8.08 | 16.0 | 1.00 | 1.96 | 17.8 |
| Shared memory, spin 20 µs then `os_sync_wait` | 200 | 0.67 | 0.88 | 1.38 | 0.75 | 0.79 | 1.25 |
| | 4096 | 0.79 | 1.21 | 1.79 | 0.83 | 1.12 | 1.25 |
| Shared memory, pure spin | 200 | 0.17 | 0.25 | 0.29 | 0.21 | 0.25 | 0.29 |
| | 4096 | 0.58 | 0.83 | 1.54 | 0.67 | 0.71 | 0.79 |

These figures are for back-to-back messages, when the other side is still awake. A new render call
usually reaches a helper that has been idle. That case was measured with a 1 ms pause before each
200-byte round trip (`idle`; best of 3 rounds of 2,000):

| Transport | min | p50 | p99 | p50 with user-interactive QoS |
|---|---|---|---|---|
| Unix socket | 3.46 | 7.83 | 52 | 15.96 |
| Pipe | 3.17 | 9.83 | 103 | 14.58 |
| Semaphore | 4.21 | 13.54 | 3,175 | 13.21 |
| `os_sync_wait` | 4.38 | 13.12 | 3,861 | 15.38 |
| Spin, then `os_sync_wait` | 3.50 | 8.92 | 3,337 | 9.42 |
| Pure spin | 0.25 | 0.92 | 3,817 | 1.17 |

**What this means.** Once a thread has slept, waking it costs 3.5 to 13 µs whatever the transport.
Shared memory wins only in bursts: about 0.8 µs instead of about 5 µs per message. A frame cut into 10
bands makes 10 calls, about 80 µs of CPU spread over 10 threads, against renders that take
milliseconds. Pure spinning burns a core for each waiting lane and gets descheduled under load (p99
3.8 ms). **Decision:** lanes use Unix sockets. Sockets also report a dead helper as end of file and
carry file descriptors. A spin-then-wait shared-memory lane (macOS 14.4+) is a later swap behind the
same `Lane` type, only if a real workload spends more than 1 % of render time in host calls.

### Frames in shared memory (f32 RGBA, `shm_open` + `ftruncate` + `mmap`, best of 3 runs)

| | 1080p (33 MB) | 4K (132 MB) |
|---|---|---|
| App: create and map | 0.014 ms | 0.011 ms |
| App: first write of every page | 3.13 ms, 2,025 faults | 8.08 ms, 8,100 faults |
| Helper: open and map | 0.008 ms | 0.007 ms |
| Helper: first pass over the frame | 2.96 ms, 2,025 faults | 14.2 ms, 8,100 faults |
| Steady pass in the helper, shared memory vs private memory | 0.74 vs 0.77 ms | 3.05 vs 3.11 ms |
| Steady copy in and copy out in the app (one thread) | 0.68 and 0.73 ms | 2.73 and 2.83 ms |
| Page faults over 30 steady frames, app and helper | 0 and 0 | 0 and 0 |

The page size is 16 KiB, so there is one fault per page on first touch in each process and none after.
Working on shared memory costs the same as working on private memory. The pool therefore pays its
fault cost once per slab per process. Remapping each frame is slower than copying
(`speed-study.md` §6); this design never remaps.

`fdpass`: the app passed the slab's file descriptor to a sandboxed helper over the socket
(`SCM_RIGHTS`). The helper could `mmap` it even though its profile grants no shared-memory name
(opening the same name was denied). Values written on either side were seen on the other.
`ftruncate` from the helper was refused with `EINVAL`, because macOS does not let a POSIX shared-memory
object change size. A helper therefore cannot shrink a slab to make the app crash with `SIGBUS`.

### Starting, loading, killing (ms, minimum of 15: 3 runs of 5)

The helper was the prototype binary, about the size a real helper would be: wasmtime, libloading and
libc. The test plugin was Plasma Beam: its native `.dylib`, and `plugin.wasm` (486 KB) precompiled by the
app with epoch checks off.

| Step | Time |
|---|---|
| `posix_spawn` until the helper says ready, sandbox applied | 8.73 |
| ...of which `sandbox_init_with_parameters` (compiling the profile) | 4.52 |
| Native: `dlopen`, symbols, `craft_plugin_init` | 0.42 |
| Native: first `describe` call, real reply | 0.055 |
| WebAssembly: `Engine::new` | 0.12 |
| WebAssembly: `Module::deserialize_file` + instantiate + ABI check | 0.27 |
| WebAssembly: first `describe` call, real reply | 0.069 |
| `SIGKILL` until the process is reaped | 0.37 |
| Restart until a WebAssembly plugin is ready to serve | 9.11 |
| App: compile `plugin.wasm` without epoch checks (once, then cached) | 201 |

Resident memory of one helper: 4.8 MB idle, 5.3 MB with the native plugin loaded, and 7.7 MB with the
WebAssembly module loaded too. Plugin heaps and frame slots come on top of that.

### The sandbox really denies

`probe` runs the same attempts with no profile, then with each profile. With no profile everything
succeeds. With `strict`:

- **Allowed:** reading `bundle/asset.txt`, writing `data/out.txt`, `dlopen` of the bundle's dylib.
- **`EPERM`:** reading a file outside, `/etc/hosts`, listing `$HOME`; writing outside, into the app's
  folder or into `/private/tmp`; TCP connect to a listener on 127.0.0.1; UDP send; running `/bin/echo`;
  `shm_open` of another name or `O_CREAT` of a new one. `dlopen` of the same dylib copied outside fails
  with "blocked by sandbox".

The `files-read` profile also reads outside and lists `$HOME`, but still cannot read the app's folder.
`files-write` also writes outside, but still cannot write the app's folder. This shows that the later
rule wins. The kernel logged every denial, for example `deny(1) network-outbound remote:*:50390`,
`deny(1) file-write-create /private/tmp/cpk-planted.txt`, `deny(1) process-fork`. The 88 lines are in
`run-sandbox-log.txt`, taken from `/usr/bin/log show --predicate 'eventMessage CONTAINS "helper-proto"'`.

## Process model: one helper per plugin

Each loaded plugin gets its own helper. The reasons:

- **Killing has to be precise.** A hang in one plugin must not fail every other plugin's renders that are
  in flight.
- **Profiles are per plugin.** Grants differ (`files.read`, the plugin's own folders). A shared helper
  would need the union of all grants.
- **Plugins cannot read each other's memory or frames.**
- **The cost is small.** It is measured at 4.8 to 7.7 MB and 9 to 12 ms per plugin. Twenty plugins add
  roughly 100 to 160 MB and start in parallel (an estimate from the per-helper figures; 20 helpers at once
  were not measured).

Helpers start when the plugin loads, because `Describe` runs then anyway. They stay up until the plugin
is unloaded or the app quits.

The helper is a separate small binary, `craft-plugin-helper`
(`crates/plugin-host/src/bin/craft-plugin-helper.rs`), installed next to the app executable. Re-running
the app binary itself would load every framework the app links, and the measured start-up and memory
figures would not hold.

## How it fits the current seam

The JSON wire (`Call`, `HostCall`, `Outcome`) and the native and WebAssembly ABIs do not change. A
plugin built today loads unchanged. The builder's test that shows this is the probe plugin's existing
tests run with the helper on.

1. **Split the guest from the host side.** Today `Engine` holds both `Identity` (grants, cache, tasks,
   images; the app side) and `Kind` (the code that runs). Move `Kind::Wasm` and `Kind::Native`, with the
   stateful main instance and the instance pool, into a `Guest` with
   `Guest::call(request, io, serve: &mut dyn Serve, stateful, budget)`. `Engine` gets
   `enum Kind { Local(Guest), Helper(Helper) }`. The app process runs the `Local` arm exactly as it does
   today. The helper process runs a `Guest`.
2. **Add a `Serve` trait.** `native::serve_erased` and `wasm::host_call` call
   `serve(&mut self, request, bulk) -> (Vec<u8>, Bulk, Duration)` through the erased context instead of
   `guarded(&mut HostCx)`. In the app, `HostCx` implements it with `guarded` (`serve.rs` and `gate.rs`
   stay as they are). In the helper, `LaneServe` implements it by sending a `HostCall` up the lane and
   waiting for the reply.
3. **`Kind::Helper` in `Engine::call`.** Take a free lane, put the pixels in a pool slot, send `Call`,
   then loop. A `HostCall` from the helper is answered with
   `native::guarded(&mut cx, request, bulk)` on the calling app thread, so `on_task_thread`, forwarding
   to `pump`, the gate and `paused` all keep working. `Reply` copies the output back if the outcome is
   `Ok` and returns. `Failed` becomes `PluginError::Trap`. End of file or the deadline kills the helper
   (see Deadlines).
4. **`copies_pixels()` is true for `Kind::Helper`.** `Plugin::render` and `run_pass` then take the
   paths that sandboxed plugins take today: `Io::Region` bands with reach, and `Io::Pass` with
   `[src | aux | out]` laid out in one buffer. Native plugins in a helper get banding with reach for free
   (`speed-study.md` #1). The helper hands that one buffer to a native guest as its in-place pixels.
   Check with the probe plugin's native build that the SDK reads the layout from the `Call`, not from the
   build target. If it does not, map `spread` tables instead: the app sends offsets, and the helper
   rewrites them to its own addresses before the call.
5. **Tasks.** `Engine::start` keeps its thread in the app. That thread drives a task lane. Cancel stays a
   failed `Progress` host call. If a task is still running `GRACE` after cancel, the app kills the helper.
   This fixes "A native task that ignores cancel keeps its thread" in
   `docs/plugin-build/known-issues.md`; delete that entry in the commit that fixes it.

### The app-to-helper protocol (internal, not seen by plugins)

The control socket is passed at spawn as the only inherited descriptor. Lanes are `socketpair`s sent
over it with `SCM_RIGHTS`. There is one lane per app rayon thread, plus 1 interactive lane and
`MAX_RUNNING` (4) task lanes. Each lane has one helper thread. Every message has a 24-byte header,
`kind: u32, json_len: u32, bulk_len: u64, slab: u32, offset: u32`, then the JSON, then the bulk data
inline if it is at most 64 KiB. Larger bulk data goes in a slab (`slab != u32::MAX`).

| Direction | Kind | Meaning |
|---|---|---|
| app → helper | `Open` | Mode, entry path, compiled-module path, limits; sent once |
| app → helper | `Slab` + descriptor (control socket) | A new pool slab; the helper maps it and keeps it mapped |
| app → helper | `Call` | The `Call` JSON, `IoDesc { slab, offset, src, aux, out }`, `stateful`, `task` |
| app → helper | `HostReply` | The answer to a `HostCall` |
| app → helper | `Release` | Unmap a slab |
| helper → app | `Ready` | After the sandbox is applied and the plugin is loaded |
| helper → app | `HostCall` | The plugin's host-call JSON and bulk data |
| helper → app | `Reply` / `Failed` | The plugin's reply bytes, or a trap message |
| helper → app | `Grow` | Asks for a slab of at least N bytes; the helper cannot create shared memory |

Host-call replies that carry pixels (`Layer`, `Source`, `Frame`) are written by the app into a slab.
For WebAssembly the copy count is the same as today. A native plugin receives the bulk data in its own
allocation, as the ABI requires, so it pays one extra copy.

### WebAssembly in the helper

- The helper engine uses `Config::new()` with `epoch_interruption(false)`. `cache_name` already hashes
  `precompile_compatibility_hash`, so its compiled files are cached apart from the in-process ones.
- The app compiles and writes the cache, as it does today. The helper gets the cache folder read-only,
  so a plugin cannot plant a compiled module. Another process of the same user still can, so that
  known-issues entry stays.
- `StoreLimits` stay. Nothing else limits time, because the deadline lives in the app.
- To keep the frame copies at today's two (app into linear memory, and back), give the helper engine
  `Config::with_host_memory(Arc<SlabMemory>)` (wasmtime 51 `MemoryCreator` / `LinearMemory`). Each
  instance's linear memory is a slab the app created at the store's maximum size (sparse, so pages cost
  only when touched; one `Grow` per new instance). The app then copies the band directly to
  `slab + pixel_buffer_offset`, an offset the helper reports in `Reply`. Without this, WebAssembly in the
  helper copies four times. This step is designed, not measured.

## Frame pool

- **The app owns it.** It is a list of slabs per helper. On macOS a slab is `shm_open` of a random name
  with `O_EXCL`, then `shm_unlink` at once, `ftruncate`, and `mmap`. On Linux it is
  `memfd_create(MFD_CLOEXEC | MFD_ALLOW_SEALING)`, `ftruncate`, then
  `F_ADD_SEALS(F_SEAL_SHRINK | F_SEAL_GROW | F_SEAL_SEAL)`, so a helper cannot cause a `SIGBUS` in the
  app. On Windows it is an unnamed `CreateFileMappingW`. The descriptor or handle goes to the helper
  once. No name stays behind if the app crashes; macOS cannot list leaked names, so this matters.
- **Slots.** Slots are frame-sized and rounded up to 1 MiB, with a free list by size. A call holds a slot
  until its reply. Keep at most 2 × the bands in flight × the largest frame size; release slabs that have
  been idle for 30 s (`Release`, then `munmap` on both sides).
- **Lifetime.** A slab outlives any helper. When a helper dies, the app keeps its slabs and mapping and
  passes the same descriptors to the next helper. That helper pays one first-touch pass, measured at
  2.96 ms for 1080p and 14.2 ms for 4K.
- **A helper that dies mid-render** leaves garbage in its slot. The app discards it. The app's own frame
  is untouched, because output is copied back only after an `Ok` outcome. For native plugins in a helper,
  this fixes "Native plugins can leave a partly written frame" and "A failed band leaves a partly
  processed frame" (all bands are copied back after all succeed; change the band loop in `Plugin::render`
  to commit only then).
- **Later: zero copy for native.** The render crate allocates layer buffers from the pool, so a native
  plugin in a helper works on the app's own buffer. Then the band must render into a scratch slot and
  be swapped in, so that failure still leaves the frame whole.

## Deadlines, kill and restart

- **The budgets do not change.** `Limits` stays at 2 s for interactive calls and 60 s for work. The app
  thread waits on its lane with `poll` and a timeout of the budget remaining. Time spent serving host
  calls counts toward the budget, as today; `cx.paused` (the file dialog) extends the deadline.
- **On a timeout**, the app sends `SIGKILL` to the helper (`TerminateJobObject` on Windows) and waits for
  it. Every other lane in flight sees end of file and fails with
  `PluginError::Trap("<plugin> stopped responding and was restarted")`. Running tasks fail with the same
  message.
- **On a crash** (a segfault in native code, an abort, an out-of-memory kill), the helper's sockets close.
  A thread in the app is told at once, and so is the reaper that reads the control socket. The failure
  is the same as above, with "crashed" instead of "stopped responding".
- **Restart** happens on the next call: spawn, `Open`, `Ready`, about 9 to 12 ms. After a restart the
  plugin has no state from `cx.remember` and no stateful panel instance, the same as after a trap today.
  The `Cache` lives in the app's `Identity`, so it survives.
- **Repeated crashes:** three deaths within 60 s disable the plugin until the user enables it again in
  the plugin manager. The manager shows the last error.
- **What the user sees:** the effect shows the render error it shows today. The app shows one
  notification per plugin per minute through `Host::notify`, for example "Plasma Beam stopped responding
  and was restarted". The plugin manager shows the restart count. A panel that times out shows its error
  in place of its widgets.

## The sandbox on each OS

On every OS, the profile is made from the manifest when the helper is spawned. The helper applies it as
the first thing it does, before it loads any plugin code. The SDK's file calls (`read_file`,
`write_file`, `list_files`, `pick_file`) are host calls that the app serves with today's gate. They work
the same with or without the OS rules below. The OS rules only limit what a native plugin's own code
can do directly through the OS.

Spawn with no inherited descriptors except the control socket. A descriptor opened before the sandbox
is not checked by it. On macOS, use `posix_spawn` with `POSIX_SPAWN_CLOEXEC_DEFAULT` and one
`adddup2`. On Linux, use `close_range(3, ~0, 0)` in `pre_exec` after moving the socket to fd 3. On
Windows, use `PROC_THREAD_ATTRIBUTE_HANDLE_LIST`. Clear the environment. `std::process::Command` does
not close inherited descriptors.

### Permissions on each OS

| Grant | macOS (Seatbelt) | Linux (Landlock + seccomp) | Windows (AppContainer + job) |
|---|---|---|---|
| Always | Read and execute the bundle, read the compile cache, read and write the plugin's data folder | Same, plus read and execute `/usr`, `/lib`, `/lib64` | ACEs for the container SID: read and execute the bundle and cache, full access to the data folder |
| `files.read` | Read everywhere except the app's folders | Read `/`; Landlock cannot exclude the app's folders | Nothing more. Only the SDK calls reach user files |
| `files.write` | Write everywhere except the app's folders | Write to every sibling along the path to each app folder, but not to that folder (a split tree) | Nothing more. Only the SDK calls reach user files |
| `document.*` | Nothing; served by the app | Nothing | Nothing |
| Network, processes, GPU devices | Denied | Denied | Denied (no capabilities; child processes restricted) |

Native plugins that need more (a vendor library that opens files by path, a licensing daemon, direct
Metal or Vulkan access) ask for one new permission, `system`: a new `Permission::System` variant,
`"system"` in `plugin.json`. It is allowed only for native plugins and appears in the native approval.
Its helper runs with no OS sandbox. It is still a separate process, so it keeps crash and hang
isolation. This is the one behaviour change: a native plugin that today opens sockets or runs processes
with its own code needs `"system"` added. No reference plugin does that; `grep` for `std::fs`,
`std::net` and `std::process` in `plugins/*/src` finds nothing. **The owner should confirm this
default.** The alternative is that every native plugin gets `system`, and only WebAssembly is sandboxed.

### macOS

Tested here. The helper calls `sandbox_init_with_parameters(profile, 0, params, &error)` with the
parameters `BUNDLE`, `CACHE`, `DATA`, and one `APP0..APPn` per app folder (`app_folders` in
`runtime/mod.rs`, plus the compile cache). This function is undocumented but stable; Chromium uses the
same family. Pass paths through `canonicalize`, because `/tmp` is `/private/tmp`. Later rules win (shown
above), so the grants come last. The plugin's own folders sit inside the app's folders, so they must
come after the denials:

```
(version 1)
(deny default)
(allow sysctl-read)
(allow file-read* (subpath "/"))                                    ; files.read only
(allow file-write* (subpath "/"))                                   ; files.write only
(deny file-read* file-write* (subpath (param "APP0")))              ; one per app folder
(allow file-read* file-map-executable (subpath (param "BUNDLE")) (subpath (param "CACHE")))
(allow file-read* file-write* (subpath (param "DATA")))
```

Write this without the `;` notes. The prototype tested the first line, the denial and the last two lines.
It also had `ipc-posix-shm` and `ipc-posix-sem` rules for the transports it compared. The descriptor
passing test shows that the helper does not need them. Compiling the profile costs 4.5 ms of each start.
If start time matters, cache the compiled profile (`sandbox_compile_string` / `sandbox_apply`, also
undocumented; not tried).

### Linux

Not tested here.

- **Landlock**: the `landlock` crate 0.4, MIT OR Apache-2.0. Use a version at least two weeks old;
  0.4.8 is from 2026-11-20. Handle every filesystem access right of the best ABI the kernel has, and add
  the rules in the table. On ABI 4 or later, also handle `NetPort` with no rules, which denies TCP bind
  and connect.
- **seccomp**: the `seccompiler` crate 0.5, Apache-2.0 OR BSD-3-Clause. It is pure Rust and needs no
  C libseccomp (that library is LGPL-2.1; the `libseccomp` crate binds it). Use a deny list that
  returns `EPERM`: `socket` for any domain except `AF_UNIX`; `execve`, `execveat`, `fork`, `vfork`,
  `clone` without `CLONE_THREAD`; `clone3` (return `ENOSYS` so glibc falls back to `clone`); `ptrace`,
  `process_vm_readv`, `process_vm_writev`, `mount`, `unshare`, `setns`, `bpf`, `perf_event_open`,
  `keyctl`, `add_key`, `userfaultfd`; and `io_uring_setup`, because io_uring operations bypass seccomp.
  An allow list is stronger, but it breaks with glibc updates.
- **Process setup:** call `prctl(PR_SET_NO_NEW_PRIVS)` before both. `PR_SET_PDEATHSIG(SIGKILL)` makes the
  helper die with the app. Use `pidfd_open` for the reaper.
- **Crates for descriptors:** `rustix` (Apache-2.0 WITH LLVM-exception OR Apache-2.0 OR MIT) for
  `memfd`, seals, `SCM_RIGHTS` and `pidfd`.
- **If the kernel has no Landlock** (before 5.13, or turned off): run with seccomp only, and show
  "file sandbox unavailable on this system" in the manager. WebAssembly is not affected, because it
  makes no system calls.

### Windows

Not tested here. Use the `windows` crate (MIT OR Apache-2.0) at the version the workspace locks. An
AppContainer cannot be given "every file", hence the table's `files.*` row.

- **AppContainer:** call `CreateAppContainerProfile` once per plugin, named `craft.plugin.<hash of id>`,
  with no capabilities. No `internetClient` means no network. Start the helper with `CreateProcessW` and
  `STARTUPINFOEXW` carrying `PROC_THREAD_ATTRIBUTE_SECURITY_CAPABILITIES`,
  `PROC_THREAD_ATTRIBUTE_HANDLE_LIST` (the lane pipe only) and
  `PROC_THREAD_ATTRIBUTE_CHILD_PROCESS_POLICY = PROCESS_CREATION_CHILD_PROCESS_RESTRICTED`.
- **Files and job:** ACEs for the container SID on the bundle, cache and data folders
  (`SetNamedSecurityInfoW`, at install). A job object with `KILL_ON_JOB_CLOSE`,
  `DIE_ON_UNHANDLED_EXCEPTION` and `ACTIVE_PROCESS = 1`; kill with `TerminateJobObject`.
- **Lanes and frames:** lanes are anonymous pipes, or `AF_UNIX` on Windows 10 1803 and later.
  `WaitOnAddress` does not work across processes, so do not use it. Frame sections go to the helper with
  `DuplicateHandle`. The reaper uses `WaitForSingleObject` on the process handle.

## GPU and the web build

- **GPU plans keep running in the app.** A `GpuPlan` comes back over the lane as JSON and the app's
  device runs the WGSL, as today. The helper has no GPU access unless the plugin has `system`.
- **Web build:** no helper. `runtime` is `unavailable.rs` there today; the helper is
  `cfg(not(target_family = "wasm"))`. WebAssembly plugins on the web run in-process with epoch checks.
- **Fallback:** on desktop, if the helper cannot start (binary missing, sandbox refused), the plugin runs
  in-process as today, with epoch checks for WebAssembly and the existing native approval. The manager
  says "running without process isolation". `CRAFT_PLUGIN_HELPER=0` forces this path for tests and
  debugging.

## Build order

1. Split `Guest` from `Engine` and add the `Serve` seam. No behaviour change; the existing
   `plugin-host` tests pass unchanged.
2. Build the helper binary, socket lanes, control socket, slabs and the macOS profile, with **native**
   plugins in the helper. Tests with the probe plugin's native build: an invert render through the
   helper is bit-identical to the in-process one; a spinning call is killed at a 200 ms budget and the
   next call succeeds; an aborting call returns `Trap` and the next call succeeds; a probe command that
   calls `std::fs::read` outside its folders returns "Operation not permitted".
3. Move WebAssembly into the helper without epoch checks, with slab-backed linear memory. Measure Plasma
   Beam and Lumen Glow against `speed-study.md` (the target is the "timeout checks off" row).
4. Linux, then Windows.
5. Allocate render buffers from the pool, for zero copy with native plugins.

## Not measured or tested here

- Nothing ran on Linux or Windows.
- The whole design was measured under load 26 to 34. Repeat the p99 values and the idle round trip on
  an idle machine.
- No real render went through a helper, only `describe`. The helper's WebAssembly speed is taken from
  `speed-study.md`, not measured again.
- Ten lanes busy at once (one frame in bands), 20 helpers starting at once, and the slab-backed
  WebAssembly linear memory were not measured.
- The prototype was ad-hoc signed and ran without the hardened runtime. Under the hardened runtime,
  wasmtime's executable memory probably needs `com.apple.security.cs.allow-jit`, or
  `allow-unsigned-executable-memory`, on the helper. Notarisation of a second executable was not tried.
- Not tried: GPU or Metal access from a sandboxed helper, caching the compiled sandbox profile, and
  `os_sync_wait_on_address` before macOS 14.4 (sockets avoid the question).
