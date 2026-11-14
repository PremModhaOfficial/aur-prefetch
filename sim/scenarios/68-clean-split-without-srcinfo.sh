#!/usr/bin/env bash
# clean: a split build whose .SRCINFO is gone must not leak forever (#68).
#
# The build directory is named after the pkgbase, which is not an installed
# name for a split package: `splitpkg` installs `splitpkg-cli`. Deriving the
# name from the directory therefore classified a real, installed build as
# "not installed" and kept it forever. Here a real split package is built with
# makepkg, one of its packages is installed with pacman, .SRCINFO is deleted,
# and clean --apply must remove the build.
#
# Nothing else may be deleted: a build whose package is not installed stays,
# and a build that names nothing at all (an aborted build) is kept with a line
# that says so instead of a guess.
source /usr/local/lib/sim/lib.sh

echo "scenario: clean removes an installed split build with no .SRCINFO"

build=/home/builder/.cache/yay
pkgbase=splitpkg

mkdir -p "$build/$pkgbase"
cat >"$build/$pkgbase/PKGBUILD" <<'EOF'
pkgbase=splitpkg
pkgname=(splitpkg-cli splitpkg-docs)
pkgver=1.0.0
pkgrel=1
arch=(any)
package_splitpkg-cli() {
  install -Dm644 /dev/null "$pkgdir/usr/share/splitpkg-cli/blob"
}
package_splitpkg-docs() {
  install -Dm644 /dev/null "$pkgdir/usr/share/splitpkg-docs/blob"
}
EOF
# yay writes .SRCINFO before it builds; the fallback only matters when it is
# gone, so a stale copy here must not be what makes the build identifiable.
printf 'pkgbase = %s\npkgname = %s-cli\npkgname = %s-docs\n' \
    "$pkgbase" "$pkgbase" "$pkgbase" >"$build/$pkgbase/.SRCINFO"

chown -R builder:builder "$build"
built=$(mktemp)
runuser -l builder -c "cd $build/$pkgbase && makepkg --nodeps --noconfirm -f" >"$built" 2>&1
status=$?
assert_eq "$status" "0" "makepkg built the real split package"
assert_true "the cli package was built" \
    test -f "$build/$pkgbase/splitpkg-cli-1.0.0-1-any.pkg.tar.zst"
assert_true "the docs package was built" \
    test -f "$build/$pkgbase/splitpkg-docs-1.0.0-1-any.pkg.tar.zst"

# Install one half of the split, through pacman itself.
pacman -U --noconfirm "$build/$pkgbase/splitpkg-cli-1.0.0-1-any.pkg.tar.zst" \
    >/dev/null 2>&1
assert_eq "$(pacman -Q splitpkg-cli 2>/dev/null)" "splitpkg-cli 1.0.0-1" \
    "pacman installed the split package"
if pacman -Q "$pkgbase" >/dev/null 2>&1; then
    fail "the pkgbase is not an installed package"
else
    pass "the pkgbase is not an installed package"
fi

# pacman cannot answer "who owns this build directory": a build directory is
# not in any package's file list. That is why the built package names itself.
ownership=$(pacman -Qqo "$build/$pkgbase" 2>&1)
assert_ne "$?" "0" "pacman -Qqo finds no owner for the build directory"
assert_contains "$ownership" "No package owns" \
    "ownership refuses rather than names a package"

# Two builds that must survive: one whose package is not installed, and one
# that names nothing at all.
mkdir -p "$build/orphan" "$build/aborted"
cp "$build/$pkgbase/splitpkg-docs-1.0.0-1-any.pkg.tar.zst" \
    "$build/orphan/orphan-1.0.0-1-any.pkg.tar.zst"

# The build directory is a real yay build whose .SRCINFO is gone.
rm -f "$build/$pkgbase/.SRCINFO"
assert_false ".SRCINFO is gone from the split build" test -e "$build/$pkgbase/.SRCINFO"
touch -d '40 days ago' "$build/$pkgbase" "$build/orphan" "$build/aborted"

before_usr=$(tree_state /usr/share/splitpkg-cli /usr/share/splitpkg-docs)
yay_cache=$build

dry_log=$(mktemp)
AUR_PREFETCH_YAY_CACHE_DIR=$yay_cache aur-prefetch clean >"$dry_log" 2>&1
assert_eq "$?" "0" "the dry run exited 0"
assert_contains "$(cat "$dry_log")" "splitpkg: would remove" \
    "the dry run would remove the installed split build"
assert_contains "$(cat "$dry_log")" "orphan: keep (not installed)" \
    "the dry run keeps a build whose package is not installed"
assert_contains "$(cat "$dry_log")" "aborted: keep (cannot tell" \
    "the dry run says why a build it cannot identify is kept"
assert_true "the dry run deleted nothing" \
    test -d "$build/$pkgbase"

apply_log=$(mktemp)
AUR_PREFETCH_YAY_CACHE_DIR=$yay_cache aur-prefetch clean --apply >"$apply_log" 2>&1
assert_eq "$?" "0" "apply exited 0"
assert_false "the installed split build did not survive" \
    test -d "$build/$pkgbase"
assert_contains "$(cat "$apply_log")" "splitpkg: removed" \
    "the output names the removed build"
assert_true "a build whose package is not installed was kept" \
    test -d "$build/orphan"
assert_true "a build that names nothing was kept" test -d "$build/aborted"
assert_eq "$(tree_state /usr/share/splitpkg-cli /usr/share/splitpkg-docs)" \
    "$before_usr" "clean never touches /usr"
assert_eq "$(pacman -Q splitpkg-cli 2>/dev/null)" "splitpkg-cli 1.0.0-1" \
    "the installed package is still installed"

finish
