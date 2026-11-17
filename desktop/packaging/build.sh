#!/usr/bin/env bash
# Builds pansy for this Linux machine: dist/pansy-<version>-linux-<arch>.tar.gz
#
#   pansy                 the window with the gateway beside it, and the command
#   nsh/                  the compiled TSX window
#   share/                the desktop entry and the icon
#   install.sh            puts the desktop entry in place for this user
#
# The window looks for updates in nsh.json's "updater" feed (GitHub Releases):
# nsh/assets/nsh-updater.json, written below as `nsh package` would, tells it
# where. To publish a version, put the archives of every platform built and
# the manifest they make in a release there:
#
#   nsh update-manifest ../dist/pansy-<version>-linux-*.tar.gz --out ../dist
set -euo pipefail
cd -- "$(dirname -- "${BASH_SOURCE[0]}")/.."
export PATH="$HOME/.cargo/bin:$PATH"

nsh_version=0.5.1
if ! nsh --version 2>/dev/null | grep -q "^nsh $nsh_version "; then
  echo "NSH $nsh_version is required: NSH_VERSION=v$nsh_version sh <(curl -fsSL https://nsh.dev/install.sh)" >&2
  exit 1
fi
[[ $(uname -s) == Linux ]] || { echo "build.sh: only Linux packages are built so far" >&2; exit 1; }
case $(uname -m) in
  x86_64 | amd64) target=linux-x86_64 ;;
  aarch64 | arm64) target=linux-aarch64 ;;
  *) echo "build.sh: unsupported architecture $(uname -m)" >&2; exit 1 ;;
esac

version=$(python3 -c 'import json; print(json.load(open("nsh.json"))["version"])')
export SOURCE_DATE_EPOCH=${SOURCE_DATE_EPOCH:-$(git log -1 --format=%ct 2>/dev/null || date +%s)}

cargo build --locked --release -p pansy-desktop

name=pansy-$version-$target
stage=target/package/$name
rm -rf -- "$stage"
mkdir -p "$stage/share/applications" "$stage/share/icons" ../dist
nsh build App.tsx --release --out "$stage/nsh"
# where the window looks for updates, and what it is now (nsh/updater)
python3 - "$stage/nsh/assets/nsh-updater.json" <<'PY'
import json, sys
app = json.load(open("nsh.json"))
if "updater" in app:
    config = {"id": app["id"], "version": app["version"], "build": app.get("build", 1), **app["updater"]}
    with open(sys.argv[1], "w") as out:
        json.dump(config, out)
PY
install -m 755 target/release/pansy "$stage/pansy"
install -m 644 packaging/dev.pansy.app.desktop "$stage/share/applications/dev.pansy.app.desktop"
install -m 644 packaging/pansy.png "$stage/share/icons/dev.pansy.app.png"
cat > "$stage/install.sh" <<'SH'
#!/bin/sh
# Adds pansy to this user's applications menu and puts `pansy` on PATH.
set -eu
root=$(cd -- "$(dirname -- "$0")" && pwd)
apps=${XDG_DATA_HOME:-$HOME/.local/share}/applications
mkdir -p "$apps" "$HOME/.local/bin"
sed "s|{root}|$root|g" "$root/share/applications/dev.pansy.app.desktop" > "$apps/dev.pansy.app.desktop"
ln -sf "$root/pansy" "$HOME/.local/bin/pansy"
echo "pansy is in your applications menu, and \`pansy\` on your PATH (~/.local/bin)."
SH
chmod 755 "$stage/install.sh"

archive=../dist/$name.tar.gz
tar --sort=name --owner=0 --group=0 --numeric-owner --mtime="@$SOURCE_DATE_EPOCH" \
  -C "$(dirname "$stage")" -cf - "$name" | gzip -n -9 > "$archive"
echo "$archive"
