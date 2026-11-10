#!/usr/bin/env bash
# Prefetch badges come from the real cache scan, not a synthetic state.
source /usr/local/lib/sim/lib.sh

echo "scenario: prefetch badges read the real cache"

cache=/var/cache/pacman/pkg
frame=$(mktemp)

before_usr=$(usr_state)
before_pkgs=$(pkg_state)

rm -f "$cache"/simapp-*
runuser -l builder -c 'aur-prefetch tui --render-once' >"$frame" 2>&1
status=$?
assert_eq "$status" "1" "render-once exits 1 for a pending package"
assert_contains "$(cat "$frame")" "simapp" "the frame names simapp"
assert_contains "$(cat "$frame")" "missing" "an empty cache is missing"
assert_contains "$(cat "$frame")" "install 1 package" "the install context is singular"
assert_contains "$(cat "$frame")" "· local" "the file:// repo host is local"
assert_contains "$(cat "$frame")" "0 ready" "missing is not ready"
assert_eq "$(usr_state)" "$before_usr" "the tree left /usr unchanged"
assert_eq "$(pkg_state)" "$before_pkgs" "the tree installed nothing"
assert_false "the tree wrote no state" test -e /home/builder/.local/state/aur-prefetch/state.json

cp /srv/simrepo/simapp-2.0.0-1-x86_64.pkg.tar.zst "$cache"/
runuser -l builder -c 'aur-prefetch tui --render-once' >"$frame" 2>&1
assert_contains "$(cat "$frame")" "cached" "an exact cache file is cached"
assert_contains "$(cat "$frame")" "1 ready" "cached counts as ready"
assert_contains "$(cat "$frame")" "0 missing" "cached is not missing"

rm -f "$cache"/simapp-*
cp /srv/simrepo/simapp-1.0.0-1-x86_64.pkg.tar.zst "$cache"/
runuser -l builder -c 'aur-prefetch tui --render-once' >"$frame" 2>&1
assert_contains "$(cat "$frame")" "stale" "an older cache file is stale"
assert_contains "$(cat "$frame")" "older cache file 1.0.0-1" "the stale reason names the old version"
assert_contains "$(cat "$frame")" "0 ready" "stale is not ready"

rm -f "$cache"/simapp-*
printf 'partial' >"$cache/simapp-2.0.0-1-x86_64.pkg.tar.zst.part"
runuser -l builder -c 'aur-prefetch tui --render-once' >"$frame" 2>&1
assert_contains "$(cat "$frame")" "partial" "a .part file is partial"
assert_contains "$(cat "$frame")" "partial download" "the partial reason is explicit"
assert_contains "$(cat "$frame")" "0 ready" "partial is not ready"

rm -f "$cache"/simapp-*
mkdir -p /var/cache/pacman/aurto
printf 'aurpkg 1.0.0-1 -> 1.1.0-1\n' >/etc/sim/yay-qua.out
cp /srv/simrepo/simapp-2.0.0-1-x86_64.pkg.tar.zst \
    /var/cache/pacman/aurto/aurpkg-1.1.0-1-x86_64.pkg.tar.zst
before_aurto=$(find /var/cache/pacman/aurto -maxdepth 1 -type f -printf '%f\n' | sort)
runuser -l builder -c 'aur-prefetch tui --render-once' >"$frame" 2>&1
assert_contains "$(cat "$frame")" "aurpkg" "the frame names the AUR package"
assert_contains "$(cat "$frame")" "built" "an exact aurto artifact is built"
assert_contains "$(cat "$frame")" "1 ready" "built counts as ready"
assert_contains "$(cat "$frame")" "install 2 packages" "the install context counts both pending packages"
assert_eq "$(find /var/cache/pacman/aurto -maxdepth 1 -type f -printf '%f\n' | sort)" \
    "$before_aurto" "the tree left the aurto repo unchanged"

before_cache=$(cache_files)
runuser -l builder -c 'aur-prefetch tui --render-once' >"$frame" 2>&1
assert_eq "$(cache_files)" "$before_cache" "the tree left the cache unchanged"

finish
