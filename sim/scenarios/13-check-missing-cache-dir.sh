#!/usr/bin/env bash
# check with a missing CacheDir: exit 1, never exit 2 from a failed df.
source /usr/local/lib/sim/lib.sh

echo "scenario: check treats a missing cache dir as actionable"

log=$(mktemp)
before_usr=$(usr_state)

runuser -l builder -c \
    'AUR_PREFETCH_PACMAN_CONF=/usr/local/lib/sim/fixtures/pacman-moved-cache.conf aur-prefetch check' \
    >"$log" 2>&1
status=$?

assert_eq "$status" "1" "check exited 1, not 2"
assert_contains "$(cat "$log")" "cache dir is missing" "check named the missing cache dir"
assert_eq "$(usr_state)" "$before_usr" "check left /usr unchanged"

finish
