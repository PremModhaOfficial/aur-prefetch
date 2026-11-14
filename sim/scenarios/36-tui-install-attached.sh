#!/usr/bin/env bash
# The armed install gate runs the exact CLI path attached: yay owns the
# terminal, the update installs, the TUI restores and re-collects, and the
# result is visible. This is the disposable container, never the host.
source /usr/local/lib/sim/lib.sh

echo "scenario: the tui install runs the attached upgrade and restores the terminal"

state_dir=/home/installer/.local/state/aur-prefetch
screen=$(mktemp)
driver=$(mktemp)

before_usr=$(usr_state)
before_pkgs=$(pkg_state)

# An allowlist, because a missing trust file now blocks the gate: the TUI
# never supplies the CLI's `--yes` (scenario 47 proves that side).
mkdir -p /etc/aurto
printf 'maintainer\n' >/etc/aurto/trusted-users

rm -rf "$state_dir"
: >/var/log/sim-sudo.log
: >/var/log/sim-yay.log
: >/var/log/sim-yay-build.log

cat >"$driver" <<'EOF'
#!/bin/bash
script -qec 'stty rows 40 cols 120; aur-prefetch tui; rc=$?; stty -a | grep -q icanon && test "$rc" = 0' /dev/null
EOF
chmod 755 "$driver"

# One second for the static frame, i, one second for the gate, space to attest,
# one second, enter to run the attached install, six seconds for the upgrade
# and the re-collected result frame, then q.
( sleep 1; printf 'i'; sleep 1; printf ' '; sleep 1; printf '\r'; sleep 6; printf 'q' ) \
    | timeout 40 runuser -l installer -c "$driver" >"$screen" 2>&1
status=$?
assert_eq "$status" "0" "the TUI exited 0 and restored the terminal"

clean=$(sed -e 's/\x1b\[[0-9;?]*[a-zA-Z]//g' -e 's/\x1b[()][A-Za-z0-9]//g' "$screen" | tr -d '\r')

# 1. The gate armed the action and the attached command ran.
assert_contains "$clean" "enter install" "the attested gate arms the install"
assert_contains "$clean" "install: running yay -Syuu" "the CLI path ran attached"
assert_contains "$clean" "install: done" "the CLI reported completion"

# The install restores the terminal instead of stopping the process, so yay's
# own pacman output is visible rather than hidden inside the alternate screen.
# A SIGTSTP stop cannot be observed here: under `script` the process group is
# orphaned and the signal is discarded, which is why this asserts the output.
assert_contains "$clean" "upgrading simapp" "the attached command's output is visible"

# 1b. The post-install receipt owns the screen and reports the exit, kernel,
#     pacnew state and reboot advice; it is read-only.
assert_contains "$clean" "Post-install" "the receipt is shown"
assert_contains "$clean" "exited 0" "the receipt carries the real exit"
assert_contains "$clean" "kernel" "the receipt names the kernel"
assert_contains "$clean" "pacdiff" "the receipt names pacdiff"
assert_contains "$clean" ".pacnew" "the receipt reports the pacnew state"

# 2. The upgrade really happened in this container.
assert_eq "$(installed_version)" "2.0.0-1" "the pending package was installed"
assert_ne "$(usr_state)" "$before_usr" "/usr changed"
assert_ne "$(pkg_state)" "$before_pkgs" "the package database changed"
assert_eq "$(payload_version)" "2.0.0" "the payload reports the new version"

# 3. The exact helper and command were used; no AUR build.
assert_contains "$(yay_log)" "-Syuu" "yay ran the exact upgrade"
assert_contains "$(cat /var/log/sim-sudo.log)" "/usr/local/bin/sim-upgrade" \
    "the one escalation is the scoped sim helper"
assert_false "the install never reaches the AUR build path" yay_built_something

# 4. The install was recorded once.
state_files=$(find "$state_dir" -type f -printf '%P\n' 2>/dev/null | sort)
assert_eq "$state_files" "state.json" "the install wrote the state file"
assert_contains "$(cat "$state_dir/state.json" 2>/dev/null)" '"last_install_unix"' \
    "the install time was recorded"

finish
