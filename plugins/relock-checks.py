#!/usr/bin/env python3
import subprocess
import sys
import tomllib
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
KIT_CRATE = "craft-plugin"
DEPENDENCY_TABLES = ("dependencies", "dev-dependencies", "build-dependencies")


def toml(path):
    return tomllib.loads(path.read_text())


def cargo(*args):
    subprocess.run(["cargo", *map(str, args)], check=True, stdout=subprocess.DEVNULL)


def workspace_kit():
    url = toml(ROOT / "Cargo.toml")["workspace"]["dependencies"][KIT_CRATE]["git"]
    revs = {
        package["source"].split("#")[1]
        for package in toml(ROOT / "Cargo.lock")["package"]
        if package.get("source", "").startswith(f"git+{url}?") or package.get("source", "").startswith(f"git+{url}#")
    }
    if len(revs) != 1:
        sys.exit(f"Cargo.lock pins {len(revs)} revisions of {url}, expected one")
    return url, revs.pop()


def registry_versions():
    versions = {}
    for package in toml(ROOT / "Cargo.lock")["package"]:
        if package.get("source", "").startswith("registry+"):
            versions.setdefault(package["name"], set()).add(package["version"])
    return versions


def git_pins(manifest):
    data = toml(manifest)
    for table in DEPENDENCY_TABLES:
        for name, spec in data.get(table, {}).items():
            if isinstance(spec, dict) and name.startswith("craft-plugin") and "git" in spec:
                yield name, spec["git"], spec.get("rev") or spec.get("branch") or spec.get("tag") or "(no rev)"


def pin_mismatches(manifest, url, rev):
    return [
        f"{manifest.relative_to(ROOT)}: {name} pins {found_url} {found_rev}, the workspace pins {url} {rev}"
        for name, found_url, found_rev in git_pins(manifest)
        if (found_url, found_rev) != (url, rev)
    ]


def next_update(lock, url, rev, versions):
    for package in toml(lock)["package"]:
        source, name, have = package.get("source", ""), package["name"], package["version"]
        if source.startswith(f"git+{url}") and source.split("#")[1] != rev:
            return f"{source.split('#')[0]}#{name}@{have}", rev
        wanted = versions.get(name, set())
        if source.startswith("registry+") and have not in wanted and len(wanted) == 1:
            return f"{name}@{have}", next(iter(wanted))
    return None


def main():
    check_only = sys.argv[1:] == ["--check"]
    url, rev = workspace_kit()
    versions = registry_versions()
    for manifest in sorted(ROOT.glob("plugins/*/check/Cargo.toml")):
        if not check_only:
            cargo("metadata", "--format-version", "1", "--manifest-path", manifest)
            while update := next_update(manifest.with_name("Cargo.lock"), url, rev, versions):
                cargo("update", "--manifest-path", manifest, update[0], "--precise", update[1])
        try:
            cargo("metadata", "--locked", "--format-version", "1", "--manifest-path", manifest)
        except subprocess.CalledProcessError:
            sys.exit(f"{manifest.relative_to(ROOT)}: its Cargo.lock is stale; run plugins/relock-checks.py and commit the result")
        print(f"{manifest.relative_to(ROOT)}: locked at kit {rev[:9]}")
    manifests = sorted(ROOT.glob("plugins/*/Cargo.toml")) + sorted(ROOT.glob("plugins/*/check/Cargo.toml"))
    mismatches = [line for manifest in manifests for line in pin_mismatches(manifest, url, rev)]
    for line in mismatches:
        print(line)
    if mismatches:
        sys.exit(f"{len(mismatches)} plugin kit pins differ from the workspace kit {url} {rev[:9]}")
    print(f"{len(manifests)} plugin manifests pin the workspace kit {rev[:9]}")


if __name__ == "__main__":
    main()
