#!/usr/bin/env bash
# doctor on an unconfigured container: exit 1 and name each missing piece.
source /usr/local/lib/sim/lib.sh

echo "scenario: doctor reports a missing setup"

log=$(mktemp)
before_usr=$(usr_state)
before_cache=$(cache_files)

aur-prefetch doctor >"$log" 2>&1
status=$?

assert_eq "$status" "1" "doctor exited 1"
assert_contains "$(cat "$log")" "timer: not scheduled" "doctor named the timer"
assert_contains "$(cat "$log")" "sudoers:" "doctor named the sudoers rule"
assert_contains "$(cat "$log")" "trust:" "doctor named the trust file"
assert_contains "$(cat "$log")" "repo:" "doctor named the repo include"
assert_eq "$(usr_state)" "$before_usr" "doctor left /usr unchanged"
assert_eq "$(cache_files)" "$before_cache" "doctor left the cache unchanged"

finish
