#!/usr/bin/env bash
# Package an already built release binary as a tarball, a .deb and an AppImage.
#
# Usage: scripts/package.sh <rust-target> [out-dir]
#
# Environment:
#   CARGO_TARGET_DIR  where cargo put the build (default: target)
#   APPIMAGETOOL      appimagetool executable; the AppImage is skipped if unset
#   APPIMAGE_RUNTIME  type2 runtime for the target architecture (needed with APPIMAGETOOL)
set -euo pipefail

target=${1:?usage: scripts/package.sh <rust-target> [out-dir]}
out=${2:-dist}

case $target in
    x86_64-*) arch=x86_64 deb_arch=amd64 appimage_arch=x86_64 ;;
    aarch64-*) arch=aarch64 deb_arch=arm64 appimage_arch=aarch64 ;;
    armv7-*) arch=armv7 deb_arch=armhf appimage_arch=armhf ;;
    i686-*) arch=i686 deb_arch=i386 appimage_arch=i686 ;;
    *) echo "unsupported target: $target" >&2; exit 1 ;;
esac

cd "$(dirname "$0")/.."
version=$(sed -n 's/^version = "\(.*\)"/\1/p' Cargo.toml | head -n1)
bin=${CARGO_TARGET_DIR:-target}/$target/release/magicpen
[ -f "$bin" ] || { echo "missing $bin: build it first" >&2; exit 1; }

mkdir -p "$out"
out=$(cd "$out" && pwd)
work=$(mktemp -d)
trap 'rm -rf "$work"' EXIT

# Files shared by every package, laid out under a /usr prefix.
stage() {
    install -Dm755 "$bin" "$1/usr/bin/magicpen"
    install -Dm644 assets/magicpen.desktop "$1/usr/share/applications/magicpen.desktop"
    install -Dm644 assets/magicpen.svg "$1/usr/share/icons/hicolor/scalable/apps/magicpen.svg"
}

# Tarball
name=magicpen-$version-$arch-linux
mkdir -p "$work/$name"
install -m755 "$bin" "$work/$name/magicpen"
install -m644 README.md LICENSE CHANGELOG.md assets/magicpen.desktop assets/magicpen.svg "$work/$name/"
tar -C "$work" --owner=0 --group=0 -czf "$out/$name.tar.gz" "$name"

# Debian package
deb=$work/deb
stage "$deb"
install -Dm644 LICENSE "$deb/usr/share/doc/magicpen/copyright"
mkdir -p "$deb/DEBIAN"
cat > "$deb/DEBIAN/control" <<EOF
Package: magicpen
Version: $version
Architecture: $deb_arch
Maintainer: SciGeek <SciGeek-saadati@users.noreply.github.com>
Installed-Size: $(du -sk "$deb/usr" | cut -f1)
Section: graphics
Priority: optional
Recommends: fontconfig
Homepage: https://github.com/SciGeek-saadati/magicpen
Description: Draw on top of everything on your screen
 A lightweight annotation overlay for X11: pen, highlighter, shapes and
 eraser on a transparent layer above the desktop, with click-through
 mouse mode and global shortcuts.
EOF
dpkg-deb --root-owner-group -Zxz --build "$deb" "$out/magicpen_${version}_$deb_arch.deb" >/dev/null

# AppImage
if [ -n "${APPIMAGETOOL:-}" ]; then
    appdir=$work/magicpen.AppDir
    stage "$appdir"
    ln -s usr/bin/magicpen "$appdir/AppRun"
    ln -s usr/share/applications/magicpen.desktop "$appdir/magicpen.desktop"
    ln -s usr/share/icons/hicolor/scalable/apps/magicpen.svg "$appdir/magicpen.svg"
    ln -s magicpen.svg "$appdir/.DirIcon"
    ARCH=$appimage_arch "$APPIMAGETOOL" --appimage-extract-and-run --no-appstream \
        --runtime-file "${APPIMAGE_RUNTIME:?APPIMAGE_RUNTIME is required with APPIMAGETOOL}" \
        "$appdir" "$out/magicpen-$version-$arch.AppImage"
fi

ls -l "$out"
