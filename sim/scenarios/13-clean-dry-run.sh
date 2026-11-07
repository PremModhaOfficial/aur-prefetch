#!/usr/bin/env bash
# clean: dry run is the default; --apply removes only installed old builds.
source /usr/local/lib/sim/lib.sh

echo "scenario: clean is a dry run unless --apply"

mkdir -p /root/.cache/yay/simapp /root/.cache/yay/ghost
touch -d '40 days ago' /root/.cache/yay/simapp /root/.cache/yay/ghost

dry_log=$(mktemp)
apply_log=$(mktemp)
before_usr=$(usr_state)

aur-prefetch clean >"$dry_log" 2>&1
status=$?

assert_eq "$status" "0" "dry run exited 0"
assert_true "dry run kept the installed build" test -d /root/.cache/yay/simapp
assert_true "dry run kept the ghost build" test -d /root/.cache/yay/ghost
assert_contains "$(cat /var/log/sim-paccache.log)" "-d" "paccache ran in dry-run mode"
assert_contains "$(cat "$dry_log")" "dry run" "the output says it was a dry run"
assert_contains "$(cat "$dry_log")" "aurto repo" "the aurto repo was reported"

aur-prefetch clean --apply >"$apply_log" 2>&1
status=$?

assert_eq "$status" "0" "apply exited 0"
assert_false "an installed old build was removed" test -d /root/.cache/yay/simapp
assert_true "a non-installed build was kept" test -d /root/.cache/yay/ghost
assert_contains "$(cat /var/log/sim-paccache.log)" "-r" "paccache ran in remove mode"
assert_eq "$(usr_state)" "$before_usr" "clean never touches /usr"

finish
