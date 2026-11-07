#!/usr/bin/env bash
# install with --yes: the upgrade runs, /usr changes, no AUR build happens.
source /usr/local/lib/sim/lib.sh

echo "scenario: install upgrades without building from the AUR"

# The helper runs as installer; make its log writable for this container.
touch /var/log/sim-yay.log
chmod 0666 /var/log/sim-yay.log

log=$(mktemp)
before_usr=$(usr_state)

runuser -l installer -c 'aur-prefetch install --yes' >"$log" 2>&1
status=$?

assert_eq "$status" "0" "install exited 0"
assert_eq "$(installed_version)" "2.0.0-1" "the pending package was installed"
assert_ne "$(usr_state)" "$before_usr" "/usr changed"
assert_contains "$(yay_log)" "-Syuu" "yay was asked for the full upgrade"
assert_false "install ran no AUR build" yay_built_something
assert_contains "$(cat "$log")" "sudo pacdiff" "the pacdiff follow-up was printed"

finish
