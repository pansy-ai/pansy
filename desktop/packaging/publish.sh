#!/usr/bin/env bash
# Publishes a release to the Cloudflare R2 bucket the window updates from
# (nsh.json's "updater" `url`; docs/release.md). The bucket holds
#
#   latest.json, latest.json.sig     the feed: the newest pansy for each
#                                    system and the newest build of its
#                                    window, signed by the release key
#   v<version>/pansy-<version>-<os>-<arch>.tar.gz|.zip   build.sh's archives
#   v<version>/dev.pansy.app-<build>.nshpkg              the window alone
#   index.html                       the download page (site/)
#
#   publish.sh       a release of pansy: ../dist's archives of nsh.json's
#                    version, from build.sh on each system
#   publish.sh ui    a release of the window alone, for the pansy that is
#                    out: nsh.json's version is that one's, its build newer.
#                    Only the TSX may have changed: the package runs on that
#                    release's Rust.
#
# The archives go up first and the feed last: what the feed names is there.
#
#   PANSY_RELEASE_KEY_FILE   the release key (default keys/release.secret)
#   PANSY_RELEASE_BUCKET     the bucket (default pansy-releases)
#   PANSY_RELEASE_NOTES      a text file with what changed
#   PANSY_PUBLISH_DRY=1      makes ../dist/release as the bucket would be,
#                            and uploads nothing
#   PANSY_PUBLISH_FORCE=1    a window-only release although Rust changed
#   CLOUDFLARE_API_TOKEN, CLOUDFLARE_ACCOUNT_ID   wrangler's, or `wrangler login`
set -euo pipefail
cd -- "$(dirname -- "${BASH_SOURCE[0]}")/.."
export PATH="$HOME/.cargo/bin:$HOME/.nsh/bin:$PATH"

kind=${1:-app}
[[ $kind == app || $kind == ui ]] || { echo "usage: publish.sh [ui]" >&2; exit 1; }
fail() { echo "publish.sh: $*" >&2; exit 1; }
field() { python3 -c 'import json, sys; v = json.load(open(sys.argv[1]))
for key in sys.argv[2].split("."): v = v.get(key) if isinstance(v, dict) else None
print("" if v is None else v)' "$1" "$2"; }

id=$(field nsh.json id)
version=$(field nsh.json version)
build=$(field nsh.json build)
feed=$(field nsh.json updater.url)
[[ $feed == https://*/latest.json ]] || fail "nsh.json's updater.url must be https://…/latest.json"
base=${feed%/latest.json}
key=${PANSY_RELEASE_KEY_FILE:-keys/release.secret}
[[ -f $key ]] || fail "the release key is not at $key (packaging/keys.sh, or PANSY_RELEASE_KEY_FILE)"
bucket=${PANSY_RELEASE_BUCKET:-pansy-releases}
read -r -a wrangler <<< "${WRANGLER:-npx --yes wrangler@4}"

out=../dist/release
rm -rf -- "$out"
mkdir -p "$out/v$version"
scratch=$(mktemp -d)
trap 'rm -rf -- "$scratch"' EXIT

# what is out now; nothing before the first release
live=$scratch/live.json
curl -fsSL "$feed" -o "$live" 2> /dev/null || rm -f -- "$live"
live_version=
live_build=0
if [[ -f $live ]]; then
  live_version=$(field "$live" version)
  live_build=$(field "$live" package.build)
fi
older() { python3 -c 'import sys
n = lambda v: [int(x) for x in v.split("-")[0].split(".")]
sys.exit(0 if n(sys.argv[1]) < n(sys.argv[2]) else 1)' "$1" "$2"; }

manifest=(--entry . --key "$key" --base-url "$base/v$version/" --out "$out")
[[ -z ${PANSY_RELEASE_NOTES:-} ]] || manifest+=(--notes "$PANSY_RELEASE_NOTES")
uploads=()

if [[ $kind == app ]]; then
  [[ -z $live_version ]] || older "$live_version" "$version" \
    || fail "$live_version is out: a release needs a newer version than $version (packaging/bump.sh)"
  shopt -s nullglob
  archives=(../dist/pansy-"$version"-*.tar.gz ../dist/pansy-"$version"-*.zip)
  shopt -u nullglob
  ((${#archives[@]})) || fail "no archives of $version in ../dist (packaging/build.sh, on each system)"
  cp -- "${archives[@]}" "$out/v$version/"
  nsh update-manifest "$out/v$version"/pansy-* "${manifest[@]}"
  for archive in "$out/v$version"/pansy-*; do uploads+=("v$version/$(basename "$archive")"); done
else
  [[ $live_version == "$version" ]] \
    || fail "the pansy that is out is ${live_version:-none}, nsh.json says $version: the window alone is released for the one that is out"
  ((build > ${live_build:-0})) || fail "build $build is not newer than the window's that is out ($live_build): packaging/bump.sh"
  tag=pansy-v$version
  if git rev-parse -q --verify "refs/tags/$tag" > /dev/null; then
    shipped=$(git show "$tag:desktop/nsh.json" | python3 -c 'import json, sys; print(json.load(sys.stdin).get("build", 1))')
    ((build > shipped)) || fail "build $build is not newer than $version's own ($shipped): packaging/bump.sh"
    # the package runs on that release's program: its Rust must be the same
    if ! git diff --quiet "$tag" -- native runner Cargo.toml Cargo.lock ../crates ../Cargo.toml ../Cargo.lock; then
      [[ -n ${PANSY_PUBLISH_FORCE:-} ]] \
        || fail "Rust changed since $tag, and the window's package runs on that release's: release pansy itself (or PANSY_PUBLISH_FORCE=1 if the window calls nothing new)"
    fi
  else
    echo "publish.sh: no tag $tag to compare with: not checked that Rust is as released" >&2
  fi
  # the archives that are out stay in the feed: fetched to be named again
  python3 - "$live" "$base/v$version/" > "$scratch/archives" <<'PY'
import json, sys
from urllib.parse import urljoin
for asset in json.load(open(sys.argv[1]))["platforms"].values():
    if asset.get("sha256"):
        print(asset["sha256"], urljoin(sys.argv[2], asset["url"]))
PY
  mkdir "$scratch/out"
  while read -r sha url; do
    file=$scratch/out/$(basename "$url")
    curl -fsSL "$url" -o "$file"
    echo "$sha  $file" | sha256sum -c --quiet - || fail "$url is not what the feed says"
  done < "$scratch/archives"
  package=$out/v$version/$id-$build.nshpkg
  nsh publish . --key "$key" --cert packaging/release.cert --out "$package"
  shopt -s nullglob
  archives=("$scratch"/out/*)
  shopt -u nullglob
  nsh update-manifest "${archives[@]}" --package "$package" "${manifest[@]}"
  uploads+=("v$version/$(basename "$package")")
fi
[[ -f $out/latest.json.sig ]] || fail "the feed was not signed: nsh.json's updater needs publicKey (packaging/keys.sh)"
cp packaging/site/index.html "$out/index.html"
uploads+=(index.html latest.json.sig latest.json)

if [[ -n ${PANSY_PUBLISH_DRY:-} ]]; then
  echo "publish.sh: dry run; the bucket would get, from $out:"
  printf '  %s\n' "${uploads[@]}"
  exit
fi
for object in "${uploads[@]}"; do
  case $object in
    *.json) type=application/json cache="public, max-age=60" ;;
    *.sig) type=text/plain cache="public, max-age=60" ;;
    *.html) type="text/html; charset=utf-8" cache="public, max-age=300" ;;
    *.zip) type=application/zip cache="public, max-age=31536000, immutable" ;;
    *.tar.gz) type=application/gzip cache="public, max-age=31536000, immutable" ;;
    *) type=application/octet-stream cache="public, max-age=31536000, immutable" ;;
  esac
  "${wrangler[@]}" r2 object put "$bucket/$object" --file "$out/$object" --remote \
    --content-type "$type" --cache-control "$cache"
done
echo "published $version (build $build) to $base"
