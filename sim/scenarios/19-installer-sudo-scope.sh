#!/usr/bin/env bash
# PR 16 finding: the sim-only installer rule must not widen sudo beyond the
# exact argv the fake yay needs.
#
# `install` runs yay as the user, and yay's one escalation is
# `pacman -Syu --noconfirm`. Any other command or argument list must be refused
# by sudo, not silently allowed by the harness. Scenario 14 separately asserts
# that this exact argv is the only escalation install performs.
source /usr/local/lib/sim/lib.sh

echo "scenario: installer may escalate only yay's exact upgrade argv"

rule=$(grep '^installer ' /etc/sudoers.d/sim)
assert_eq "$rule" \
    "installer ALL=(root) NOPASSWD: /usr/bin/pacman -Syu --noconfirm" \
    "the installer rule names the exact argv and no wildcard"

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
deny "installer cannot download with fetch's argv" /usr/bin/pacman -Syuw --noconfirm
deny "installer cannot run -Syu without --noconfirm" /usr/bin/pacman -Syu
deny "installer cannot add arguments to the allowed upgrade" \
    /usr/bin/pacman -Syu --noconfirm --needed
deny "installer cannot run another pacman action" /usr/bin/pacman -Sy

finish
