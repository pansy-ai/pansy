#!/usr/bin/env bash
# Publishes a release to the Cloudflare R2 bucket the window updates from
# (nsh.json's "updater" `url`; docs/release.md). The bucket holds
#
#   latest.json, latest.json.sig     the feed: the newest pansy for each
#                                    system and the newest build of its
#                                    window, signed by the update key
#   v<version>/pansy-<version>-<os>-<arch>.tar.gz|.zip   build.sh's archives
#   v<version>/pansy-<version>-<os>-<arch>.dmg|-setup.exe|.deb|.AppImage
#                                    installers.sh's installers, which the
#                                    feed does not name
#   v<version>/dev.pansy.app-<build>.nshpkg              the window alone
#   v<version>/notes.<language>.md   what changed, in the window's other languages
#   beta/latest.json, …              the same feed for those who chose beta
#   index.html, downloads.json       the download page (site/) and the
#                                    installers it offers
#
#   publish.sh       a release of pansy: ../dist's archives of nsh.json's
#                    version, from build.sh on each system, and the
#                    installers there are (installers.sh)
#                    A version with a suffix (0.2.0-beta.1) goes to the beta
#                    feed alone, any other to both.
#   publish.sh ui    a release of the window alone, for the pansy that is
#                    out: nsh.json's version is that one's, its build newer.
#                    Only the TSX may have changed: the package runs on that
#                    release's Rust.
#
# The archives go up first and the feed last: what the feed names is there.
#
#   PANSY_KEYS               where release.secret (signs the window's
#                            packages) and update.secret (signs the feed)
#                            are (default keys/)
#   PANSY_RELEASE_BUCKET     the bucket (default pansy-releases)
#   PANSY_RELEASE_NOTES      a text file with what changed, which the window
#                            shows as What's new (default packaging/notes.md,
#                            when there is one: headings, bullets, lines);
#                            notes.zh-Hans.md beside it is the same in Chinese
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
# over HTTPS; this computer's own address is for trying a release (docs/release.md)
[[ $feed == https://*/latest.json || $feed == http://127.0.0.1:*/latest.json ]] || fail "nsh.json's updater.url must be https://…/latest.json"
base=${feed%/latest.json}
keys=${PANSY_KEYS:-keys}
[[ -f $keys/update.secret ]] || fail "the update key is not at $keys/update.secret (packaging/keys.sh, or PANSY_KEYS)"
bucket=${PANSY_RELEASE_BUCKET:-pansy-releases}
read -r -a wrangler <<< "${WRANGLER:-npx --yes wrangler@4}"

out=../dist/release
rm -rf -- "$out"
mkdir -p "$out/v$version"
scratch=$(mktemp -d)
trap 'rm -rf -- "$scratch"' EXIT

# The feeds this release is written to: "" is the one everyone reads, beta
# the one nsh.json's channel names. A version with a suffix (0.2.0-beta.1)
# is for beta alone; any other goes to both, so those on beta get it too.
beta_feed=$(field nsh.json updater.channels.beta)
[[ -z $beta_feed || $beta_feed == "$base/beta/latest.json" ]] || fail "nsh.json's updater.channels.beta must be $base/beta/latest.json"
case $kind-$version in
  ui-*-*) fail "$version is a beta: the window alone is released for the pansy everyone has" ;;
  ui-*) feeds=("") ;;
  app-*-*)
    [[ -n $beta_feed ]] || fail "$version is a beta, and nsh.json's updater has no beta channel"
    feeds=(beta)
    ;;
  app-*)
    feeds=("")
    [[ -z $beta_feed ]] || feeds+=(beta)
    ;;
esac

# what a feed has out now; nothing before its first release
live() {
  curl -fsSL "$base/${1:+$1/}latest.json" -o "$scratch/live.json" 2> /dev/null || rm -f -- "$scratch/live.json"
  live_version=
  live_build=0
  if [[ -f $scratch/live.json ]]; then
    live_version=$(field "$scratch/live.json" version)
    live_build=$(field "$scratch/live.json" package.build)
  fi
}
# semver's order: 0.2.0-beta.1 < 0.2.0-beta.2 < 0.2.0
older() { python3 -c 'import sys
def order(v):
    core, _, pre = v.partition("-")
    return [int(x) for x in core.split(".")], not pre, [(not x.isdigit(), int(x) if x.isdigit() else 0, x) for x in pre.split(".") if x]
sys.exit(0 if order(sys.argv[1]) < order(sys.argv[2]) else 1)' "$1" "$2"; }

# what changed: packaging/notes.md, and notes.<language>.md for each other
# language of the window (notes.zh-Hans.md), which a pansy in it is shown
manifest=(--entry . --key "$keys/update.secret" --base-url "$base/v$version/")
notes=${PANSY_RELEASE_NOTES:-packaging/notes.md}
if [[ -s $notes ]]; then
  manifest+=(--notes "$notes")
  shopt -s nullglob
  for file in "${notes%.md}".*.md; do
    tag=${file#"${notes%.md}".}
    manifest+=(--notes "${tag%.md}=$file")
  done
  shopt -u nullglob
fi
uploads=()
# a feed's own files, the manifest last: what it names is there. The notes
# in other languages are written beside it and named beside the archives.
feed_files() {
  local dir=${1:+$1/} file object
  [[ -f $out/${dir}latest.json.sig ]] || fail "the feed was not signed: nsh.json's updater needs publicKey (packaging/keys.sh)"
  for file in "$out/$dir"notes.*.md; do
    [[ -f $file ]] || continue
    object=v$version/$(basename "$file")
    mv -- "$file" "$out/$object"
    [[ " ${uploads[*]} " == *" $object "* ]] || uploads+=("$object")
  done
  uploads+=("${dir}latest.json.sig" "${dir}latest.json")
}

if [[ $kind == app ]]; then
  shopt -s nullglob
  archives=(../dist/pansy-"$version"-*.tar.gz ../dist/pansy-"$version"-*.zip)
  shopt -u nullglob
  ((${#archives[@]})) || fail "no archives of $version in ../dist (packaging/build.sh, on each system)"
  # installers.sh's, for a first install: beside the archives, not in the feed
  shopt -s nullglob
  installers=(../dist/pansy-"$version"-*.dmg ../dist/pansy-"$version"-*-setup.exe
    ../dist/pansy-"$version"-*.deb ../dist/pansy-"$version"-*.AppImage)
  shopt -u nullglob
  cp -- "${archives[@]}" "${installers[@]}" "$out/v$version/"
  for file in "${archives[@]}" "${installers[@]}"; do uploads+=("v$version/$(basename "$file")"); done
  shipped=()
  for archive in "${archives[@]}"; do shipped+=("$out/v$version/$(basename "$archive")"); done
  # the download page offers what everyone gets: the installers it reads
  # from downloads.json, the archives from the feed
  if [[ ${feeds[0]} != beta ]]; then
    cp packaging/site/index.html "$out/index.html"
    python3 - "$version" "$base/v$version/" "${installers[@]}" > "$out/downloads.json" <<'PY'
import hashlib, json, os, re, sys
version, base, files = sys.argv[1], sys.argv[2], sys.argv[3:]
installers = []
for path in files:
    name = os.path.basename(path)
    system, kind = re.fullmatch(rf"pansy-{re.escape(version)}-(\w+-\w+?)(?:-setup)?\.(dmg|exe|deb|AppImage)", name).groups()
    with open(path, "rb") as f:
        sha = hashlib.file_digest(f, "sha256").hexdigest()
    installers.append({"system": system, "kind": kind, "url": base + name, "size": os.path.getsize(path), "sha256": sha})
json.dump({"version": version, "installers": installers}, sys.stdout, indent=2)
PY
    uploads+=(downloads.json index.html)
  fi
  for feed in "${feeds[@]}"; do
    live "$feed"
    if [[ -n $live_version ]] && ! older "$live_version" "$version"; then
      # a beta ahead of this release stays what beta offers
      [[ $feed == beta && ${feeds[0]} != beta ]] && continue
      fail "$live_version is out${feed:+ on $feed}: a release needs a newer version than $version (packaging/bump.sh)"
    fi
    nsh update-manifest "${shipped[@]}" "${manifest[@]}" --out "$out/$feed"
    feed_files "$feed"
  done
else
  live ""
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
  python3 - "$scratch/live.json" "$base/v$version/" > "$scratch/archives" <<'PY'
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
  [[ -f $keys/release.secret ]] || fail "the release key is not at $keys/release.secret (packaging/keys.sh, or PANSY_KEYS)"
  package=$out/v$version/$id-$build.nshpkg
  nsh publish . --key "$keys/release.secret" --cert packaging/release.cert --out "$package"
  # as a pansy installed will check it: a root nsh.json trusts certifies its key
  roots=()
  while read -r root; do
    echo "$root" > "$scratch/root-${#roots[@]}.pub"
    roots+=("$scratch/root-${#roots[@]}.pub")
  done < <(python3 -c 'import json; print("\n".join(json.load(open("nsh.json"))["updater"]["packages"]["roots"]))')
  nsh verify "$package" "${roots[@]/#/--root=}"
  shopt -s nullglob
  archives=("$scratch"/out/*)
  shopt -u nullglob
  nsh update-manifest "${archives[@]}" --package "$package" "${manifest[@]}" --out "$out"
  uploads+=("v$version/$(basename "$package")")
  cp packaging/site/index.html "$out/index.html"
  uploads+=(index.html)
  feed_files ""
fi

if [[ -n ${PANSY_PUBLISH_DRY:-} ]]; then
  echo "publish.sh: dry run; the bucket would get, from $out:"
  printf '  %s\n' "${uploads[@]}"
  exit
fi
for object in "${uploads[@]}"; do
  case $object in
    *.json) type=application/json cache="public, max-age=60" ;;
    *.sig) type=text/plain cache="public, max-age=60" ;;
    *.md) type="text/markdown; charset=utf-8" cache="public, max-age=60" ;;
    *.html) type="text/html; charset=utf-8" cache="public, max-age=300" ;;
    *.zip) type=application/zip cache="public, max-age=31536000, immutable" ;;
    *.tar.gz) type=application/gzip cache="public, max-age=31536000, immutable" ;;
    *.dmg) type=application/x-apple-diskimage cache="public, max-age=31536000, immutable" ;;
    *.exe) type=application/vnd.microsoft.portable-executable cache="public, max-age=31536000, immutable" ;;
    *.deb) type=application/vnd.debian.binary-package cache="public, max-age=31536000, immutable" ;;
    *) type=application/octet-stream cache="public, max-age=31536000, immutable" ;;
  esac
  "${wrangler[@]}" r2 object put "$bucket/$object" --file "$out/$object" --remote \
    --content-type "$type" --cache-control "$cache"
done
echo "published $version (build $build) to $base"
