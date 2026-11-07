#!/usr/bin/env bash
# Detector half 2 of 2: a real upgrade must change /usr.
#
# If this ever fails, the harness is blind and scenario 01 proves nothing.
source /usr/local/lib/sim/lib.sh

echo "scenario: pacman -Syu installs and changes /usr"

before_usr=$(usr_state)
before_pkgs=$(pkg_state)

pacman -Syu --noconfirm

assert_eq "$(installed_version)" "2.0.0-1" "installed version is now 2.0.0-1"
assert_eq "$(payload_version)" "2.0.0" "payload reports 2.0.0"
assert_ne "$(usr_state)" "$before_usr" "/usr payload digest changed"
assert_ne "$(pkg_state)" "$before_pkgs" "package database changed"

finish
