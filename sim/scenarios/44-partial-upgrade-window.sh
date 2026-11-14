#!/usr/bin/env bash
# Install one package between fetch and install, and ask `check` whether it can
# see what that did.
#
# This is the window spec.md calls the number one way to break the system: the
# lists are refreshed and the artifacts are prefetched, then somebody installs a
# single package from the new lists. The system is now mixed - one package from
# the new lists, the rest still on the old ones - and the prefetched artifact
# sitting in the cache is exactly the upgrade that would finish the job.
#
# `report::Drift` is filled by `check` from this log: an ALPM install after the
# last download-only fetch, with updates still pending, is the partial upgrade
# `spec.md` calls the number one way to break the system.
source /usr/local/lib/sim/lib.sh

echo "scenario: a single install between fetch and install is the partial upgrade check reports"

pkgname=bigpkg
state=/home/builder/.local/state/aur-prefetch/state.json
cache=/var/cache/pacman/pkg

# Two pending packages: without the second one, installing "one package" would
# be the whole upgrade and prove nothing.
make-pkg.sh "$pkgname" 1.0.0 4096 >/dev/null
pacman -Syy --noconfirm >/dev/null
pacman -S --noconfirm "$pkgname" >/dev/null
make-pkg.sh "$pkgname" 2.0.0 8192 >/dev/null
pacman -Syy --noconfirm >/dev/null

assert_eq "$(pacman -Qu | wc -l)" "2" "two packages are pending after the refresh"

before_pkgs=$(pkg_state | sha256sum)

# Step 1: the prefetch. Lists refreshed, both artifacts downloaded, state
# written - the window is now open and the tool knows when it opened.
fetch_log=$(mktemp)
runuser -l builder -c 'aur-prefetch fetch' >"$fetch_log" 2>&1
assert_eq "$?" "0" "fetch prefetched the pending set"
assert_contains "$(cat "$fetch_log")" "fetch: ok" "fetch reported success"
assert_true "both artifacts are in the cache" \
    test -f "$cache/simapp-2.0.0-1-x86_64.pkg.tar.zst" -a \
    -f "$cache/$pkgname-2.0.0-1-x86_64.pkg.tar.zst"
assert_true "the fetch was recorded in the state file" test -f "$state"

# Step 2: the fault. One package is installed from the new lists, by hand, in
# the window. No tool can prevent this; only a report can see it happened.
pacman -S --noconfirm "$pkgname" >/dev/null 2>&1
assert_eq "$?" "0" "the single install succeeded"
assert_eq "$(pacman -Q "$pkgname")" "$pkgname 2.0.0-1" "one package moved to the new lists"
assert_eq "$(installed_version)" "1.0.0-1" "the rest of the system did not"
assert_eq "$(pacman -Qu | wc -l)" "1" "one package is still pending"
assert_ne "$(pkg_state | sha256sum)" "$before_pkgs" \
    "the installed set changed while the rest of /usr did not: half upgraded"

# The evidence a detector would read: an ALPM upgrade in the log, after the
# fetch recorded in the state file.
assert_contains "$(cat /var/log/pacman.log)" "upgraded $pkgname (1.0.0-1 -> 2.0.0-1)" \
    "pacman's log records the install that opened the window"
assert_true "the prefetched artifact for the still-pending package is still cached" \
    test -f "$cache/simapp-2.0.0-1-x86_64.pkg.tar.zst"

# Step 3: what the read-only report says about that. It names the pending
# update, and it names the state the window put the system in.
check_log=$(mktemp)
before_check_local=$(local_db_state | sha256sum)
before_check_usr=$(usr_state)
runuser -l builder -c 'aur-prefetch check' >"$check_log" 2>&1
status=$?

assert_eq "$status" "1" "check exits 1 (an update is still pending)"
assert_contains "$(cat "$check_log")" "simapp" "check names the still-pending package"

# The deliverable: the evidence in the log is read, and the mixed system is
# named rather than resolved silently.
assert_true "check reports the partial upgrade it was handed" \
    grep -qi "partial upgrade" "$check_log"
assert_contains "$(cat "$check_log")" "an upgrade ran after the last fetch" \
    "the report names the evidence it read"

# 4. Read-only, and nothing else moved: the mixed /usr is the install's doing,
#    not the checks'.
assert_eq "$(local_db_state | sha256sum)" "$before_check_local" \
    "no check wrote the package DB"
assert_eq "$(usr_state)" "$before_check_usr" "check left /usr alone"
assert_eq "$(installed_version)" "1.0.0-1" "check installed nothing"
assert_eq "$(yay_log | grep -vc '^-Qua$')" "0" "no command reached yay for a transaction"
assert_false "nothing ran an AUR build" yay_built_something

finish
