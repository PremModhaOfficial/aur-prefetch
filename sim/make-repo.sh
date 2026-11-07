#!/usr/bin/env bash
# Build the deterministic pacman repo used by the simulation scenarios.
#
# End state inside the image:
#   simapp 1.0.0-1 installed
#   simapp 2.0.0-1 available as an upgrade
#   download cache empty
#
# No network, no keys, no AUR. Packages are hand-built tar archives with a
# .PKGINFO and .MTREE, which is the same shape makepkg produces.
set -euo pipefail

REPO=/srv/simrepo
WORK=/srv/simbuild
PKG=simapp
OLD=1.0.0
NEW=2.0.0

rm -rf "$REPO" "$WORK"
mkdir -p "$REPO" "$WORK"

build_pkg() {
    local ver=$1
    local dir="$WORK/$PKG-$ver"

    mkdir -p "$dir/usr/bin" "$dir/usr/share/$PKG"
    printf '#!/bin/sh\necho "%s %s"\n' "$PKG" "$ver" >"$dir/usr/bin/$PKG"
    chmod 755 "$dir/usr/bin/$PKG"
    printf '%s\n' "$ver" >"$dir/usr/share/$PKG/version"

    local isize
    isize=$(du -sb "$dir/usr" | cut -f1)

    cat >"$dir/.PKGINFO" <<EOF
pkgname = $PKG
pkgbase = $PKG
pkgver = $ver-1
pkgdesc = Deterministic package for aur-prefetch simulation tests
url = https://example.invalid/aur-prefetch/sim
builddate = $(date -u +%s)
packager = aur-prefetch sim harness
size = $isize
arch = x86_64
license = MIT
EOF

    # makepkg-equivalent metadata archive, so `pacman -Qkk` has something real
    # to verify in corruption scenarios.
    (cd "$dir" && bsdtar -czf .MTREE --format=mtree \
        --options='!all,use-set,type,uid,gid,mode,time,size,md5,sha256,link' usr)

    (cd "$dir" && bsdtar -c --zstd -f "$REPO/$PKG-$ver-1-x86_64.pkg.tar.zst" \
        .PKGINFO .MTREE usr)
}

# 1. Publish 1.0.0, install it, then publish 2.0.0 on top.
build_pkg "$OLD"
repo-add "$REPO/sim.db.tar.gz" "$REPO/$PKG-$OLD-1-x86_64.pkg.tar.zst" >/dev/null

pacman -Sy --noconfirm >/dev/null
pacman -S --noconfirm "$PKG" >/dev/null

build_pkg "$NEW"
repo-add "$REPO/sim.db.tar.gz" "$REPO/$PKG-$NEW-1-x86_64.pkg.tar.zst" >/dev/null
pacman -Sy --noconfirm >/dev/null

rm -rf "$WORK"
rm -f /var/cache/pacman/pkg/*

echo "sim repo ready:"
pacman -Q "$PKG"
pacman -Qu "$PKG"
