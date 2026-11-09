#!/usr/bin/env bash
# A failed fetch freezes the frontier, keeps the failed row, and opens the
# repair card. Nothing is installed and no state is written.
source /usr/local/lib/sim/lib.sh

echo "scenario: a failed fetch freezes the tree and shows the repair card"

cache=/var/cache/pacman/pkg
package=simapp-2.0.0-1-x86_64.pkg.tar.zst
state_dir=/home/builder/.local/state/aur-prefetch
screen=$(mktemp)
driver=$(mktemp)

before_usr=$(usr_state)
before_pkgs=$(pkg_state)

# The repo db still advertises the package, but its bytes are corrupt. The
# paced XferCommand writes a real `.part` in two chunks, so the TUI starts a
# running row; pacman then verifies the download and exits nonzero without
# installing anything. This is a real failure through the real command path,
# not a stubbed exit.
cp /usr/local/lib/sim/fixtures/pacman-slow-xfer.conf /etc/pacman.conf
printf 'x' >>"/srv/simrepo/$package"
rm -f "$cache"/simapp-*
rm -rf "$state_dir"
: >/var/log/sim-sudo.log

cat >"$driver" <<'EOF'
#!/bin/bash
script -qec "stty rows 40 cols 120; aur-prefetch tui" /dev/null
EOF
chmod 755 "$driver"

# One second for the static frame, F, six seconds for the paced transfer, the
# failure and the result frame, then q.
( sleep 1; printf 'F'; sleep 6; printf 'q' ) \
    | timeout 30 runuser -l builder -c "$driver" >"$screen" 2>&1
status=$?
assert_eq "$status" "0" "the TUI exited 0 after dismissing the failure"

clean=$(sed -e 's/\x1b\[[0-9;?]*[a-zA-Z]//g' -e 's/\x1b[()][A-Za-z0-9]//g' "$screen" | tr -d '\r')

# 1. The failed node stays visible and the frontier is frozen.
assert_contains "$clean" "⚠ simapp" "the failed row stays visible"
assert_contains "$clean" "frozen" "the frontier is frozen, not retried"

# 2. The repair card replaces the action line with observed facts.
assert_contains "$clean" "repair: " "the repair card names a correction"
assert_contains "$clean" "no state written" "the card states the write did not happen"
assert_contains "$clean" "no downstream blocked" "the only package has no dependents"
assert_contains "$clean" "F fetch" "the card offers another fetch"
assert_contains "$clean" "exit 1" "the real nonzero exit is on screen"

# 3. Nothing was installed and the pending upgrade is still pending.
assert_eq "$(usr_state)" "$before_usr" "the failed fetch left /usr unchanged"
assert_eq "$(pkg_state)" "$before_pkgs" "the failed fetch installed nothing"
assert_eq "$(installed_version)" "1.0.0-1" "the pending upgrade is still pending"

# 4. A failed fetch never records success and never produces the final file.
state_files=$(find "$state_dir" -type f -printf '%P\n' 2>/dev/null | sort)
assert_eq "$state_files" "" "a failed fetch wrote no state"
assert_false "no final artifact was installed into the cache" test -e "$cache/$package"

# 5. The only escalation is the one sudoers allows, and no AUR build ran.
assert_eq "$(grep -c . /var/log/sim-sudo.log 2>/dev/null || true)" "1" \
    "the TUI escalated exactly once"
assert_contains "$(cat /var/log/sim-sudo.log)" \
    "-n /usr/bin/pacman -Syuw --noconfirm" "the one escalation is the fetch argv"
assert_false "the failed fetch never runs an AUR build" yay_built_something

finish
