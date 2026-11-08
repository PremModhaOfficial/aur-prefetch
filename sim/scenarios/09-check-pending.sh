#!/usr/bin/env bash
# check with a pending update: exit 1, name the package, write nothing.
source /usr/local/lib/sim/lib.sh

echo "scenario: check reports a pending update"

log=$(mktemp)
before_usr=$(usr_state)
before_pkgs=$(pkg_state)
before_cache=$(cache_files)

runuser -l builder -c 'aur-prefetch check' >"$log" 2>&1
status=$?

assert_eq "$status" "1" "check exited 1"
assert_contains "$(cat "$log")" "simapp" "check named the pending package"
assert_contains "$(cat "$log")" "official updates: 1" "check counted one official update"
assert_eq "$(usr_state)" "$before_usr" "check left /usr unchanged"
assert_eq "$(pkg_state)" "$before_pkgs" "check installed nothing"
assert_eq "$(cache_files)" "$before_cache" "check left the cache unchanged"
assert_false "check wrote no state" test -e /home/builder/.local/state/aur-prefetch/state.json

finish
