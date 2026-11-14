#!/usr/bin/env bash
# The live fetch is the real fetch: F runs the allowed argv and the tree reads
# real `.part` bytes from the pacman cache.
#
# The sim pacman config adds one transfer hook that writes the package `.part`
# in two chunks with a pause. pacman still chooses the argv, verifies the
# package, and renames the `.part`; the hook creates the path and paces the
# local copy so the 100 ms poll can see it.
source /usr/local/lib/sim/lib.sh

echo "scenario: live fetch streams real bytes from the real cache"

cache=/var/cache/pacman/pkg
package=simapp-2.0.0-1-x86_64.pkg.tar.zst
state_dir=/home/builder/.local/state/aur-prefetch
screen=$(mktemp)
driver=$(mktemp)

before_usr=$(usr_state)
before_pkgs=$(pkg_state)

cp /usr/local/lib/sim/fixtures/pacman-slow-xfer.conf /etc/pacman.conf
rm -f "$cache"/simapp-*
rm -rf "$state_dir"
: >/var/log/sim-sudo.log

cat >"$driver" <<'EOF'
#!/bin/bash
script -qec "stty rows 40 cols 120; aur-prefetch tui" /dev/null
EOF
chmod 755 "$driver"

# One second for the static frame, F, one second of the running fetch, i (which
# must be refused while the fetch owns the lock), three seconds for the paced
# transfer and the result frame, l for the captured log, one second to prove a
# real line, esc to close the log (not detach), then q. pacman's stdout is
# block-buffered when piped, so its lines are complete only after it exits.
( sleep 1; printf 'F'; sleep 1; printf 'i'; sleep 3; printf 'l'; sleep 1; printf '\033'; sleep 1; printf 'q' ) \
    | timeout 30 runuser -l builder -c "$driver" >"$screen" 2>&1
status=$?
assert_eq "$status" "0" "the TUI exited 0 after the fetch result"

# `script` records a delta stream. Strip the CSI sequences so a row written
# across several cells can be asserted as one string.
clean=$(sed -e 's/\x1b\[[0-9;?]*[a-zA-Z]//g' -e 's/\x1b[()][A-Za-z0-9]//g' "$screen" | tr -d '\r')

# 1. A running row with real `.part` bytes and a bar. The package size depends
#    on the builddate the repo was produced with, so derive it from the real
#    repo file. The paced transfer writes the first half, so both sizes are
#    observed and the bar is drawn. The live footer names the exact command.
total=$(stat -c %s "/srv/simrepo/$package")
half=$((total / 2))
pct=$(awk -v h="$half" -v t="$total" 'BEGIN { printf "%.1f%%", h * 100 / t }')
assert_contains "$clean" "downloading" "the row is downloading"
assert_contains "$clean" "$total B" "the row shows the real total"
assert_contains "$clean" "$half B" "the row shows the observed .part size"
# The fixture renames the first chunk into place, so the half state is the
# first thing any poll of the `.part` can observe and this percentage is drawn
# whole. `script` records a delta stream, and a percentage drawn once and then
# diffed (0.0%, then the digits changing to 49.9%) is never a whole string
# again, so the first full draw is the only assertable one.
assert_contains "$clean" "$pct" "the percentage needs both observed sizes"
assert_contains "$clean" "■" "the bar is drawn"
assert_contains "$clean" "fetch: running sudo -n /usr/bin/pacman -Syuw --noconfirm" \
    "the status names the exact argv"
assert_contains "$clean" "esc detach" "a running fetch offers detach only"

# 1b. The gates that ran are on screen with their real inputs, and the captured
#     log shows pacman's own line rather than a rendered guess. The delta
#     stream skips cells whose content did not change, so a space between two
#     words can be absent; a single word is durable.
assert_contains "$clean" "PRE-FLIGHT" "the gate checklist is visible"
assert_contains "$clean" "lock" "the lock gate ran"
assert_contains "$clean" "1 pending" "the lists gate carries the real count"
assert_contains "$clean" "Synchronizing" "l opens the captured log and shows a real pacman line"

# The install gate must stay shut while the fetch is in flight: one mutation at
# a time, and the lock may not exist for the first moments.
assert_false "the install gate is refused during a fetch" \
    bash -c 'printf "%s" "$1" | grep -q "space attest"' _ "$clean"
assert_contains "$clean" "blocked" "the refused install key names the reason"

# 2. The row becomes done from the final artifact.
assert_contains "$clean" "[√]" "the final artifact finishes the row"
assert_contains "$clean" "done" "the row says done"
assert_contains "$clean" "exit 0" "the result footer carries the real exit"
# A zero exit is not a failure and not a freeze. A download the fetch cannot
# prove (its `.part` vanished, no final artifact) is finished as unproven, a
# terminal row, so a finished run never draws a frozen running row.
assert_false "a zero-exit result never reads as frozen" \
    bash -c 'printf "%s" "$1" | grep -q "frozen"' _ "$clean"
# The key line is drawn across cells, so the space between the words can be
# absent from the delta stream. "F fetch" is the durable fragment.
assert_contains "$clean" "F fetch" "the result offers another fetch"

# 3. /usr and the installed package set are unchanged.
assert_eq "$(usr_state)" "$before_usr" "the live fetch left /usr unchanged"
assert_eq "$(pkg_state)" "$before_pkgs" "the live fetch installed nothing"
assert_eq "$(installed_version)" "1.0.0-1" "the pending upgrade is still pending"

# 4. The cache changed only through the fetch, and it holds exactly the final
#    artifact. No `.part` is left and the bytes match the repo.
assert_eq "$(cache_files)" "$package" "the cache holds only the fetched package"
assert_false "no partial file is left behind" test -e "$cache/$package.part"
assert_true "the fetched bytes match the local repo" cmp -s "$cache/$package" "/srv/simrepo/$package"

# 5. The only state write is fetch's existing file, written after success.
state_files=$(find "$state_dir" -type f -printf '%P\n' 2>/dev/null | sort)
assert_eq "$state_files" "state.json" "the TUI wrote only fetch's state file"
assert_contains "$(cat "$state_dir/state.json" 2>/dev/null)" '"last_fetch_pending":1' \
    "fetch recorded the one pending package"
assert_eq "$(find /home/builder/.local/state -type f -printf '%P\n' 2>/dev/null | sort)" \
    "aur-prefetch/state.json" "no other state file appeared"

# 6. The only escalation is the one sudoers allows. No install, no AUR build,
#    and no second pacman argv.
assert_eq "$(grep -c . /var/log/sim-sudo.log 2>/dev/null || true)" "1" \
    "the TUI escalated exactly once"
assert_contains "$(cat /var/log/sim-sudo.log)" \
    "-n /usr/bin/pacman -Syuw --noconfirm" "the one escalation is the fetch argv"
assert_false "the live fetch never runs an AUR build" yay_built_something

finish
