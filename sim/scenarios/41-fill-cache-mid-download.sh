#!/usr/bin/env bash
# sim-run-args: --tmpfs /var/cache/pacman/pkg:size=2m
#
# The CacheDir fills during the download. The cache is a 2 MiB tmpfs (the flag
# above) and the transfer hook writes filler into it until the kernel refuses
# more, so pacman hits a real ENOSPC halfway through the package.
#
# The free-space floor is lowered on purpose: with the default 2 GiB floor the
# gate in scenario 07 would refuse before pacman ran, and this scenario would
# pass while proving nothing about a cache that fills mid-download.
source /usr/local/lib/sim/lib.sh

echo "scenario: a CacheDir that fills mid-download fails without touching /usr"

cache=/var/cache/pacman/pkg
package=simapp-2.0.0-1-x86_64.pkg.tar.zst
floor=65536

avail() { df -B1 --output=avail "$cache" | tail -1 | tr -d ' '; }

cp /usr/local/lib/sim/fixtures/pacman-slow-xfer.conf /etc/pacman.conf
echo fill >/etc/sim/xfer-fault

assert_eq "$(stat -f -c %T "$cache")" "tmpfs" "the CacheDir is a tmpfs"
assert_true "the CacheDir starts with room for the package and the floor" \
    test "$(avail)" -gt "$floor"

before_usr=$(usr_state)
before_pkgs=$(pkg_state)
before_local=$(local_db_state)

log=$(mktemp)
runuser -l builder -c "AUR_PREFETCH_MIN_FREE_BYTES=$floor aur-prefetch fetch" >"$log" 2>&1
status=$?

# 1. The failure is an execution error, not a green run and not a silent skip.
assert_eq "$status" "2" "a failed download is an execution error (exit 2, D6)"
assert_false "fetch never claims success" \
    bash -c 'grep -q "fetch: ok" "$1"' _ "$log"
assert_contains "$(cat "$log")" "failed to retrieve some files" \
    "pacman reports the transfer it could not finish"
assert_contains "$(cat "$log")" "error invoking external downloader" \
    "the failure is the downloader dying, not the transaction itself"

# 2. The CacheDir itself was full when the package transfer failed, and the
#    download really had started: the partial artifact and the filler are there
#    and no final artifact is.
assert_true "the CacheDir is full ($(avail) bytes free)" test "$(avail)" -lt 4096
assert_eq "$(cache_files)" "$(printf '%s\n%s' "$package.part" "$package.part.filler")" \
    "the cache holds only the aborted transfer and the filler"
assert_false "no final artifact was written" test -e "$cache/$package"

# 3. A pacman that exits on its own removes its lock; only a kill leaves one.
assert_false "the ENOSPC failure left no lock" test -f /var/lib/pacman/db.lck

# 4. Nothing outside the cache and the sync DB moved. The lists arrived before
#    the package did.
assert_true "the refreshed lists are the repo's file" \
    cmp -s /var/lib/pacman/sync/sim.db /srv/simrepo/sim.db.tar.gz
assert_eq "$(usr_state)" "$before_usr" "the failed download wrote nothing to /usr"
assert_eq "$(pkg_state)" "$before_pkgs" "the failed download installed nothing"
assert_eq "$(local_db_state)" "$before_local" "the local package DB is untouched"

# 5. Free the space by hand, as the user must, and the same fetch completes: the
#    failure was a full disk, not a broken cache.
rm -f "$cache"/*.part "$cache"/*.filler
retry=$(mktemp)
runuser -l builder -c "AUR_PREFETCH_MIN_FREE_BYTES=$floor aur-prefetch fetch" >"$retry" 2>&1
status=$?

assert_eq "$status" "0" "the retry after freeing space succeeds"
assert_contains "$(cat "$retry")" "fetch: ok" "the retry reports success"
assert_eq "$(cache_files)" "$package" "the cache holds exactly the fetched package"
assert_true "the fetched bytes match the real repo artifact" \
    cmp -s "$cache/$package" "/srv/simrepo/$package"

# 6. Invariants: one escalation per fetch, the same allowed argv each time, no
#    AUR helper, no build, and /usr still untouched.
assert_eq "$(grep -c . /var/log/sim-sudo.log)" "2" "each fetch escalated once"
assert_eq "$(grep -vc '^-n /usr/bin/pacman -Syuw --noconfirm$' /var/log/sim-sudo.log)" "0" \
    "both escalations are the exact fetch argv"
assert_eq "$(yay_log)" "" "no fetch reached yay"
assert_false "nothing ran an AUR build" yay_built_something
assert_eq "$(usr_state)" "$before_usr" "the retried fetch wrote nothing to /usr"
assert_eq "$(pkg_state)" "$before_pkgs" "the retried fetch installed nothing"

finish
