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
cp target/wasm32-unknown-unknown/release/curves.wasm dist/plugin.wasm

CLI="$CLI" BUNDLE="$HERE/dist" OUT="$EFFECTCRAFT_CONFIG_DIR" python3 -I - <<'PY'
import json, os, struct, subprocess, sys, zlib

CLI, BUNDLE, OUT = os.environ["CLI"], os.environ["BUNDLE"], os.environ["OUT"]
EFFECT = "org.craftkit.curves.curves"
S_CURVE = [[0.0, 0.0], [0.3, 0.2], [0.7, 0.8], [1.0, 1.0]]
LINE = [[0.0, 0.0], [1.0, 1.0]]
failures = []


def check(name, ok, detail=""):
    print(("PASS " if ok else "FAIL ") + name + (f"  ({detail})" if detail else ""))
    if not ok:
        failures.append(name)


def centre(png):
    data = open(png, "rb").read()
    width, height, depth, kind = struct.unpack(">IIBB", data[16:26])
    chunks, at = b"", 8
    while at < len(data):
        length, tag = struct.unpack(">I4s", data[at:at + 8])
        if tag == b"IDAT":
            chunks += data[at + 8:at + 8 + length]
        at += 12 + length
    raw = zlib.decompress(chunks)
    channels = {2: 3, 6: 4}[kind]
    stride = width * channels
    rows, previous, offset = [], bytearray(stride), 0
    for _ in range(height):
        kind_of_filter, line = raw[offset], bytearray(raw[offset + 1:offset + 1 + stride])
        offset += 1 + stride
        for i in range(stride):
            left = line[i - channels] if i >= channels else 0
            up, corner = previous[i], previous[i - channels] if i >= channels else 0
            if kind_of_filter == 1:
                line[i] = (line[i] + left) & 255
            elif kind_of_filter == 2:
                line[i] = (line[i] + up) & 255
            elif kind_of_filter == 3:
                line[i] = (line[i] + (left + up) // 2) & 255
            elif kind_of_filter == 4:
                p = left + up - corner
                pa, pb, pc = abs(p - left), abs(p - up), abs(p - corner)
                line[i] = (line[i] + (left if pa <= pb and pa <= pc else up if pb <= pc else corner)) & 255
        rows.append(line)
        previous = line
    y, x = height // 2, width // 2
    return list(rows[y][x * channels:x * channels + 3])


def project(name, grey, curves, plugin=True):
    path = os.path.join(OUT, name + ".ecproj")
    value = json.dumps({"channels": curves})
    steps = ["comp.new", json.dumps({"name": "T", "width": 64, "height": 64, "duration": 1}),
             "layer.newSolid", json.dumps({"name": "S", "color": "#%02x%02x%02x" % (grey, grey, grey)}),
             "plugin.load", json.dumps({"path": BUNDLE}),
             "effect.apply", json.dumps({"effect": EFFECT}),
             "prop.set", json.dumps({"path": "effects/#1/curves", "value": value})]
    out = subprocess.run([CLI, "run", *steps, "--empty", "--save-as", path, "--json"], capture_output=True, text=True)
    if out.returncode != 0:
        raise SystemExit(out.stdout + out.stderr)
    return path


def render(path, plugin):
    png = path.replace(".ecproj", "-with.png" if plugin else "-without.png")
    env = dict(os.environ, EFFECTCRAFT_PLUGIN_PATH=BUNDLE if plugin else "")
    out = subprocess.run([CLI, "render-frame", "--project", path, "--time", "0", "--out", png], capture_output=True, text=True, env=env)
    if out.returncode != 0:
        raise SystemExit(out.stdout + out.stderr)
    return centre(png)


for grey, wanted in [(64, 39), (192, 217)]:
    path = project(f"s-{grey}", grey, [S_CURVE, LINE, LINE, LINE, LINE])
    found = render(path, True)
    check(f"the S curve maps grey {grey} to {wanted}", all(abs(c - wanted) <= 1 for c in found), f"rendered {found}")
    found = render(path, False)
    check(f"without the plugin grey {grey} renders unchanged", all(abs(c - grey) <= 1 for c in found), f"rendered {found}")

path = project("red-half", 200, [LINE, [[0.0, 0.0], [1.0, 0.5]], LINE, LINE, LINE])
found = render(path, True)
check("the red curve halves red only", abs(found[0] - 100) <= 1 and abs(found[1] - 200) <= 1 and abs(found[2] - 200) <= 1, f"rendered {found}")

print(f"{len(failures)} failed" if failures else "all checks passed")
sys.exit(1 if failures else 0)
PY
