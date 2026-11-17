#!/usr/bin/env bash
# Numbers the next release (docs/release.md):
#
#   bump.sh 0.2.0   a release of pansy: the version in nsh.json and both
#                   workspaces, and the next build
#   bump.sh         a release of the window alone: the next build
#
# The build grows with every release of either kind: pansy runs a package
# of its window only when its build is newer than the one it has. Prints
# the tag that releases it.
set -euo pipefail
cd -- "$(dirname -- "${BASH_SOURCE[0]}")/.."
export PATH="$HOME/.cargo/bin:$PATH"

version=${1:-}
if [[ -n $version && ! $version =~ ^[0-9]+\.[0-9]+\.[0-9]+$ ]]; then
  echo "usage: bump.sh [MAJOR.MINOR.PATCH]" >&2
  exit 1
fi
# nsh.json and the manifests keep their layout: only these lines change
python3 - "$version" <<'PY'
import re, sys
version = sys.argv[1]
def edit(path, pattern, value):
    text = open(path).read()
    text, n = re.subn(pattern, lambda m: m.group(1) + value(m.group(2)) + m.group(3), text, count=1, flags=re.M)
    assert n == 1, f"{path}: nothing matches {pattern}"
    open(path, "w").write(text)
edit("nsh.json", r'^(  "build": )(\d+)(,)$', lambda build: str(int(build) + 1))
if version:
    edit("nsh.json", r'^(  "version": ")([^"]+)(",)$', lambda _: version)
    for manifest in ("Cargo.toml", "../Cargo.toml"):
        edit(manifest, r'^(version = ")([^"]+)(")$', lambda _: version)
PY
if [[ -n $version ]]; then
  cargo update --workspace --quiet
  (cd .. && cargo update --workspace --quiet)
fi
python3 - "$version" <<'PY'
import json, sys
app = json.load(open("nsh.json"))
tag = f"pansy-v{app['version']}" + ("" if sys.argv[1] else f"-ui.{app['build']}")
print(f"{app['version']} (build {app['build']}); commit, then: git tag {tag} && git push origin {tag}")
PY
