#!/usr/bin/env bash
set -euo pipefail
HERE="$(cd "$(dirname "$0")" && pwd)"
CLI="${CLI:-$HERE/../../target/debug/effectcraft-cli}"
export EFFECTCRAFT_CONFIG_DIR="$HERE/.scratch-config"
mkdir -p "$EFFECTCRAFT_CONFIG_DIR"

cd "$HERE"
cargo build --offline --release --target wasm32-unknown-unknown --quiet
mkdir -p dist
cp plugin.json dist/
cp target/wasm32-unknown-unknown/release/glide.wasm dist/plugin.wasm

CLI="$CLI" BUNDLE="$HERE/dist" python3 -I - <<'PY'
import json, math, os, subprocess, sys

CLI = os.environ["CLI"]
BUNDLE = os.environ["BUNDLE"]
PLUGIN = "org.craftkit.glide"
DEFAULT = (0.42, 0.0, 0.58, 1.0)
EASY = (1 / 3, 0.0, 2 / 3, 1.0)
EASE_IN = (0.42, 0.0, 1.0, 1.0)
LINEAR = (1 / 6, 1 / 6, 5 / 6, 5 / 6)
failures = []


def bez(p1, p2, u):
    v = 1 - u
    return 3 * v * v * u * p1 + 3 * v * u * u * p2 + u ** 3


def ease(curve, t):
    x1, y1, x2, y2 = curve
    lo, hi = 0.0, 1.0
    for _ in range(100):
        mid = (lo + hi) / 2
        if bez(x1, x2, mid) < t:
            lo = mid
        else:
            hi = mid
    return bez(y1, y2, (lo + hi) / 2)


def check(name, ok, detail=""):
    print(("PASS " if ok else "FAIL ") + name + (f"  ({detail})" if detail else ""))
    if not ok:
        failures.append(name)


def session(*steps):
    args = [CLI, "run", "comp.new", json.dumps({"name": "T", "width": 640, "height": 360, "duration": 6}),
            "layer.newSolid", json.dumps({"name": "S"}), "plugin.load", json.dumps({"path": BUNDLE})]
    for command, params in steps:
        args += [command, json.dumps(params)]
    out = subprocess.run(args + ["--empty", "--json"], capture_output=True, text=True)
    data = json.loads(out.stdout)
    if isinstance(data, dict):
        return data
    return [step.get("result") for step in data[3:]]


def key(path, time, value):
    return ("prop.addKey", {"path": path, "time": time, "value": value})


def select(path, *times):
    return ("keys.select", {"keys": [{"layer": "S", "prop": path, "time": t} for t in times]})


def run_plugin(command):
    return ("plugin.run", {"plugin": PLUGIN, "command": command})


def samples(path, t0, t1, n):
    return [("prop.get", {"path": path, "time": t0 + (t1 - t0) * k / n}) for k in range(n + 1)]


def values(results):
    return [r["value"] for r in results]


def key_times(info, path):
    return sorted(k["compTime"] for k in info if k["path"] == path)


def worst(errors):
    return max(errors) if errors else float("inf")


def scalar_progress_error(vals, v0, v1, times, t0, t1, curve):
    errors = []
    for v, t in zip(vals, times):
        x = (t - t0) / (t1 - t0)
        errors.append(abs((v - v0) / (v1 - v0) - ease(curve, x)))
    return worst(errors)


def sample_times(t0, t1, n):
    return [t0 + (t1 - t0) * k / n for k in range(n + 1)]


def one_dimension(command, curve, tolerance, label):
    n = 16
    r = session(key("transform/rotation", 0, 0), key("transform/rotation", 2, 90), select("transform/rotation", 0, 2.002),
                run_plugin(command), ("keys.info", {}), *samples("transform/rotation", 0, 2.002, n))
    if isinstance(r, dict):
        return check(label, False, r["error"])
    info = r[4]
    t0, t1 = key_times(info, "transform/rotation")
    vals = values(r[5:])
    err = scalar_progress_error(vals, 0, 90, sample_times(0, 2.002, n), t0, t1, curve)
    bezier = info[0]["out"] == "bezier" and info[1]["in"] == "bezier"
    check(label, err < tolerance and bezier, f"max progress error {err:.2e}, sides bezier={bezier}")


one_dimension("apply", DEFAULT, 1e-6, "1D rotation follows the panel curve cubic-bezier(0.42, 0, 0.58, 1)")
one_dimension("easy-ease", EASY, 1e-6, "1D rotation follows Easy Ease")
one_dimension("linear", LINEAR, 1e-9, "1D rotation follows Linear exactly")
one_dimension("ease-in", EASE_IN, 1e-3, "1D rotation follows Ease In (handle on its anchor: 0.1 % minimum influence)")

n = 16
r = session(key("transform/opacity", 0, 10), key("transform/opacity", 1.5, 80), select("transform/opacity", 0, 1.5015),
            ("keys.info", {}), run_plugin("apply"), ("keys.info", {}), ("edit.undo", {}), ("keys.info", {}))
if isinstance(r, dict):
    check("one undo restores the eases on opacity", False, r["error"])
else:
    before, applied, undone = r[3], r[5], r[7]
    check("one undo restores the eases on opacity", before == undone and applied != before and r[6] == {"undone": "Glide: Apply Curve"},
          f"undo step {r[6]}")

r = session(key("transform/scale", 0, [100, 50]), key("transform/scale", 3, [160, 250]), select("transform/scale", 0, 3.003),
            run_plugin("easy-ease"), ("keys.info", {}), *samples("transform/scale", 0, 3.003, n))
if isinstance(r, dict):
    check("2D scale follows Easy Ease in each dimension", False, r["error"])
else:
    t0, t1 = key_times(r[4], "transform/scale")
    vals = values(r[5:])
    times = sample_times(0, 3.003, n)
    ex = scalar_progress_error([v[0] for v in vals], 100, 160, times, t0, t1, EASY)
    ey = scalar_progress_error([v[1] for v in vals], 50, 250, times, t0, t1, EASY)
    check("2D scale follows Easy Ease in each dimension", max(ex, ey) < 1e-6, f"x error {ex:.2e}, y error {ey:.2e}")


def path_progress(points):
    cumulative = [0.0]
    for a, b in zip(points, points[1:]):
        cumulative.append(cumulative[-1] + math.dist(a[:2], b[:2]))
    return [c / cumulative[-1] for c in cumulative]


r = session(key("transform/position", 0, [100, 100]), key("transform/position", 2, [500, 300]), select("transform/position", 0, 2.002),
            run_plugin("apply"), ("keys.info", {}), *samples("transform/position", 0, 2.002, n))
if isinstance(r, dict):
    check("spatial position on a straight path follows the curve", False, r["error"])
else:
    t0, t1 = key_times(r[4], "transform/position")
    pts = values(r[5:])
    chord = math.dist([100, 100], [500, 300])
    errors = [abs(math.dist(p[:2], [100, 100]) / chord - ease(DEFAULT, (t - t0) / (t1 - t0))) for p, t in zip(pts, sample_times(0, 2.002, n))]
    check("spatial position on a straight path follows the curve", worst(errors) < 1e-6, f"max progress error {worst(errors):.2e}")

fine = 512
r = session(key("transform/position", 0, [100, 100]), key("transform/position", 1, [300, 260]), key("transform/position", 2, [520, 120]),
            select("transform/position", 0, 1.001, 2.002), ("keys.info", {}), run_plugin("apply"), ("keys.info", {}),
            *samples("transform/position", 0, 1.001, fine), *samples("transform/position", 1.001, 2.002, fine),
            ("edit.undo", {}), ("keys.info", {}))
if isinstance(r, dict):
    check("spatial position on a curved path follows the curve along its arc length", False, r["error"])
else:
    before, applied = r[4], r[6]
    first, second = values(r[7:8 + fine]), values(r[8 + fine:9 + 2 * fine])
    times = key_times(applied, "transform/position")
    errors = []
    for pts, (t0, t1), (s0, s1) in [(first, times[0:2], (0, 1.001)), (second, times[1:3], (1.001, 2.002))]:
        progress = path_progress(pts)
        for k in range(0, fine + 1, 32):
            t = s0 + (s1 - s0) * k / fine
            errors.append(abs(progress[k] - ease(DEFAULT, (t - t0) / (t1 - t0))))
    check("spatial position on a curved path follows the curve along its arc length", worst(errors) < 1e-4, f"max progress error {worst(errors):.2e}")
    middle = applied[1]
    check("the middle key of three is Bezier on both sides", middle["in"] == "bezier" and middle["out"] == "bezier", f"{middle['in']}/{middle['out']}")
    undone = r[10 + 2 * fine]
    check("one undo restores the eases on position", before == undone and r[9 + 2 * fine] == {"undone": "Glide: Apply Curve"}, f"undo step {r[9 + 2 * fine]}")

outer = lambda: [*samples("transform/rotation", 0, 1.001, n), *samples("transform/rotation", 2.002, 3.003, n)]
r = session(key("transform/rotation", 0, 0), key("transform/rotation", 1, 40), key("transform/rotation", 2, 100), key("transform/rotation", 3, 110),
            ("keys.selectAll", {}), ("keys.interpolation", {"interpolation": "autoBezier"}), *outer(),
            select("transform/rotation", 1.001, 2.002), run_plugin("apply"), *outer(), ("keys.info", {}))
if isinstance(r, dict):
    check("auto-Bezier neighbours keep their shape outside the selection", False, r["error"])
else:
    width = 2 * (n + 1)
    before = values(r[6:6 + width])
    after = values(r[8 + width:8 + 2 * width])
    selected = r[8 + 2 * width]
    drift = worst([abs(a - b) for a, b in zip(before, after)])
    changed = len(selected) == 2 and selected[0]["out"] == "bezier" and abs(selected[0]["outEase"][0]["influence"] - 42) < 1e-9
    check("auto-Bezier neighbours keep their shape outside the selection", drift < 1e-9 and changed, f"max drift {drift:.2e}")

r = session(key("transform/rotation", 0, 0), key("transform/rotation", 1, 40), key("transform/rotation", 2, 100),
            ("keys.selectAll", {}), ("keys.interpolation", {"interpolation": "continuousBezier"}), run_plugin("apply"), ("keys.info", {}),
            *samples("transform/rotation", 0, 1.001, n), *samples("transform/rotation", 1.001, 2.002, n))
if isinstance(r, dict):
    check("continuous keys get independent in and out eases", False, r["error"])
else:
    t = key_times(r[6], "transform/rotation")
    e1 = scalar_progress_error(values(r[7:8 + n]), 0, 40, sample_times(0, 1.001, n), t[0], t[1], DEFAULT)
    e2 = scalar_progress_error(values(r[8 + n:9 + 2 * n]), 40, 100, sample_times(1.001, 2.002, n), t[1], t[2], DEFAULT)
    check("continuous keys get independent in and out eases", max(e1, e2) < 1e-6, f"errors {e1:.2e}, {e2:.2e}")

r = session(key("transform/rotation", 0, 0), key("transform/rotation", 2, 90), select("transform/rotation", 0), run_plugin("apply"))
message = r.get("error", "") if isinstance(r, dict) else ""
check("one selected key reports what to select", "select two or more neighbouring keyframes" in message, message)

r = session(key("transform/rotation", 0, 0), key("transform/rotation", 1, 30), key("transform/rotation", 2, 90),
            select("transform/rotation", 0, 2.002), run_plugin("apply"))
message = r.get("error", "") if isinstance(r, dict) else ""
check("keys with an unselected key between them are refused", "not neighbours" in message, message)

r = session(run_plugin("easy-ease"))
message = r.get("error", "") if isinstance(r, dict) else ""
check("an empty selection reports what to select", "Select two or more neighbouring keyframes" in message, message)

print(f"{len(failures)} failed" if failures else "all checks passed")
sys.exit(1 if failures else 0)
PY
