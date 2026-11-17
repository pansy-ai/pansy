#!/usr/bin/env bash
# Sets up a scratch directory (Bun 1.4.2, checked against its SHASUMS256.txt;
# magpie's host.js, pi.js and fake plugin from magpie's last commit) and runs
# the spike in it. Nothing is written outside the scratch directory.
#
#   spikes/plugin-host/run.sh [scratch]   (default: $TMPDIR/pansy-p0)
set -euo pipefail

here=$(cd "$(dirname "$0")" && pwd)
repo=$(cd "$here/../.." && pwd)
scratch=${1:-${TMPDIR:-/tmp}/pansy-p0}
magpie=5d23ae860 # magpie's last commit on this branch's history
bun_version=1.4.2

case "$(uname -s)-$(uname -m)" in
  Linux-x86_64) asset=bun-linux-x64 ;;
  Linux-aarch64) asset=bun-linux-aarch64 ;;
  Darwin-arm64) asset=bun-darwin-aarch64 ;;
  Darwin-x86_64) asset=bun-darwin-x64 ;;
  MINGW*-x86_64 | MSYS*-x86_64) asset=bun-windows-x64 ;;
  *) echo "no Bun for $(uname -s)-$(uname -m)" >&2; exit 1 ;;
esac

mkdir -p "$scratch/bun" "$scratch/host" "$scratch/plugins/fake" "$scratch/home"
cd "$scratch/bun"
if [ ! -x "$asset/bun" ] && [ ! -x "$asset/bun.exe" ]; then
  base=https://github.com/oven-sh/bun/releases/download/bun-v$bun_version
  curl -fsSL -o SHASUMS256.txt "$base/SHASUMS256.txt"
  curl -fsSL -o "$asset.zip" "$base/$asset.zip"
  if command -v sha256sum >/dev/null; then sum="sha256sum"; else sum="shasum -a 256"; fi
  grep " $asset.zip\$" SHASUMS256.txt | $sum -c -
  python3 -c "import zipfile,sys; zipfile.ZipFile(sys.argv[1]).extractall('.')" "$asset.zip"
  chmod +x "$asset"/bun* || true
fi
bun=$scratch/bun/$asset/bun
[ -x "$bun" ] || bun=$bun.exe
"$bun" --version

git -C "$repo" show "$magpie:internal/plugin/host.js" >"$scratch/host/host.js"
git -C "$repo" show "$magpie:internal/plugin/pi.js" >"$scratch/host/pi.js"
git -C "$repo" show "$magpie:internal/plugin/testdata/fake/index.js" >"$scratch/plugins/fake/index.js"

cargo run --quiet --manifest-path "$here/Cargo.toml" -- "$bun" "$scratch"
