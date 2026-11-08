#!/usr/bin/env bash
# install without --yes on a non-TTY must stop at the news gate.
source /usr/local/lib/sim/lib.sh

echo "scenario: install refuses without --yes on a non-TTY"

# The helper runs as installer; make its log writable for this container.
touch /var/log/sim-yay.log
chmod 0666 /var/log/sim-yay.log

log=$(mktemp)
before_usr=$(usr_state)

runuser -l installer -c 'aur-prefetch install' </dev/null >"$log" 2>&1
status=$?

assert_eq "$status" "1" "install refused with exit 1"
assert_contains "$(cat "$log")" "stdin is not a TTY" "the refusal names the non-TTY gate"
assert_eq "$(installed_version)" "1.0.0-1" "nothing was installed"
assert_eq "$(usr_state)" "$before_usr" "/usr is unchanged"
assert_false "yay -Syuu never ran" bash -c 'grep -q -- "-Syuu" /var/log/sim-yay.log 2>/dev/null'

finish
