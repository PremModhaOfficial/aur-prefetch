#!/usr/bin/env bash
# Publish one ad-hoc package into the sim repo.
#
#   make-pkg.sh NAME VERSION SIZE_BYTES
#
# The package holds a single file, /usr/share/NAME/blob, of SIZE zero bytes
# (zeros compress, so the image does not grow). A chaos scenario that needs a
# payload larger than a file-size limit, or a payload big enough to be worth
# interrupting, uses this; make-repo.sh still owns the two deterministic simapp
# versions.
set -euo pipefail

REPO=/srv/simrepo
name=${1:?usage: make-pkg.sh NAME VERSION SIZE_BYTES}
version=${2:?usage: make-pkg.sh NAME VERSION SIZE_BYTES}
bytes=${3:?usage: make-pkg.sh NAME VERSION SIZE_BYTES}

dir=$(mktemp -d)
mkdir -p "$dir/usr/share/$name"
head -c "$bytes" /dev/zero >"$dir/usr/share/$name/blob"

isize=$(du -sb "$dir/usr" | cut -f1)
cat >"$dir/.PKGINFO" <<EOF
pkgname = $name
pkgbase = $name
pkgver = $version-1
pkgdesc = Deterministic package for aur-prefetch simulation tests
url = https://example.invalid/aur-prefetch/sim
builddate = $(date -u +%s)
packager = aur-prefetch sim harness
size = $isize
arch = x86_64
license = MIT
EOF

(cd "$dir" && bsdtar -czf .MTREE --format=mtree \
    --options='!all,use-set,type,uid,gid,mode,time,size,md5,sha256,link' usr)

(cd "$dir" && bsdtar -c --zstd -f "$REPO/$name-$version-1-x86_64.pkg.tar.zst" \
    .PKGINFO .MTREE usr)

repo-add "$REPO/sim.db.tar.gz" "$REPO/$name-$version-1-x86_64.pkg.tar.zst" >/dev/null
rm -rf "$dir"
echo "published $name $version-1 ($bytes bytes)"
