#!/usr/bin/env bash
# Kill pacman in the middle of a download, with real bytes already on disk.
#
# The paced transfer (`sim-xfer`) SIGKILLs pacman after the first half of the
# package, so the download dies the way a crash or a `kill -9` leaves it: a
# `.part` holding real bytes, no final artifact, and a lock nobody holds any
# more. The scenario checks that the partial file is genuine - a byte-for-byte
# prefix of the repo artifact - and that the cache is retryable afterwards.
#
# The fault is driven from the harness (`echo kill > /etc/sim/xfer-fault`,
# disarmed by the fault itself), never by a sleep racing a signal.
source /usr/local/lib/sim/lib.sh

echo "scenario: kill pacman mid-download, keep the real .part, and retry the cache"

cache=/var/cache/pacman/pkg
package=simapp-2.0.0-1-x86_64.pkg.tar.zst
source=/srv/simrepo/$package
total=$(stat -c %s "$source")

cp /usr/local/lib/sim/fixtures/pacman-slow-xfer.conf /etc/pacman.conf
echo 5 >/etc/sim/xfer-delay
echo kill >/etc/sim/xfer-fault

# Remove the sync DB so "the lists are new" means "this fetch fetched them".
# The kill only fires on a package transfer, so the database transfer is real.
rm -f /var/lib/pacman/sync/sim.db /var/lib/pacman/sync/sim.db.part

before_usr=$(usr_state)
before_pkgs=$(pkg_state | sha256sum)
before_local=$(local_db_state | sha256sum)

log=$(mktemp)
runuser -l builder -c 'aur-prefetch fetch' >"$log" 2>&1
status=$?

# 1. The kill is an execution error, and the tool names the command that died.
assert_eq "$status" "2" "the killed download is an execution error (exit 2, D6)"
assert_false "fetch never claims success" \
    bash -c 'grep -q "fetch: ok" "$1"' _ "$log"
assert_contains "$(cat "$log")" "exited" "the failure names the command that died"

# 2. pacman really died inside the transfer: the partial artifact is there, it
#    holds exactly the first half, it is a real prefix of the repo artifact, and
#    the final artifact was never written.
part=$cache/$package.part
assert_true "pacman left the partial artifact it was writing" test -f "$part"
assert_eq "$(stat -c %s "$part" 2>/dev/null)" "$((total / 2))" \
    "the partial artifact holds exactly the first chunk"
assert_true "the partial artifact is a byte-for-byte prefix of the real package" \
    cmp -s -n "$((total / 2))" "$part" "$source"
assert_false "no final artifact was written" test -e "$cache/$package"
assert_false "no filler was written: this is a kill, not a full disk" \
    test -e "$part.filler"

# A hard kill leaves the lock. Removing it is the user's step, never ours:
# scenario 06 owns that refusal.
assert_true "the hard kill left db.lck behind" test -f /var/lib/pacman/db.lck

# 3. The lists arrived before the package transfer did, so the version that
#    killed the download is the one the system now knows about.
assert_true "the killed fetch still delivered the package lists" \
    cmp -s /var/lib/pacman/sync/sim.db /srv/simrepo/sim.db.tar.gz
assert_false "no half-written sync DB is left" test -e /var/lib/pacman/sync/sim.db.part
assert_contains "$(pacman -Qu 2>&1)" "simapp 1.0.0-1 -> 2.0.0-1" \
    "the new lists carry the pending update"

# 4. Nothing was installed, and nothing outside the cache and the lists moved.
assert_eq "$(usr_state)" "$before_usr" "the killed download wrote nothing to /usr"
assert_eq "$(pkg_state | sha256sum)" "$before_pkgs" "the killed download installed nothing"
assert_eq "$(local_db_state | sha256sum)" "$before_local" "the local package DB is untouched"

# 5. Retry. The dead pacman's lock makes the next fetch refuse once and say why;
#    the tool never removes the lock itself. After the user's own removal the
#    same cache completes, and the `.part` is replaced rather than appended to.
stale=$(mktemp)
runuser -l builder -c 'aur-prefetch fetch' >"$stale" 2>&1
assert_eq "$?" "1" "the retry refuses while the dead pacman's lock is there"
assert_contains "$(cat "$stale")" "stale lock" "the refusal names the stale lock"
assert_true "the tool never removed the lock itself" test -f /var/lib/pacman/db.lck

rm -f /var/lib/pacman/db.lck
retry=$(mktemp)
runuser -l builder -c 'aur-prefetch fetch' >"$retry" 2>&1
status=$?

assert_eq "$status" "0" "the retry on the same cache succeeds"
assert_contains "$(cat "$retry")" "fetch: ok" "the retry reports success"
assert_eq "$(cache_files)" "$package" "the cache now holds the one package"
assert_false "the retry left no partial file behind" test -e "$part"
assert_true "the retried bytes match the real repo artifact" \
    cmp -s "$cache/$package" "$source"
assert_eq "$(usr_state)" "$before_usr" "the retried fetch wrote nothing to /usr"
assert_eq "$(pkg_state | sha256sum)" "$before_pkgs" "the retried fetch installed nothing"

# 6. Invariants: each fetch escalated exactly the allowed argv, and no fetch
#    reached the AUR path at all.
assert_eq "$(grep -c . /var/log/sim-sudo.log)" "2" "each fetch escalated once"
assert_eq "$(grep -vc '^-n /usr/bin/pacman -Syuw --noconfirm$' /var/log/sim-sudo.log)" "0" \
    "both escalations are the exact fetch argv"
assert_eq "$(yay_log)" "" "no fetch reached yay"
assert_false "nothing ran an AUR build" yay_built_something

finish
