#!/usr/bin/env bash
# init: dry run prints, apply writes and validates, aurto is never run.
source /usr/local/lib/sim/lib.sh

echo "scenario: init renders and applies a validated setup"

dry_log=$(mktemp)
apply_log=$(mktemp)
before_usr=$(usr_state)

aur-prefetch init >"$dry_log" 2>&1
status=$?

assert_eq "$status" "0" "dry run exited 0"
assert_false "dry run wrote no sudoers" test -e /etc/sudoers.d/aur-prefetch
assert_false "dry run wrote no service" test -e /root/.config/systemd/user/aur-prefetch.service
assert_contains "$(cat "$dry_log")" "NOPASSWD: /usr/bin/pacman -Syuw --noconfirm" "the rule was printed"

aur-prefetch init --apply >"$apply_log" 2>&1
status=$?

assert_eq "$status" "0" "apply exited 0"
assert_true "sudoers written" test -f /etc/sudoers.d/aur-prefetch
assert_true "service written" test -f /root/.config/systemd/user/aur-prefetch.service
assert_true "timer written" test -f /root/.config/systemd/user/aur-prefetch.timer
assert_true "visudo accepts the generated rule" visudo -cf /etc/sudoers.d/aur-prefetch
assert_contains "$(cat /root/.config/systemd/user/aur-prefetch.service)" "ExecStart=/usr/local/bin/aur-prefetch fetch" "the unit calls fetch"
assert_contains "$(cat /var/log/sim-systemctl.log)" "daemon-reload" "systemctl reloaded"
assert_contains "$(cat /var/log/sim-systemctl.log)" "enable" "the timer was enabled"
assert_false "aurto was never run" bash -c 'test -s /var/log/sim-aurto.log'
assert_eq "$(usr_state)" "$before_usr" "init left /usr unchanged"

finish
