#!/usr/bin/env bash
# The log screen belongs to its session. Opening the log and then ending the
# session with a refused fetch must not strand the flag: one esc still clears
# the notice, and the app never runs on with a log it could not close.
source /usr/local/lib/sim/lib.sh

echo "scenario: a dismissed session closes the log, so no flag is stranded"

cache=/var/cache/pacman/pkg
state_dir=/home/builder/.local/state/aur-prefetch
screen=$(mktemp)
driver=$(mktemp)
before_usr=$(usr_state)
before_pkgs=$(pkg_state)

rm -f "$cache"/simapp-*
rm -rf "$state_dir"
: >/var/log/sim-sudo.log
assert_true "no lock file to begin with" test ! -f /var/lib/pacman/db.lck

# A process whose comm is exactly `makepkg` is the lock gate, so the second
# fetch is refused before any command starts. It appears only after the first
# fetch has finished, so the first one really runs.
cp /bin/sleep /tmp/makepkg

cat >"$driver" <<'EOF'
#!/bin/bash
script -qec "stty rows 40 cols 120; aur-prefetch tui" /dev/null
EOF
chmod 755 "$driver"

# t=0 static frame · t=1 F runs the real fetch · t=5 l opens the finished
# session's log · t=6 makepkg appears · t=7 F is refused and draws the notice
# while the log is open · t=9 one esc must clear it · t=10 q.
( sleep 6; /tmp/makepkg 300 & echo $! >/tmp/makepkg.pid; wait ) &
( sleep 1; printf 'F'; sleep 4; printf 'l'; sleep 2; printf 'F'; sleep 2; printf '\033'; sleep 1; printf 'q' ) \
    | timeout 30 runuser -l builder -c "$driver" >"$screen" 2>&1
status=$?

holder=$(cat /tmp/makepkg.pid 2>/dev/null || true)
if [[ -n $holder ]]; then kill "$holder" 2>/dev/null; fi
wait 2>/dev/null || true
rm -f /tmp/makepkg /tmp/makepkg.pid

assert_eq "$status" "0" "the TUI exited 0 instead of aborting on the invariant"

# `script` records a delta stream; strip the CSI sequences so a row can be
# asserted as text.
clean=$(sed -e 's/\x1b\[[0-9;?]*[a-zA-Z]//g' -e 's/\x1b[()][A-Za-z0-9]//g' "$screen" | tr -d '\r')

# 1. The premise really happened: a real fetch produced a real logged line, the
#    log opened on it, and the second fetch was refused by the lock gate.
assert_contains "$clean" "Synchronizing" "the opened log showed a real pacman line"
assert_contains "$clean" "makepkg is running" "the refused fetch names the live build"

# 2. One esc after the refusal must clear the notice and redraw the static
#    frame. Had the log flag outlived its session, that esc would have been
#    spent closing the log and the notice frame would stay the last one drawn.
#    Only the footer rows change between those two frames, so the assertion is
#    on the tail of the stream, which holds the last rows written. The window
#    spans more than one frame's footer so a wider redraw cannot push the key
#    line out of it.
last=${clean: -400}
assert_contains "$last" "q quit · g ledger · ? help · r refresh" \
    "one esc dismissed the notice and drew the static key line last"
assert_false "the notice frame is not the last one drawn" \
    bash -c 'printf "%s" "$1" | grep -q "F retry"' _ "$last"

# 3. Nothing was installed, no state was invented, and the only escalation is
#    the one allowed fetch argv.
assert_eq "$(usr_state)" "$before_usr" "the refused fetch left /usr unchanged"
assert_eq "$(pkg_state)" "$before_pkgs" "the refused fetch installed nothing"
assert_false "no AUR build ran" yay_built_something
assert_contains "$(cat /var/log/sim-sudo.log)" \
    "-n /usr/bin/pacman -Syuw --noconfirm" "the one escalation is the fetch argv"

finish
