#!/usr/bin/env bash
# init: as the unprivileged builder, escalate only the sudoers install, and
# write the rule for builder, not for root. The base image grants builder no
# sudo except pacman -Syuw, so this scenario adds a narrow rule at run time for
# exactly the two commands init uses. That rule is what proves the sudo path a
# normal user hits: writing /etc/sudoers.d directly fails with EACCES.
source /usr/local/lib/sim/lib.sh

echo "scenario: init applies as builder and escalates only the sudoers rule"

log=$(mktemp)

# Under sudo, USER is root and HOME is /root. init must refuse instead of
# writing root's units and a root sudoers rule.
if env SUDO_USER=builder aur-prefetch init --apply >"$log" 2>&1; then
    fail "init under sudo exited 0"
else
    pass "init under sudo was refused"
fi
assert_contains "$(cat "$log")" "as your user" "the refusal says to run as the user"
assert_false "a refused sudo run wrote no sudoers rule" test -e /etc/sudoers.d/aur-prefetch
assert_false "a refused sudo run wrote no root unit" test -e /root/.config/systemd/user/aur-prefetch.service

# The two commands init runs through sudo, and nothing else. Inputs carry a
# pid/nanos suffix, so the rule matches the prefix.
printf '%s\n' \
    'Defaults logfile=/var/log/sim-sudo.log' \
    'builder ALL=(root) NOPASSWD: /usr/bin/install -m 0440 /tmp/aur-prefetch-sudoers-* /etc/sudoers.d/.aur-prefetch-sudoers-install-*, /usr/bin/mv /etc/sudoers.d/.aur-prefetch-sudoers-install-* /etc/sudoers.d/aur-prefetch' \
    >/etc/sudoers.d/sim-init
chmod 0440 /etc/sudoers.d/sim-init
if ! visudo -cf /etc/sudoers.d/sim-init >/dev/null 2>&1; then
    fail "the scenario's own narrow sudo rule is invalid"
    finish
fi

# builder must not be able to write the directory directly, or the sudo
# fallback would never run and the test would pass for the wrong reason.
if runuser -u builder -- touch /etc/sudoers.d/sim-write-probe >/dev/null 2>&1; then
    rm -f /etc/sudoers.d/sim-write-probe
    fail "builder can write /etc/sudoers.d directly"
else
    pass "builder cannot write /etc/sudoers.d directly"
fi

runuser -l builder -c 'aur-prefetch init --apply' >"$log" 2>&1
status=$?

assert_eq "$status" "0" "init as builder exited 0"
assert_contains "$(cat "$log")" "init: wrote /etc/sudoers.d/aur-prefetch" "init reported the rule"

assert_true "the rule landed" test -f /etc/sudoers.d/aur-prefetch
assert_eq "$(stat -c %a /etc/sudoers.d/aur-prefetch)" "440" "the rule is mode 0440"
assert_true "visudo accepts the installed rule" visudo -cf /etc/sudoers.d/aur-prefetch
assert_contains "$(cat /etc/sudoers.d/aur-prefetch)" "builder ALL=(root) NOPASSWD: /usr/bin/pacman -Syuw --noconfirm" "the rule names builder"
assert_false "the rule does not name root" grep -q '^root ' /etc/sudoers.d/aur-prefetch
assert_eq "$(find /etc/sudoers.d -maxdepth 1 -name '.aur-prefetch-sudoers*' | wc -l | tr -d ' ')" "0" "no staged file is left behind"

assert_true "the service landed in builder's config" test -f /home/builder/.config/systemd/user/aur-prefetch.service
assert_true "the timer landed in builder's config" test -f /home/builder/.config/systemd/user/aur-prefetch.timer
assert_contains "$(cat /home/builder/.config/systemd/user/aur-prefetch.service)" 'ExecStart="/usr/local/bin/aur-prefetch" fetch' "ExecStart is quoted"

sudo_log=$(tr '\n' ' ' </var/log/sim-sudo.log 2>/dev/null | tr -s ' ' || true)
assert_contains "$sudo_log" "COMMAND=/usr/bin/install -m 0440 /tmp/aur-prefetch-sudoers-" "the rule was staged with the exact install argv"
assert_contains "$sudo_log" " /etc/sudoers.d/.aur-prefetch-sudoers-install-" "the staged file is hidden from sudo's include dir"
assert_contains "$sudo_log" "COMMAND=/usr/bin/mv /etc/sudoers.d/.aur-prefetch-sudoers-install-" "the staged rule was moved into place"

assert_contains "$(cat /var/log/sim-systemctl.log)" "--user daemon-reload" "a reload ran as builder"
assert_contains "$(cat /var/log/sim-systemctl.log)" "--user enable --now aur-prefetch.timer" "the timer was enabled as builder"
assert_false "aurto was never run" bash -c 'test -s /var/log/sim-aurto.log'

finish
