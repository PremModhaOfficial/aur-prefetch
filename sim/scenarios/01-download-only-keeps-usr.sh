#!/usr/bin/env bash
# Detector half 1 of 2: a download-only upgrade must leave /usr untouched.
#
# This asserts the harness can see the difference between `pacman -Syuw`
# (download) and a real install. Its pair, 02, proves the detector is not blind.
source /usr/local/lib/sim/lib.sh

echo "scenario: pacman -Syuw downloads but does not install"

before_usr=$(usr_state)
before_pkgs=$(pkg_state)
before_cache=$(cache_files)

pacman -Syuw --noconfirm

assert_eq "$(installed_version)" "1.0.0-1" "installed version unchanged"
assert_eq "$(payload_version)" "1.0.0" "payload still reports 1.0.0"
assert_contains "$(cache_files)" "simapp-2.0.0" "new package downloaded to cache"
assert_ne "$(cache_files)" "$before_cache" "cache changed"
assert_eq "$(usr_state)" "$before_usr" "/usr payload digest unchanged"
assert_eq "$(pkg_state)" "$before_pkgs" "no package was installed, removed, or upgraded"
assert_eq "$(yay_log)" "" "yay was not invoked"

finish
