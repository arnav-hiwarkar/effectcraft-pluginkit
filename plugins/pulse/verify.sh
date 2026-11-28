#!/usr/bin/env bash
set -euo pipefail
HERE="$(cd "$(dirname "$0")" && pwd)"
source "$HERE/../lib.sh"
export EFFECTCRAFT_CONFIG_DIR="$HERE/.scratch-config"
mkdir -p "$EFFECTCRAFT_CONFIG_DIR"

BUNDLE="$(xtask_plugin "$HERE" --out "$HERE/dist")"
BUNDLE="${BUNDLE%%$'\n'*}"

CLI="$CLI" BUNDLE="$BUNDLE" python3 -I - <<'PY'
import json, os, subprocess, sys, time

CLI = os.environ["CLI"]
BUNDLE = os.environ["BUNDLE"]
PLUGIN = "org.craftkit.pulse"
SECONDS = 60
FPS = 30
failures = []


def check(name, ok, detail=""):
    print(("PASS " if ok else "FAIL ") + name + (f"  ({detail})" if detail else ""))
    if not ok:
        failures.append(name)


def session(*steps):
    setup = [
        ("comp.new", {"name": "T", "width": 320, "height": 180, "duration": SECONDS, "frameRate": FPS}),
        ("layer.newSolid", {"name": "Beat"}),
        ("effect.apply", {"layers": ["Beat"], "effect": "ec.audio.tone"}),
        ("layer.setSwitch", {"layers": ["Beat"], "switch": "audio", "value": True}),
        *[("prop.set", {"layer": "Beat", "path": f"effects/#1/frequency{i}", "value": 0}) for i in range(2, 6)],
        ("prop.set", {"layer": "Beat", "path": "effects/#1/frequency1", "value": 100}),
        ("prop.addKey", {"layer": "Beat", "path": "effects/#1/level", "time": 0, "value": 0}),
        ("prop.addKey", {"layer": "Beat", "path": "effects/#1/level", "time": SECONDS, "value": 100}),
        ("plugin.load", {"path": BUNDLE}),
    ]
    args = [CLI, "run"]
    for command, params in [*setup, *steps]:
        args += [command, json.dumps(params)]
    started = time.monotonic()
    out = subprocess.run(args + ["--empty", "--json"], capture_output=True, text=True)
    elapsed = time.monotonic() - started
    data = json.loads(out.stdout)
    if isinstance(data, dict):
        return data, elapsed
    return [step.get("result") for step in data[len(setup):]], elapsed


def analyse(write, band=(60, 250)):
    args = {"layer": "Beat", "low": band[0], "high": band[1], "fps": FPS, "duration": SECONDS, "write": write}
    return ("plugin.startTask", {"plugin": PLUGIN, "task": "analyse", "args": args})


WAIT = ("plugin.tasks", {"wait": True, "timeout": 300})
TARGET = "Pulse 60-250 Hz"
SAMPLES = [15, 30, 45]

base_time = min(session(("plugin.list", {}))[1] for _ in range(3))
runs = [session(analyse(False), WAIT) for _ in range(3)]
timed = runs[0][0]
timed_time = min(elapsed for _, elapsed in runs)
if isinstance(timed, dict):
    check("the analysis of 60 s of audio finishes", False, timed.get("error"))
else:
    state = timed[1][-1]
    check("the analysis of 60 s of audio finishes", state["status"] == "done", f"{state['status']} {state.get('error')}")
    values = state["result"]["values"]
    check("one value per comp frame", len(values) == SECONDS * FPS, f"{len(values)} values")
    print(f"MEASURE analysing {SECONDS} s of audio at 48 kHz: {timed_time - base_time:.2f} s (best of 3 runs with analysis {timed_time:.2f} s, without {base_time:.2f} s)")

r, _ = session(analyse(True), WAIT, *[("prop.get", {"layer": TARGET, "path": "effects/#1/slider", "time": t}) for t in SAMPLES],
               ("edit.undo", {}), ("prop.get", {"layer": TARGET, "path": "effects/#1/slider", "time": 30}))
if isinstance(r, dict):
    check("the band energy is written as keyframes that follow the tone's level", False, r.get("error"))
else:
    state = r[1][-1]
    got = [v["value"] for v in r[2:2 + len(SAMPLES)]]
    expected = [t / SECONDS * 100 for t in SAMPLES]
    error = max(abs(a - b) for a, b in zip(got, expected))
    check("the band energy is written as keyframes that follow the tone's level", state["status"] == "done" and error < 3, f"slider {got}, expected {expected}")
    check("a keyframe for every frame", state["result"]["keys"] == SECONDS * FPS, f"{state['result']['keys']} keys")
    undone = r[2 + len(SAMPLES)]
    after = r[3 + len(SAMPLES)]
    check("one undo removes every keyframe", undone == {"undone": f"{TARGET}: Keyframes"} and after["value"] == 0, f"undo {undone}, slider after {after}")

r, _ = session(analyse(False), ("plugin.cancelTask", {"plugin": PLUGIN, "id": 1}), WAIT)
if isinstance(r, dict):
    check("cancel stops the analysis", False, r.get("error"))
else:
    check("cancel stops the analysis", r[2][-1]["status"] == "cancelled", r[2][-1]["status"])

sys.exit(1 if failures else 0)
PY
