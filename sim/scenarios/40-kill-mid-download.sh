#!/usr/bin/env bash
# Kill pacman mid-download. The paced transfer (`sim-xfer`) SIGKILLs pacman
# after the first chunk of the package, so the download dies with real bytes on
# disk and a stale lock. What the user is left with must be retryable, and the
# package lists must already be the refreshed ones.
#
# The fault is driven from the harness: `echo kill > /etc/sim/xfer-fault`
# before the fetch. Nothing in this scenario sleeps to fake timing.
source /usr/local/lib/sim/lib.sh

echo "scenario: kill pacman mid-download, then retry the cache"

cache=/var/cache/pacman/pkg
package=simapp-2.0.0-1-x86_64.pkg.tar.zst
total=$(stat -c %s "/srv/simrepo/$package")

cp /usr/local/lib/sim/fixtures/pacman-slow-xfer.conf /etc/pacman.conf
echo 5 >/etc/sim/xfer-delay
echo kill >/etc/sim/xfer-fault

# Remove the sync DB so "the lists are new" means "this fetch fetched them".
# The kill only fires on a package transfer, so the database transfer is real.
rm -f /var/lib/pacman/sync/sim.db /var/lib/pacman/sync/sim.db.part

before_usr=$(usr_state)
before_pkgs=$(pkg_state)
before_local=$(local_db_state)

log=$(mktemp)
runuser -l builder -c 'aur-prefetch fetch' >"$log" 2>&1
status=$?

assert_eq "$status" "2" "the killed download is an execution error (exit 2, D6)"
assert_contains "$(cat "$log")" "exited" "the failure names the command that died"

# 1. pacman really died in the middle of the transfer.
part="$cache/$package.part"
assert_true "pacman left the partial artifact it was writing" test -f "$part"
assert_eq "$(stat -c %s "$part" 2>/dev/null)" "$((total / 2))" \
    "the partial artifact holds exactly the first chunk"
assert_false "no final artifact was written" test -e "$cache/$package"

# A hard kill leaves the lock. Removing it is the user's step, never ours:
# scenario 06 owns that refusal.
assert_true "the hard kill left db.lck behind" test -f /var/lib/pacman/db.lck

# 2. The lists were refreshed before the kill: the sync DB is the repo's file,
#    no `.part` of it survives, and pacman reads a pending update from it.
assert_true "the killed fetch still delivered the package lists" \
    cmp -s /var/lib/pacman/sync/sim.db /srv/simrepo/sim.db.tar.gz
assert_false "no half-written sync DB is left" test -e /var/lib/pacman/sync/sim.db.part
assert_contains "$(pacman -Qu 2>&1)" "simapp 1.0.0-1 -> 2.0.0-1" \
    "the new lists carry the pending update"

# 3. Nothing was installed and nothing outside the cache and the sync DB moved.
assert_eq "$(usr_state)" "$before_usr" "the killed download wrote nothing to /usr"
assert_eq "$(pkg_state)" "$before_pkgs" "the killed download installed nothing"
assert_eq "$(local_db_state)" "$before_local" "the local package DB is untouched"

# 4. Retry. A lock left by a hard kill is stale, so the tool refuses once and
#    says why; after the user's own removal the same cache completes.
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
assert_false "the retry leaves no partial file" test -e "$part"
assert_true "the retried bytes match the real repo artifact" \
    cmp -s "$cache/$package" "/srv/simrepo/$package"

# 5. Invariants: the two fetches escalated only the allowed argv, and no AUR
#    helper or build ran at all.
assert_eq "$(grep -c . /var/log/sim-sudo.log)" "2" "each fetch escalated once"
assert_eq "$(grep -vc '^-n /usr/bin/pacman -Syuw --noconfirm$' /var/log/sim-sudo.log)" "0" \
    "both escalations are the exact fetch argv"
assert_eq "$(yay_log)" "" "no fetches reached yay"
assert_false "nothing ran an AUR build" yay_built_something
assert_eq "$(usr_state)" "$before_usr" "the retried fetch wrote nothing to /usr"
assert_eq "$(pkg_state)" "$before_pkgs" "the retried fetch installed nothing"

finish
