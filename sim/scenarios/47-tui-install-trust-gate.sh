#!/usr/bin/env bash
# The gate and the action must agree on the trust file. A missing
# /etc/aurto/trusted-users means aurto trusts everyone, and the CLI refuses
# that state without --yes. The TUI has no way to supply --yes, so the gate
# blocks: enter never arms, and yay -Syuu never runs.
source /usr/local/lib/sim/lib.sh

echo "scenario: a missing trust file blocks the install gate and the action"

state_dir=/home/builder/.local/state/aur-prefetch
screen=$(mktemp)
driver=$(mktemp)

before_usr=$(usr_state)
before_pkgs=$(pkg_state)

rm -rf "$state_dir"
rm -f /etc/aurto/trusted-users
: >/var/log/sim-sudo.log
: >/var/log/sim-yay.log
: >/var/log/sim-yay-build.log

cat >"$driver" <<'EOF'
#!/bin/bash
script -qec "stty rows 40 cols 120; aur-prefetch tui" /dev/null
EOF
chmod 755 "$driver"

# One second for the static frame, i for the gate, one second to review, space
# to attest the news, one second, enter on the blocked gate, then q.
( sleep 1; printf 'i'; sleep 1; printf ' '; sleep 1; printf '\r'; sleep 1; printf 'q' ) \
    | timeout 30 runuser -l builder -c "$driver" >"$screen" 2>&1
status=$?
assert_eq "$status" "0" "the TUI exited 0"

clean=$(sed -e 's/\x1b\[[0-9;?]*[a-zA-Z]//g' -e 's/\x1b[()][A-Za-z0-9]//g' "$screen" | tr -d '\r')

# 1. The gate shows the real trust state and says it blocks.
assert_contains "$clean" "PRE-FLIGHT" "the gate has a checklist"
assert_contains "$clean" "trusts everyone" "the trust gate names the state"
assert_contains "$clean" "a gate blocks install" "the attested gate still blocks"
assert_false "enter never arms a blocked gate" \
    grep -qF "enter install" <<<"$clean"

# 2. Nothing ran or changed.
assert_eq "$(usr_state)" "$before_usr" "the gate left /usr unchanged"
assert_eq "$(pkg_state)" "$before_pkgs" "the gate installed nothing"
assert_eq "$(find "$state_dir" -type f -printf '%P\n' 2>/dev/null | sort)" "" \
    "the gate wrote no state"
assert_eq "$(grep -c . /var/log/sim-sudo.log 2>/dev/null || true)" "0" \
    "the gate escalated nothing"
assert_false "the gate never runs yay -Syuu" grep -q -- -Syuu /var/log/sim-yay.log
assert_false "the gate never reaches the AUR build path" yay_built_something

# 3. The CLI refuses the same state on a real terminal, with the same reason
#    and the contract's exit code.
cli=$(timeout 30 runuser -l builder -c \
    'script -qec "aur-prefetch install" /dev/null' 2>&1)
cli_rc=$?
assert_eq "$cli_rc" "1" "the CLI exits 1 on the blocked state"
assert_contains "$cli" "trusted-users is missing" "the CLI refuses a missing trust file"
assert_contains "$cli" "use --yes" "the CLI names --yes as the way out"
assert_eq "$(installed_version)" "1.0.0-1" "the pending upgrade is still pending"
assert_false "the CLI never runs yay -Syuu" grep -q -- -Syuu /var/log/sim-yay.log

finish
