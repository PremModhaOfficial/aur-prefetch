#!/usr/bin/env bash
# #31: check is the command the user runs to learn whether the next fetch will
# work. A snapshot config raises fetch's floor to 5 GiB, so with the CacheDir on
# a 64 MiB filesystem both commands must refuse, and check must name the same
# floor. Before the shared floor, check compared against the 1 MiB configured
# value and exited 0 while fetch refused.
source /usr/local/lib/sim/lib.sh

echo "scenario: check and fetch agree on the free-space floor"

mkdir -p /dev/shm/simcache
mkdir -p /etc/snapper/configs
printf '[options]\nCacheDir = /dev/shm/simcache\n' >/tmp/sim-pacman.conf

# The configured floor is 1 MiB; the snapshot floor is 5 GiB; the CacheDir
# filesystem is 64 MiB. Only the shared rule refuses.
envs='AUR_PREFETCH_PACMAN_CONF=/tmp/sim-pacman.conf AUR_PREFETCH_MIN_FREE_BYTES=1048576'

fetch_log=$(mktemp)
runuser -l builder -c "$envs aur-prefetch fetch" >"$fetch_log" 2>&1
fetch_status=$?

check_log=$(mktemp)
runuser -l builder -c "$envs aur-prefetch check" >"$check_log" 2>&1
check_status=$?

assert_eq "$fetch_status" "1" "fetch refused below the snapshot floor"
assert_contains "$(cat "$fetch_log")" "not enough free space" "fetch named free space"
assert_contains "$(cat "$fetch_log")" "/dev/shm/simcache" "fetch named the CacheDir"

assert_eq "$check_status" "1" "check refused on the same machine, not clean"
# The printed floor is `5 GiB + 2 x pending download` rounded to one decimal,
# so assert the verdict and the filesystem, not the truncated number.
assert_contains "$(cat "$check_log")" "below the" \
    "check put the CacheDir below the floor fetch refused on"
assert_contains "$(cat "$check_log")" "disk /dev/shm/simcache:" \
    "check named the CacheDir filesystem"

assert_false "nothing was downloaded" \
    bash -c 'compgen -G "/dev/shm/simcache/simapp-2.0.0*" >/dev/null'
assert_eq "$(installed_version)" "1.0.0-1" "nothing was installed"

finish
