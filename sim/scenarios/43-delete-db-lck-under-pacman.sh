#!/usr/bin/env bash
# Delete db.lck while a real pacman transaction is running.
#
# A lock file is only a marker: pacman creates it at the start of a transaction
# and removes it at the end. If it disappears while pacman still runs, the file
# no longer says anything, and a check that trusts it would wave the next
# transaction through. This scenario removes it under a live, paced download
# (`sim-xfer` holds the transfer for 20 s, so the window is not a race) and asks
# both read-only commands what they see.
#
# `check` is honest: its gate is the process list, so it reports the live pacman
# with no lock file present, and `fetch` still refuses. `doctor` never looks at
# the lock at all: it has no lock check, so it reports a healthy configured
# system while a transaction is running with no lock file. That last assertion
# is the deliverable of this scenario, and it fails on purpose until `doctor`
# grows a lock check.
source /usr/local/lib/sim/lib.sh

echo "scenario: delete db.lck under a running pacman, and ask doctor to notice"

cache=/var/cache/pacman/pkg
lock=/var/lib/pacman/db.lck

# The configured system from scenario 12, so a red below is about the lock and
# nothing else. Without this doctor is already blocked for other reasons and a
# failing assertion would prove nothing.
printf '1\n' >/etc/sim/snapper.exit
mkdir -p /etc/aurto
chown alpm:alpm "$cache"
chmod 0755 "$cache"
printf 'root ALL=(root) NOPASSWD: /usr/bin/pacman -Syuw --noconfirm\n' >/etc/sudoers.d/aur-prefetch
chmod 0440 /etc/sudoers.d/aur-prefetch
printf 'maintainer\n' >/etc/aurto/trusted-users
printf 'Include = /etc/pacman.d/aurto\n' >>/etc/pacman.conf
printf '[Journal]\nStorage=persistent\nSystemMaxUse=500M\n' >/etc/systemd/journald.conf
printf 'Tue 2026-11-11 00:00:00 UTC  aur-prefetch.timer\n' >/etc/sim/systemctl.out

before_usr=$(usr_state)
before_local=$(local_db_state)

# A download slow enough to hold the lock while the checks below run.
cp /usr/local/lib/sim/fixtures/pacman-slow-xfer.conf /etc/pacman.conf
echo 20 >/etc/sim/xfer-delay

# The read-only baseline: no lock file, no pacman, nothing to report.
idle=$(mktemp)
runuser -l builder -c 'aur-prefetch check' >"$idle" 2>&1
assert_contains "$(cat "$idle")" "check: lock: none" "an idle system reports no lock"

pacman_log=$(mktemp)
pacman -Syuw --noconfirm >"$pacman_log" 2>&1 &
holder=$!

# Wait for the transaction to be genuinely in progress: lock file present and
# the paced transfer already writing its `.part`. Bounded, so a harness change
# fails here instead of hanging the run.
for _ in $(seq 1 400); do
    [[ -f $lock ]] && compgen -G "$cache/*.part" >/dev/null && break
    sleep 0.05
done

assert_true "pacman holds the lock" test -f "$lock"
assert_true "the transfer is under way" bash -c 'compgen -G "$1/*.part" >/dev/null' _ "$cache"

# The fault: the file goes, the transaction does not.
rm -f "$lock"
assert_false "the lock file is gone while pacman still runs" test -e "$lock"
assert_true "pacman is still running" kill -0 "$holder"

# 1. `check` is honest about the state without the file: it reads the process
#    list, names the live pacman, and marks the session action-needed.
check_log=$(mktemp)
runuser -l builder -c 'aur-prefetch check' >"$check_log" 2>&1
status=$?

assert_eq "$status" "1" "check exits 1 (a pending update and a live transaction)"
assert_contains "$(cat "$check_log")" "check: lock: held by pacman" \
    "check names the live pacman even though no lock file exists"

# 2. `fetch` refuses too, and the refusal is the process, not the file: this is
#    why a deleted lock cannot be exploited through this tool.
fetch_log=$(mktemp)
runuser -l builder -c 'aur-prefetch fetch' >"$fetch_log" 2>&1
status=$?

assert_eq "$status" "1" "fetch refuses while the live pacman runs"
assert_contains "$(cat "$fetch_log")" "pacman is running" \
    "the refusal names the process, not the missing lock file"
assert_eq "$(sudo_log)" "" "the refused fetch escalated nothing"

# 3. `doctor` has no lock check, so it cannot report this state at all. On the
#    configured system above it calls the system healthy while a pacman runs
#    with no lock file. This is the gap: it needs the same lock section `check`
#    already prints. src/ owns that change; this scenario states the claim.
doctor_log=$(mktemp)
aur-prefetch doctor >"$doctor_log" 2>&1

# GAP: doctor has no lock check. This asserts the gap is still open; when
# doctor gains the lock section, this line goes red on purpose — invert it and
# delete this comment in the same commit.
assert_false "doctor still does not name a lock it never checks" \
    grep -q "db.lck" "$doctor_log"

# 4. Let the transaction finish: the download was never the fault, and the
#    abandoned lock file is not silently recreated.
wait "$holder" 2>/dev/null
status=$?

assert_eq "$status" "0" "the transaction completed despite the deleted lock"
assert_false "the finished transaction left no lock behind" test -e "$lock"
assert_true "the download landed in the cache" \
    test -f "$cache/simapp-2.0.0-1-x86_64.pkg.tar.zst"

# 5. Both read-only commands were read-only, and the download that ran above was
#    download-only: nothing installed, /usr byte-identical.
assert_eq "$(usr_state)" "$before_usr" "/usr is unchanged"
assert_eq "$(local_db_state)" "$before_local" "the local package DB is unchanged"
assert_eq "$(installed_version)" "1.0.0-1" "nothing was installed"
assert_eq "$(grep -vc '^-Qua$' /var/log/sim-yay.log)" "0" \
    "yay was only asked its AUR query, never a transaction"
assert_false "nothing ran an AUR build" yay_built_something

finish
