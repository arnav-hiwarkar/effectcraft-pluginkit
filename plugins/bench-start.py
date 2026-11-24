import json, os, shutil, sys

template, out, count, mode = sys.argv[1], sys.argv[2], int(sys.argv[3]), sys.argv[4]
shutil.rmtree(out, ignore_errors=True)
plugins = os.path.join(out, "Plugins")
os.makedirs(plugins)
base = json.load(open(os.path.join(template, "plugin.json")))
wasm_name = base["entry"]["wasm"]
wasm = open(os.path.join(template, wasm_name), "rb").read()


def leb(n):
    out = bytearray()
    while True:
        b = n & 0x7F
        n >>= 7
        out.append(b | (0x80 if n else 0))
        if not n:
            return bytes(out)


consents = {}
for i in range(count):
    pid = f"com.bench.p{i:03d}"
    dest = os.path.join(plugins, f"p{i:03d}")
    shutil.copytree(template, dest)
    json.dump(dict(base, id=pid, name=f"Bench {i:03d}"), open(os.path.join(dest, "plugin.json"), "w"))
    name = b"unique"
    payload = leb(len(name)) + name + f"{i:08d}".encode()
    open(os.path.join(dest, wasm_name), "wb").write(wasm + b"\x00" + leb(len(payload)) + payload)
    consents[pid] = {"enabled": True, "granted": sorted(base.get("permissions", [])), "unsandboxed": base["version"] if mode == "native" else None}
json.dump(consents, open(os.path.join(plugins, "state.json"), "w"))
