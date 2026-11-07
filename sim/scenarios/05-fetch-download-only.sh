#!/usr/bin/env bash
# fetch, as the non-root user: download only, record state, never install.
#
# Exiting 0 also proves fetch used the one argv sudo allows. Any other argv
# would be refused by sudo and fetch would report exit 2.
source /usr/local/lib/sim/lib.sh

echo "scenario: fetch downloads as builder and records state"

before_usr=$(usr_state)
before_pkgs=$(pkg_state)

runuser -l builder -c 'aur-prefetch fetch' >/tmp/sim-fetch.log 2>&1
status=$?

assert_eq "$status" "0" "fetch exited 0"
assert_contains "$(cat /tmp/sim-fetch.log)" "running sudo -n /usr/bin/pacman -Syuw --noconfirm" "fetch used the allowed argv"
assert_contains "$(cache_files)" "simapp-2.0.0" "fetch downloaded the pending package"
assert_eq "$(cache_files | wc -l | tr -d ' ')" "1" "cache holds exactly the pending package"
assert_eq "$(installed_version)" "1.0.0-1" "fetch did not install the download"
assert_eq "$(usr_state)" "$before_usr" "fetch left /usr unchanged"
assert_eq "$(pkg_state)" "$before_pkgs" "fetch changed no package"
assert_contains "$(cat /home/builder/.local/state/aur-prefetch/state.json 2>/dev/null)" '"last_fetch_pending":1' "fetch recorded one pending package"
assert_false "fetch never runs an AUR build" yay_built_something

finish
