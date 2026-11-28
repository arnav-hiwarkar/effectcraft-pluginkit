#!/usr/bin/env python3
import subprocess
import tomllib
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
KIT = "git+https://github.com/arnav-hiwarkar/craft-extensions-plugin-beta?branch=kit"


def packages(lock):
    return tomllib.loads(lock.read_text())["package"]


def cargo(*args):
    subprocess.run(["cargo", *map(str, args)], check=True, stdout=subprocess.DEVNULL)


def workspace_pins():
    kit_revs, versions = set(), {}
    for package in packages(ROOT / "Cargo.lock"):
        source = package.get("source", "")
        if source.startswith(KIT + "#"):
            kit_revs.add(source.split("#")[1])
        if source.startswith("registry+"):
            versions.setdefault(package["name"], set()).add(package["version"])
    assert len(kit_revs) == 1, f"Cargo.lock pins {len(kit_revs)} kit revisions, expected one"
    return kit_revs.pop(), versions


def next_update(lock, kit_rev, versions):
    for package in packages(lock):
        source, name, have = package.get("source", ""), package["name"], package["version"]
        if source.startswith(KIT + "#") and source.split("#")[1] != kit_rev:
            return f"{KIT}#{name}@{have}", kit_rev
        wanted = versions.get(name, set())
        if source.startswith("registry+") and have not in wanted and len(wanted) == 1:
            return f"{name}@{have}", next(iter(wanted))
    return None


def main():
    kit_rev, versions = workspace_pins()
    for manifest in sorted(ROOT.glob("plugins/*/check/Cargo.toml")):
        cargo("metadata", "--format-version", "1", "--manifest-path", manifest)
        while update := next_update(manifest.with_name("Cargo.lock"), kit_rev, versions):
            cargo("update", "--manifest-path", manifest, update[0], "--precise", update[1])
        cargo("metadata", "--locked", "--format-version", "1", "--manifest-path", manifest)
        print(f"{manifest.relative_to(ROOT)}: locked at kit {kit_rev[:9]}")


main()
