#!/usr/bin/env bash
# Builds pansy for this machine, into ../dist:
#
#   Linux     pansy-<version>-linux-<arch>.tar.gz, a directory, pansy/ (the
#             updater replaces what is in it, so it names no version):
#               pansy        the window with the gateway beside it, and the command
#               nsh/         the compiled TSX window
#               mobile-web/  the phone's page, which the remote interface serves
#               share/       the desktop entry and the icon
#               install.sh   puts the desktop entry in place for this user
#   macOS     pansy-<version>-macos-<arch>.tar.gz, Pansy.app (signed ad hoc,
#             or by PANSY_CODESIGN_IDENTITY): Contents/MacOS/pansy,
#             Contents/Resources/nsh and Contents/Resources/mobile-web
#   Windows   pansy-<version>-windows-<arch>.zip, a directory, pansy\:
#             pansy.exe, nsh\ and mobile-web\ (run from Git Bash)
#
# Each is laid out as `nsh package` lays an app out, which is what the
# window's updater replaces (nsh/updater): it finds its release feed in
# nsh/assets/nsh-updater.json, written below from nsh.json's "updater".
# publish.sh puts the archives of every platform in a release.
set -euo pipefail
cd -- "$(dirname -- "${BASH_SOURCE[0]}")/.."
export PATH="$HOME/.cargo/bin:$HOME/.nsh/bin:$PATH"

nsh_version=0.6.1
if ! nsh --version 2>/dev/null | grep -q "^nsh $nsh_version "; then
  echo "NSH $nsh_version is required: NSH_VERSION=v$nsh_version sh <(curl -fsSL https://nsh.dev/install.sh)" >&2
  exit 1
fi
case $(uname -s) in
  Linux) os=linux ;;
  Darwin) os=macos ;;
  MINGW* | MSYS* | CYGWIN*) os=windows ;;
  *) echo "build.sh: unsupported system $(uname -s)" >&2; exit 1 ;;
esac
case $(uname -m) in
  x86_64 | amd64) arch=x86_64 ;;
  aarch64 | arm64) arch=aarch64 ;;
  *) echo "build.sh: unsupported architecture $(uname -m)" >&2; exit 1 ;;
esac
python=$(command -v python3 || command -v python)

version=$("$python" -c 'import json; print(json.load(open("nsh.json"))["version"])')
build=$("$python" -c 'import json; print(json.load(open("nsh.json")).get("build", 1))')
export SOURCE_DATE_EPOCH=${SOURCE_DATE_EPOCH:-$(git log -1 --format=%ct 2>/dev/null || date +%s)}

cargo build --locked --release -p pansy-desktop

name=pansy-$version-$os-$arch
package=target/package/$name
rm -rf -- "$package"
mkdir -p "$package" ../dist
# what the archive holds, where the program goes in it, where the window's code
# and where the phone's page (pansy-remote's server.rs, web_dir)
case $os in
  linux) stage=$package/pansy program=$stage/pansy resources=$stage/nsh mobile=$stage/mobile-web ;;
  macos) stage=$package/Pansy.app program=$stage/Contents/MacOS/pansy resources=$stage/Contents/Resources/nsh mobile=$stage/Contents/Resources/mobile-web ;;
  windows) stage=$package/pansy program=$stage/pansy.exe resources=$stage/nsh mobile=$stage/mobile-web ;;
esac
mkdir -p "$(dirname "$program")" "$(dirname "$resources")"

nsh build App.tsx --release --out "$resources"
# where the window looks for updates, and what it is now (nsh/updater)
"$python" - "$resources/assets/nsh-updater.json" <<'PY'
import json, sys
app = json.load(open("nsh.json"))
if "updater" in app:
    config = {"id": app["id"], "version": app["version"], "build": app.get("build", 1), **app["updater"]}
    with open(sys.argv[1], "w") as out:
        json.dump(config, out)
PY

# the phone's page, the same on every system: `nsh package` writes it in
# a directory named after the app (Pansy)
web=target/package/mobile-web
rm -rf -- "$web"
(cd ../mobile && nsh package --target web -o "$OLDPWD/$web" App.tsx)
cp -R "$web/Pansy" "$mobile"

case $os in
  linux)
    install -m 755 target/release/pansy "$program"
    mkdir -p "$stage/share/applications" "$stage/share/icons"
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
      -C "$package" -cf - pansy | gzip -n -9 > "$archive"
    ;;
  macos)
    install -m 755 target/release/pansy "$program"
    # the icon at the sizes Finder asks for, from the one PNG
    iconset=$package/pansy.iconset
    mkdir -p "$iconset"
    for size in 16 32 128 256; do
      sips -z $size $size packaging/pansy.png --out "$iconset/icon_${size}x${size}.png" > /dev/null
    done
    cp "$iconset/icon_32x32.png" "$iconset/icon_16x16@2x.png"
    cp "$iconset/icon_256x256.png" "$iconset/icon_128x128@2x.png"
    iconutil -c icns "$iconset" -o "$stage/Contents/Resources/pansy.icns"
    cat > "$stage/Contents/Info.plist" <<PLIST
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
  <key>CFBundleName</key><string>Pansy</string>
  <key>CFBundleDisplayName</key><string>Pansy</string>
  <key>CFBundleIdentifier</key><string>dev.pansy.app</string>
  <key>CFBundleExecutable</key><string>pansy</string>
  <key>CFBundleIconFile</key><string>pansy</string>
  <key>CFBundlePackageType</key><string>APPL</string>
  <key>CFBundleInfoDictionaryVersion</key><string>6.0</string>
  <key>CFBundleShortVersionString</key><string>$version</string>
  <key>CFBundleVersion</key><string>$build</string>
  <key>LSMinimumSystemVersion</key><string>14.0</string>
  <key>LSApplicationCategoryType</key><string>public.app-category.developer-tools</string>
  <key>NSHighResolutionCapable</key><true/>
  <key>CFBundleURLTypes</key>
  <array>
    <dict>
      <key>CFBundleURLName</key><string>dev.pansy.app</string>
      <key>CFBundleURLSchemes</key><array><string>pansy</string></array>
    </dict>
  </array>
</dict>
</plist>
PLIST
    # the updater takes a new Pansy.app only with a signature that holds
    codesign --force --deep --sign "${PANSY_CODESIGN_IDENTITY:--}" "$stage"
    archive=../dist/$name.tar.gz
    COPYFILE_DISABLE=1 tar -C "$package" -czf "$archive" Pansy.app
    ;;
  windows)
    install -m 755 target/release/pansy.exe "$program"
    archive=../dist/$name.zip
    rm -f -- "$archive"
    if command -v 7z > /dev/null; then
      (cd "$package" && 7z a -tzip -mx=9 -bso0 "$OLDPWD/$archive" pansy)
    else
      powershell.exe -NoProfile -Command "Compress-Archive -Path '$(cygpath -w "$stage")' -DestinationPath '$(cygpath -w "$archive")'"
    fi
    ;;
esac
echo "$archive"
