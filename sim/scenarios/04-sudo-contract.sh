#!/usr/bin/env bash
# Phase 2: the sudoers contract.
#
# builder may run exactly one command without a password. The allowed argv must
# work; any other pacman action and any other command must be refused before
# pacman runs.
source /usr/local/lib/sim/lib.sh

echo "scenario: only the exact fetch argv is allowed through sudo"

before_usr=$(usr_state)
before_pkgs=$(pkg_state)

# The allowed command downloads and must not install.
if runuser -u builder -- sudo -n /usr/bin/pacman -Syuw --noconfirm >/tmp/sim-allowed.log 2>&1; then
    pass "sudo allowed pacman -Syuw --noconfirm"
else
    fail "sudo refused the one allowed command"
fi
assert_eq "$(installed_version)" "1.0.0-1" "the allowed command installed nothing"
assert_contains "$(cache_files)" "simapp-2.0.0" "the allowed command reached the cache"
assert_eq "$(usr_state)" "$before_usr" "the allowed command left /usr unchanged"
assert_eq "$(pkg_state)" "$before_pkgs" "the allowed command changed no package"

# A different pacman action must be refused by sudo, not by pacman.
if runuser -u builder -- sudo -n /usr/bin/pacman -Syu --noconfirm >/tmp/sim-denied.log 2>&1; then
    fail "sudo allowed pacman -Syu"
else
    pass "sudo refused pacman -Syu"
fi
assert_eq "$(installed_version)" "1.0.0-1" "the refused command installed nothing"

# There must be no blanket rule.
if runuser -u builder -- sudo -n true >/dev/null 2>&1; then
    fail "builder has blanket sudo"
else
    pass "builder has no blanket sudo"
fi

finish
