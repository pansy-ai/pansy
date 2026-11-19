#!/usr/bin/env bash
# Makes the installers for this machine from what build.sh staged (run it
# first), into ../dist beside its archive:
#
#   macOS     pansy-<version>-macos-<arch>.dmg: Pansy.app and a link to
#             Applications to drag it to (the app signed as build.sh signed
#             it; the image too with PANSY_CODESIGN_IDENTITY)
#   Windows   pansy-<version>-windows-<arch>-setup.exe: Inno Setup
#             (pansy.iss), for this user in %LOCALAPPDATA%\Programs\Pansy,
#             where the window's updater can replace it
#   Linux     pansy-<version>-linux-<arch>.deb: /opt/pansy, `pansy` on PATH
#             and the desktop entry; pansy-<version>-linux-<arch>.AppImage,
#             one file to run. Both take GTK 4 from the system,
#             as the archive does. The updater cannot replace either (one is
#             the system's, one is read only): the window sends their users
#             to the download page.
#
# The updater itself keeps to the archives (publish.sh); these are for the
# first install.
#
#   PANSY_ISCC           Inno Setup 6's compiler (default: ISCC on PATH or
#                        where Inno Setup installs itself)
#   PANSY_APPIMAGETOOL   an appimagetool (default: 1.9.1, downloaded into
#                        target/tools and checked against its hash)
set -euo pipefail
cd -- "$(dirname -- "${BASH_SOURCE[0]}")/.."
fail() { echo "installers.sh: $*" >&2; exit 1; }

case $(uname -s) in
  Linux) os=linux ;;
  Darwin) os=macos ;;
  MINGW* | MSYS* | CYGWIN*) os=windows ;;
  *) fail "unsupported system $(uname -s)" ;;
esac
case $(uname -m) in
  x86_64 | amd64) arch=x86_64 ;;
  aarch64 | arm64) arch=aarch64 ;;
  *) fail "unsupported architecture $(uname -m)" ;;
esac
python=$(command -v python3 || command -v python)

version=$("$python" -c 'import json; print(json.load(open("nsh.json"))["version"])')
build=$("$python" -c 'import json; print(json.load(open("nsh.json")).get("build", 1))')
export SOURCE_DATE_EPOCH=${SOURCE_DATE_EPOCH:-$(git log -1 --format=%ct 2>/dev/null || date +%s)}

name=pansy-$version-$os-$arch
package=target/package/$name
case $os in
  macos) staged=$package/Pansy.app ;;
  *) staged=$package/pansy ;;
esac
[[ -d $staged ]] || fail "nothing staged at $staged: packaging/build.sh first"
mkdir -p ../dist
made=()

# a file of a pinned release, downloaded once into target/tools
tool() {
  local url=$1 sha=$2 file=target/tools/$3
  mkdir -p target/tools
  if ! echo "$sha  $file" | sha256sum -c --quiet - > /dev/null 2>&1; then
    curl -fsSL "$url" -o "$file.part"
    echo "$sha  $file.part" | sha256sum -c --quiet - || fail "$url is not the file pinned here"
    mv -- "$file.part" "$file"
  fi
  chmod 755 "$file"
  echo "$file"
}

case $os in
  macos)
    volume=$package/dmg
    rm -rf -- "$volume"
    mkdir -p "$volume"
    ditto "$staged" "$volume/Pansy.app"
    ln -s /Applications "$volume/Applications"
    image=../dist/$name.dmg
    rm -f -- "$image"
    # hdiutil is now and then busy on a machine just started: a few tries
    for try in 1 2 3 4 5; do
      hdiutil create -volname Pansy -srcfolder "$volume" -fs HFS+ -format UDZO \
        -imagekey zlib-level=9 -ov "$image" > /dev/null && break
      ((try < 5)) || fail "hdiutil could not make $image"
      sleep $((try * 5))
    done
    if [[ -n ${PANSY_CODESIGN_IDENTITY:-} ]]; then
      codesign --force --sign "$PANSY_CODESIGN_IDENTITY" "$image"
    fi
    made+=("$image")
    ;;

  windows)
    iscc=${PANSY_ISCC:-$(command -v iscc || command -v ISCC || true)}
    if [[ -z $iscc ]]; then
      for candidate in "/c/Program Files (x86)/Inno Setup 6/ISCC.exe" "/c/Program Files/Inno Setup 6/ISCC.exe" \
        "$(cygpath -u "${LOCALAPPDATA:-C:\\}")/Programs/Inno Setup 6/ISCC.exe"; do
        [[ -x $candidate ]] && iscc=$candidate && break
      done
    fi
    [[ -n $iscc ]] || fail "Inno Setup 6 is not installed (winget install JRSoftware.InnoSetup, or PANSY_ISCC)"
    # Windows' version numbers are four numbers: the version's own, and the build
    numbers=${version%%-*}.$build
    # ISCC's switches start with /, which Git Bash would take for paths; the
    # paths are whole, since ISCC reads others from the script's directory
    MSYS2_ARG_CONV_EXCL='*' "$iscc" /Q \
      "/DVersion=$version" "/DNumbers=$numbers" \
      "/DSource=$(cygpath -aw "$staged")" \
      "/DOutput=$(cygpath -aw ../dist)" "/DOutputName=$name-setup" \
      "$(cygpath -aw packaging/pansy.iss)"
    made+=("../dist/$name-setup.exe")
    ;;

  linux)
    # the Debian package: the archive's directory in /opt/pansy
    case $arch in
      x86_64) deb_arch=amd64 ;;
      aarch64) deb_arch=arm64 ;;
    esac
    root=$package/deb
    rm -rf -- "$root"
    mkdir -p "$root/DEBIAN" "$root/opt" "$root/usr/bin" "$root/usr/share/applications"
    cp -a -- "$staged" "$root/opt/pansy"
    rm -f -- "$root/opt/pansy/install.sh"
    sed 's|{root}|/opt/pansy|g' packaging/dev.pansy.app.desktop > "$root/usr/share/applications/dev.pansy.app.desktop"
    ln -s /opt/pansy/pansy "$root/usr/bin/pansy"
    chmod -R u=rwX,go=rX "$root"
    # the libraries it links, as Debian names their packages, where dpkg-dev is
    depends="libgtk-4-1"
    if command -v dpkg-shlibdeps > /dev/null; then
      shlibs=$package/shlibdeps
      rm -rf -- "$shlibs"
      mkdir -p "$shlibs/debian"
      printf 'Source: pansy\n\nPackage: pansy\nArchitecture: any\n' > "$shlibs/debian/control"
      program=$(pwd)/$root/opt/pansy/pansy
      found=$(cd "$shlibs" && dpkg-shlibdeps -O "$program" 2> /dev/null | sed -n 's/^shlibs:Depends=//p') || true
      [[ -n $found ]] && depends=$found
    fi
    # 0.3.0-beta.1 is ~beta.1 to dpkg, so that it comes before 0.3.0
    cat > "$root/DEBIAN/control" <<CONTROL
Package: pansy
Version: ${version/-/\~}
Architecture: $deb_arch
Maintainer: pansy <https://github.com/pansy-ai/pansy>
Installed-Size: $(du -sk "$root/opt" "$root/usr" | awk '{ s += $1 } END { print s }')
Depends: $depends
Section: devel
Priority: optional
Homepage: https://github.com/pansy-ai/pansy
Description: Every agent's model, one place
 A local gateway that lets every coding agent use every provider's models,
 with a window to manage it and the pansy command.
CONTROL
    deb=../dist/$name.deb
    dpkg-deb --root-owner-group -Zxz --build "$root" "$deb" > /dev/null
    made+=("$deb")

    # the AppImage: the archive's directory, started by AppRun
    appdir=$package/AppDir
    rm -rf -- "$appdir"
    mkdir -p "$appdir"
    cp -a -- "$staged" "$appdir/pansy"
    rm -f -- "$appdir/pansy/install.sh"
    cat > "$appdir/AppRun" <<'SH'
#!/bin/sh
here=$(dirname -- "$(readlink -f -- "$0")")
exec "$here/pansy/pansy" "$@"
SH
    chmod 755 "$appdir/AppRun"
    sed -e 's|^Exec=.*|Exec=pansy|' -e 's|^Icon=.*|Icon=dev.pansy.app|' \
      packaging/dev.pansy.app.desktop > "$appdir/dev.pansy.app.desktop"
    cp packaging/pansy.png "$appdir/dev.pansy.app.png"
    ln -s dev.pansy.app.png "$appdir/.DirIcon"
    case $arch in
      x86_64)
        appimagetool_sha=ed4ce84f0d9caff66f50bcca6ff6f35aae54ce8135408b3fa33abfc3cb384eb0
        runtime_sha=2fca8b443c92510f1483a883f60061ad09b46b978b2631c807cd873a47ec260d
        ;;
      aarch64)
        appimagetool_sha=f0837e7448a0c1e4e650a93bb3e85802546e60654ef287576f46c71c126a9158
        runtime_sha=00cbdfcf917cc6c0ff6d3347d59e0ca1f7f45a6df1a428a0d6d8a78664d87444
        ;;
    esac
    appimagetool=${PANSY_APPIMAGETOOL:-$(tool \
      "https://github.com/AppImage/appimagetool/releases/download/1.9.1/appimagetool-$arch.AppImage" \
      "$appimagetool_sha" "appimagetool-1.9.1-$arch.AppImage")}
    runtime=$(tool "https://github.com/AppImage/type2-runtime/releases/download/20251108/runtime-$arch" \
      "$runtime_sha" "runtime-20251108-$arch")
    appimage=../dist/$name.AppImage
    rm -f -- "$appimage"
    # extracted rather than mounted: CI machines have no FUSE
    ARCH=$arch APPIMAGE_EXTRACT_AND_RUN=1 "$appimagetool" --no-appstream \
      --runtime-file "$runtime" "$appdir" "$appimage" > /dev/null
    made+=("$appimage")
    ;;
esac
printf '%s\n' "${made[@]}"
