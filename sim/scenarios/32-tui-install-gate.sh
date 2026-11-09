#!/usr/bin/env bash
# The install gate is a read-only review surface: it runs the list queries,
# shows the exact command, accepts a session-local news attestation, and
# installs nothing.
source /usr/local/lib/sim/lib.sh

echo "scenario: the install gate reviews and cancels without installing"

state_dir=/home/builder/.local/state/aur-prefetch
screen=$(mktemp)
driver=$(mktemp)

before_usr=$(usr_state)
before_pkgs=$(pkg_state)
before_cache=$(cache_files)

rm -rf "$state_dir"
: >/var/log/sim-sudo.log
: >/var/log/sim-yay.log
: >/var/log/sim-yay-build.log

cat >"$driver" <<'EOF'
#!/bin/bash
script -qec "stty rows 40 cols 120; aur-prefetch tui" /dev/null
EOF
chmod 755 "$driver"

# One second for the static frame, i to open the gate, one second to review,
# space to attest the news for this session, one second to see the attested
# line, esc to cancel, then q.
( sleep 1; printf 'i'; sleep 1; printf ' '; sleep 1; printf '\033'; sleep 1; printf 'q' ) \
    | timeout 30 runuser -l builder -c "$driver" >"$screen" 2>&1
status=$?
assert_eq "$status" "0" "the TUI exited 0 after cancelling the gate"

clean=$(sed -e 's/\x1b\[[0-9;?]*[a-zA-Z]//g' -e 's/\x1b[()][A-Za-z0-9]//g' "$screen" | tr -d '\r')

# 1. The overlay shows the real checks and the exact command.
assert_contains "$clean" "Install" "the gate is open"
assert_contains "$clean" "PRE-FLIGHT" "the gate has a checklist"
assert_contains "$clean" "root" "the root gate ran"
assert_contains "$clean" "lock" "the lock gate ran"
assert_contains "$clean" "lists" "the lists gate ran"
assert_contains "$clean" "trust" "the trust gate ran"
assert_contains "$clean" "archlinux.org/news" "the news URL is shown"
assert_contains "$clean" "COMMAND   yay -Syuu" "the exact command is named"
assert_contains "$clean" "space attest" "the attestation key is offered"

# 2. Space attests for this session only, and esc cancels.
assert_contains "$clean" "attested for this session" "space attests the news"
assert_contains "$clean" "esc cancel" "esc cancels"

# 3. Nothing was installed, downloaded, recorded, or escalated.
assert_eq "$(usr_state)" "$before_usr" "the gate left /usr unchanged"
assert_eq "$(pkg_state)" "$before_pkgs" "the gate installed nothing"
assert_eq "$(installed_version)" "1.0.0-1" "the pending upgrade is still pending"
assert_eq "$(cache_files)" "$before_cache" "the gate downloaded nothing"
assert_eq "$(find "$state_dir" -type f -printf '%P\n' 2>/dev/null | sort)" "" \
    "the gate wrote no state"
assert_eq "$(grep -c . /var/log/sim-sudo.log 2>/dev/null || true)" "0" \
    "the gate escalated nothing"
assert_contains "$(yay_log)" "-Qua" "the gate only asked yay for AUR updates"
assert_false "the gate never runs yay -Syuu" grep -q -- -Syuu /var/log/sim-yay.log
assert_false "the gate never reaches the AUR build path" yay_built_something

finish
