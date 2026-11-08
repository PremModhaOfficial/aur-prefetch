#!/usr/bin/env bash
# PR 16 finding: the sim must not widen privilege beyond the product contract.
#
# `install` runs yay as the user. The simulation gives yay exactly one allowed
# command, `/usr/local/bin/sim-upgrade`, which takes no arguments and always
# runs `pacman -Syu --noconfirm`. No direct pacman argv is allowed through sudo.
source /usr/local/lib/sim/lib.sh

echo "scenario: installer may escalate only the sim-upgrade helper"

rule=$(grep '^installer ' /etc/sudoers.d/sim)
assert_eq "$rule" \
    "installer ALL=(root) NOPASSWD: /usr/local/bin/sim-upgrade" \
    "the installer rule names only the helper, with no wildcard"

helper=/usr/local/bin/sim-upgrade
assert_true "the scoped helper exists" test -x "$helper"
assert_eq "$(stat -c '%U %a' "$helper")" "root 755" "the helper is a root-owned script"

log=$(mktemp)

deny() {
    local label=$1
    shift
    if runuser -u installer -- sudo -n "$@" >"$log" 2>&1; then
        fail "$label"
    else
        pass "$label"
    fi
    assert_contains "$(cat "$log")" "sudo:" "$label: refused by sudo, not pacman"
    assert_false "$label: pacman never started" grep -q "Synchronizing" "$log"
}

deny "installer has no blanket sudo" true
deny "installer cannot run a direct pacman upgrade" /usr/bin/pacman -Syu --noconfirm
deny "installer cannot run fetch's pacman argv" /usr/bin/pacman -Syuw --noconfirm
deny "installer cannot run another pacman action" /usr/bin/pacman -Sy

# The helper itself rejects arguments, so the allowed rule cannot be widened.
if runuser -u installer -- sudo -n /usr/local/bin/sim-upgrade extra >"$log" 2>&1; then
    fail "the helper accepted arguments"
else
    pass "the helper refuses arguments"
fi
assert_contains "$(cat "$log")" "takes no arguments" "the helper explains the refusal"

finish
