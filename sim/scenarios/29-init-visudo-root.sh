#!/usr/bin/env bash
# init validates its user-owned temp rule with `sudo visudo -cf`. Root visudo
# accepts that file; this proves the environment assumption scenario 20 fakes.
source /usr/local/lib/sim/lib.sh

echo "scenario: root visudo accepts a user-owned temp rule"

rule=$(runuser -u builder -- mktemp /tmp/init-visudo.XXXXXX)
runuser -u builder -- sh -c \
    'printf "%s\n" "builder ALL=(root) NOPASSWD: /usr/bin/pacman -Syuw --noconfirm" > "$1"' \
    sh "$rule"

assert_eq "$(stat -c %U "$rule")" "builder" "the temp rule is user-owned"
assert_eq "$(stat -c %a "$rule")" "600" "the temp rule is mode 0600"

log=$(mktemp)
visudo -cf "$rule" >"$log" 2>&1
status=$?

assert_eq "$status" "0" "root visudo accepted the user-owned temp rule"
assert_contains "$(cat "$log")" "parsed OK" "visudo parsed the rule"
rm -f "$rule"

finish
