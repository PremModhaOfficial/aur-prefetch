#!/usr/bin/env bash
# The AUR build path must fail loudly, never silently pass.
source /usr/local/lib/sim/lib.sh

echo "scenario: the AUR build path fails loudly"

touch /var/log/sim-yay.log /var/log/sim-yay-build.log
chmod 0666 /var/log/sim-yay.log /var/log/sim-yay-build.log

log=$(mktemp)

runuser -l installer -c 'yay -S ghostpkg' >"$log" 2>&1
status=$?

assert_ne "$status" "0" "the build path exited non-zero"
assert_contains "$(cat "$log")" "AUR build path is not simulated" "the failure is loud"
assert_true "the build attempt was recorded" test -s /var/log/sim-yay-build.log

finish
