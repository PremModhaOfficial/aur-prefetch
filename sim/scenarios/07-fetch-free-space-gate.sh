#!/usr/bin/env bash
# The free-space gate: a floor larger than the filesystem must refuse before
# pacman runs.
source /usr/local/lib/sim/lib.sh

echo "scenario: fetch refuses below the free-space floor"

runuser -l builder -c 'AUR_PREFETCH_MIN_FREE_BYTES=1099511627776 aur-prefetch fetch' \
    >/tmp/sim-disk.log 2>&1
status=$?

assert_eq "$status" "1" "fetch refused below the floor"
assert_contains "$(cat /tmp/sim-disk.log)" "not enough free space" "the refusal names free space"
assert_false "nothing was downloaded" bash -c 'compgen -G "/var/cache/pacman/pkg/simapp-2.0.0*" >/dev/null'
assert_eq "$(installed_version)" "1.0.0-1" "nothing was installed"

finish
