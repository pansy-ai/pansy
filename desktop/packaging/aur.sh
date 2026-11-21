#!/usr/bin/env bash
# Puts the release that is out on the AUR as pansy-bin, so Arch Linux and
# Omarchy install it with `yay -S pansy-bin` (docs/release.md). The package
# (packaging/aur/PKGBUILD) is the release's Linux archives in /opt/pansy, as
# the .deb has them; this writes its version and the archives' checksums,
# and its .SRCINFO, into ../dist/aur, the AUR's repository of it.
#
#   aur.sh       nsh.json's version: the checksums of ../dist's Linux
#                archives, or the feed's when ../dist has none
#
# A beta is not put there: the AUR has the pansy everyone has. The same
# version again with a PKGBUILD changed is the next pkgrel.
#
#   PANSY_AUR_KEY   a private SSH key the AUR account of pansy-bin has: the
#                   repository is cloned from the AUR, and what changed is
#                   pushed. Without it, ../dist/aur is written and nothing
#                   is pushed.
set -euo pipefail
cd -- "$(dirname -- "${BASH_SOURCE[0]}")/.."

fail() { echo "aur.sh: $*" >&2; exit 1; }
field() { python3 -c 'import json, sys; v = json.load(open(sys.argv[1]))
for key in sys.argv[2].split("."): v = v.get(key) if isinstance(v, dict) else None
print("" if v is None else v)' "$1" "$2"; }

package=pansy-bin
# the AUR's host key, as https://aur.archlinux.org says it
host_key=SHA256:RFzBCUItH9LZS0cKB5UE6ceAYhBD5C8GeOBip8Z11+4

version=$(field nsh.json version)
feed=$(field nsh.json updater.url)
[[ $version == *-* ]] && fail "$version is a beta: the AUR has the pansy everyone has"

# the checksum of each Linux archive: built here, or as the feed has it
sums=()
for arch in x86_64 aarch64; do
  archive=../dist/pansy-$version-linux-$arch.tar.gz
  if [[ -f $archive ]]; then
    sum=$(sha256sum -- "$archive" | cut -d' ' -f1)
  else
    said=$(curl -fsSL --retry 3 -- "$feed") || fail "../dist has no $archive, and $feed did not answer"
    sum=$(python3 -c 'import json, sys
feed = json.loads(sys.argv[1])
print(feed["platforms"][sys.argv[3]]["sha256"] if feed.get("version") == sys.argv[2] else "")' "$said" "$version" "linux-$arch")
    [[ -n $sum ]] || fail "../dist has no $archive, and the feed is not of $version"
  fi
  sums+=("$sum")
done

out=../dist/aur
rm -rf -- "$out"
mkdir -p ../dist
if [[ -n ${PANSY_AUR_KEY:-} ]]; then
  [[ -f $PANSY_AUR_KEY ]] || fail "PANSY_AUR_KEY is not a file: $PANSY_AUR_KEY"
  ssh_dir=$(mktemp -d)
  trap 'rm -rf -- "$ssh_dir"' EXIT
  ssh-keyscan -t ed25519 aur.archlinux.org 2> /dev/null > "$ssh_dir/known_hosts"
  [[ $(ssh-keygen -lf "$ssh_dir/known_hosts" | cut -d' ' -f2) == "$host_key" ]] ||
    fail "aur.archlinux.org's host key is not $host_key"
  export GIT_SSH_COMMAND="ssh -i '$PANSY_AUR_KEY' -o IdentitiesOnly=yes -o UserKnownHostsFile='$ssh_dir/known_hosts' -o StrictHostKeyChecking=yes"
  # a package not on the AUR yet clones empty, and the first push makes it
  git clone --quiet "ssh://aur@aur.archlinux.org/$package.git" "$out"
else
  mkdir -p -- "$out"
fi

# what the AUR has, if it is of this version: its pkgrel, the next if the PKGBUILD changed
pkgrel=1
before=
if [[ -f $out/PKGBUILD ]]; then
  before=$(cat -- "$out/PKGBUILD")
  if [[ $(sed -n 's/^pkgver=//p' "$out/PKGBUILD") == "$version" ]]; then
    pkgrel=$(sed -n 's/^pkgrel=//p' "$out/PKGBUILD")
  fi
fi
license_sum=$(sha256sum ../LICENSE | cut -d' ' -f1)
write() {
  sed -e "s/^pkgver=.*/pkgver=$version/" -e "s/^pkgrel=.*/pkgrel=$1/" \
    -e "s/^sha256sums=.*/sha256sums=('$license_sum')/" \
    -e "s/^sha256sums_x86_64=.*/sha256sums_x86_64=('${sums[0]}')/" \
    -e "s/^sha256sums_aarch64=.*/sha256sums_aarch64=('${sums[1]}')/" \
    packaging/aur/PKGBUILD > "$out/PKGBUILD"
}
write "$pkgrel"
if [[ -n $before && $(cat -- "$out/PKGBUILD") != "$before" && $(sed -n 's/^pkgver=//p' <<< "$before") == "$version" ]]; then
  pkgrel=$((pkgrel + 1))
  write "$pkgrel"
fi
cp -- ../LICENSE "$out/LICENSE"

# .SRCINFO, as `makepkg --printsrcinfo` writes it (the AUR reads it; a
# runner without makepkg writes it the same way)
(
  cd -- "$out"
  # shellcheck source=/dev/null
  source ./PKGBUILD
  {
    echo "pkgbase = $pkgname"
    printf '\t%s = %s\n' pkgdesc "$pkgdesc" pkgver "$pkgver" pkgrel "$pkgrel" url "$url"
    for a in "${arch[@]}"; do printf '\tarch = %s\n' "$a"; done
    for l in "${license[@]}"; do printf '\tlicense = %s\n' "$l"; done
    for d in "${depends[@]}"; do printf '\tdepends = %s\n' "$d"; done
    for p in "${provides[@]}"; do printf '\tprovides = %s\n' "$p"; done
    for c in "${conflicts[@]}"; do printf '\tconflicts = %s\n' "$c"; done
    for o in "${options[@]}"; do printf '\toptions = %s\n' "$o"; done
    for s in "${source[@]}"; do printf '\tsource = %s\n' "$s"; done
    for s in "${sha256sums[@]}"; do printf '\tsha256sums = %s\n' "$s"; done
    for s in "${source_x86_64[@]}"; do printf '\tsource_x86_64 = %s\n' "$s"; done
    for s in "${sha256sums_x86_64[@]}"; do printf '\tsha256sums_x86_64 = %s\n' "$s"; done
    for s in "${source_aarch64[@]}"; do printf '\tsource_aarch64 = %s\n' "$s"; done
    for s in "${sha256sums_aarch64[@]}"; do printf '\tsha256sums_aarch64 = %s\n' "$s"; done
    printf '\npkgname = %s\n' "$pkgname"
  } > .SRCINFO
)

if [[ -z ${PANSY_AUR_KEY:-} ]]; then
  echo "$package $version-$pkgrel is in $out; PANSY_AUR_KEY pushes it to the AUR"
  exit 0
fi
cd -- "$out"
git add PKGBUILD .SRCINFO LICENSE
if git diff --cached --quiet; then
  echo "the AUR has $package $version-$pkgrel already"
  exit 0
fi
git -c user.name=pansy -c user.email=noreply@pansyai.dev commit --quiet -m "pansy $version-$pkgrel"
git push --quiet origin HEAD:master
echo "$package $version-$pkgrel is on the AUR"
