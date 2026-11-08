#!/usr/bin/env bash
# check on an up-to-date container: exit 0 and write nothing.
source /usr/local/lib/sim/lib.sh

echo "scenario: check is clean when the system is current"

pacman -Syu --noconfirm >/dev/null

log=$(mktemp)
before_usr=$(usr_state)

runuser -l builder -c 'aur-prefetch check' >"$log" 2>&1
status=$?

assert_eq "$status" "0" "check exited 0"
assert_contains "$(cat "$log")" "check: clean" "check reported clean"
assert_eq "$(usr_state)" "$before_usr" "check left /usr unchanged"
assert_false "check wrote no state" test -e /home/builder/.local/state/aur-prefetch/state.json

finish
