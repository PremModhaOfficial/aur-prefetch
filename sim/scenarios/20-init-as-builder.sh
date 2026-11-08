#!/usr/bin/env bash
# init: as the unprivileged builder, escalate only the sudoers install, and
# write the rule for builder, not for root. A test double for sudo records the
# exact argv init asks for; the scenario replays the install and move as root.
# No wildcard sudoers rule is written to exercise the fallback.
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

# Test double: sudo logs the exact argv and returns 0. It does not escalate.
# The scenario replays install and move as root below, so the fixture never
# grants builder a wildcard rule.
touch /var/log/sim-sudo.log
chmod 0666 /var/log/sim-sudo.log
cat >/usr/local/bin/sudo <<'EOF'
#!/usr/bin/env bash
{
    printf 'SUDO'
    printf ' %s' "$@"
    printf '\n'
} >>/var/log/sim-sudo.log
if [[ ${1:-} == install ]]; then
    cp -- "$4" /tmp/sim-init-rule
fi
exit 0
EOF
chmod 0755 /usr/local/bin/sudo

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

mapfile -t sudo_lines < <(grep '^SUDO ' /var/log/sim-sudo.log || true)
assert_eq "${#sudo_lines[@]}" "3" "init escalates exactly three commands"
assert_contains "${sudo_lines[0]:-}" "SUDO visudo -cf /tmp/aur-prefetch-sudoers-" "the rendered rule was validated by root visudo"
assert_contains "${sudo_lines[1]:-}" "SUDO install -m 0440 /tmp/aur-prefetch-sudoers-" "the rule was staged with the exact install argv"
assert_contains "${sudo_lines[1]:-}" " /etc/sudoers.d/.aur-prefetch-sudoers-install-" "the staged file is hidden from sudo's include dir"
assert_contains "${sudo_lines[2]:-}" "SUDO mv /etc/sudoers.d/.aur-prefetch-sudoers-install-" "the staged rule was moved into place"
assert_contains "${sudo_lines[2]:-}" " /etc/sudoers.d/aur-prefetch" "the move targets the final rule"

install_line=${sudo_lines[1]:-}
staged=$(awk '{print $6}' <<<"$install_line")
assert_contains "$staged" "/etc/sudoers.d/.aur-prefetch-sudoers-install-" "the install dest is the hidden staged path"

# Replay what sudo would have done, from the rule init rendered and the paths
# it asked sudo to use.
assert_true "the rendered rule was captured" test -s /tmp/sim-init-rule
assert_eq "$(cat /tmp/sim-init-rule)" "builder ALL=(root) NOPASSWD: /usr/bin/pacman -Syuw --noconfirm" "the rule names builder"
install -m 0440 /tmp/sim-init-rule "$staged"
mv "$staged" /etc/sudoers.d/aur-prefetch

assert_true "the rule landed" test -f /etc/sudoers.d/aur-prefetch
assert_eq "$(stat -c %a /etc/sudoers.d/aur-prefetch)" "440" "the rule is mode 0440"
assert_true "visudo accepts the installed rule" visudo -cf /etc/sudoers.d/aur-prefetch
assert_contains "$(cat /etc/sudoers.d/aur-prefetch)" "builder ALL=(root) NOPASSWD: /usr/bin/pacman -Syuw --noconfirm" "the installed rule names builder"
assert_false "the rule does not name root" grep -q '^root ' /etc/sudoers.d/aur-prefetch
assert_eq "$(find /etc/sudoers.d -maxdepth 1 -name '.aur-prefetch-sudoers*' | wc -l | tr -d ' ')" "0" "no staged file is left behind"

assert_true "the service landed in builder's config" test -f /home/builder/.config/systemd/user/aur-prefetch.service
assert_true "the timer landed in builder's config" test -f /home/builder/.config/systemd/user/aur-prefetch.timer
assert_contains "$(cat /home/builder/.config/systemd/user/aur-prefetch.service)" 'ExecStart="/usr/local/bin/aur-prefetch" fetch' "ExecStart is quoted"
assert_eq "$(find /home/builder/.config/systemd/user -maxdepth 1 -name '.*' | wc -l | tr -d ' ')" "0" "no hidden unit temp remains"

assert_contains "$(cat /var/log/sim-systemctl.log)" "--user daemon-reload" "a reload ran as builder"
assert_contains "$(cat /var/log/sim-systemctl.log)" "--user enable --now aur-prefetch.timer" "the timer was enabled as builder"
assert_false "aurto was never run" bash -c 'test -s /var/log/sim-aurto.log'

finish
