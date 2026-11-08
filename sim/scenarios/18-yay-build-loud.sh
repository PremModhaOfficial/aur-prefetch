#!/usr/bin/env bash
# The AUR build path must fail loudly through `install`, never silently pass.
#
# Scenario 08 proves the fake yay rejects root and replays stubs. Here the
# helper fails loudly on a direct build request, then `install` surfaces that
# same failure as exit 2 instead of reporting success.
source /usr/local/lib/sim/lib.sh

echo "scenario: install surfaces the AUR build failure loudly"

touch /var/log/sim-yay.log /var/log/sim-yay-build.log
chmod 0666 /var/log/sim-yay.log /var/log/sim-yay-build.log

# Direct yay build request: the helper must fail instead of pretending.
direct=$(mktemp)
runuser -l installer -c 'yay -S ghostpkg' >"$direct" 2>&1
status=$?

assert_ne "$status" "0" "a direct yay build exited non-zero"
assert_contains "$(cat "$direct")" "AUR build path is not simulated" "the direct failure is loud"
assert_true "the direct build attempt was recorded" test -s /var/log/sim-yay-build.log

# Same failure through the product: yay's -Syuu path fails, install must not
# print "done" or write a successful install state.
: >/var/log/sim-yay-build.log
touch /etc/sim/yay-syuu-fail
log=$(mktemp)

runuser -l installer -c 'aur-prefetch install --yes' >"$log" 2>&1
status=$?

assert_eq "$status" "2" "install exited 2 on yay's build failure"
assert_contains "$(cat "$log")" "AUR build path is not simulated" "the AUR failure is loud"
assert_contains "$(cat "$log")" "yay -Syuu exited 97" "install names yay's exit code"
assert_false "install did not report success" grep -q '^install: done' "$log"
assert_true "the build attempt was recorded" test -s /var/log/sim-yay-build.log
assert_eq "$(installed_version)" "1.0.0-1" "nothing was installed"

finish
