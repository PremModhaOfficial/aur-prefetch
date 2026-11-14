#!/usr/bin/env bash
# clean: dry run is the default; --apply removes only installed old builds.
source /usr/local/lib/sim/lib.sh

echo "scenario: clean is a dry run unless --apply"

mkdir -p /root/.cache/yay/simapp /root/.cache/yay/ghost
# A real build leaves its package behind; the name in the file is what makes
# the build identifiable, not the directory name.
: >/root/.cache/yay/simapp/simapp-1.0.0-1-x86_64.pkg.tar.zst
: >/root/.cache/yay/ghost/ghost-1.0.0-1-x86_64.pkg.tar.zst
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
assert_contains "$(cat "$dry_log")" "paccache -r -c" "the dry run prints the apply command"
assert_contains "$(cat "$dry_log")" "aurto repo" "the aurto repo was reported"

aur-prefetch clean --apply >"$apply_log" 2>&1
status=$?

assert_eq "$status" "0" "apply exited 0"
assert_false "an installed old build was removed" test -d /root/.cache/yay/simapp
assert_true "a non-installed build was kept" test -d /root/.cache/yay/ghost
assert_contains "$(cat /var/log/sim-paccache.log)" "-r" "paccache ran in remove mode"
assert_eq "$(usr_state)" "$before_usr" "clean never touches /usr"

finish
