#!/usr/bin/env bash
# The TUI is read-only, --render-once works in a pipe, and interactive mode
# owns and restores its terminal.
source /usr/local/lib/sim/lib.sh

echo "scenario: tui is read-only and restores the terminal"

frame=$(mktemp)
pipe_err=$(mktemp)

before_usr=$(usr_state)
before_pkgs=$(pkg_state)
before_cache=$(cache_files)

runuser -l builder -c 'aur-prefetch tui --render-once' >"$frame" 2>&1
status=$?

assert_eq "$status" "1" "render-once exits 1 for a pending snapshot"
assert_contains "$(cat "$frame")" "simapp" "the frame names the pending package"
assert_contains "$(cat "$frame")" "install" "the frame has an install group"
assert_eq "$(usr_state)" "$before_usr" "tui left /usr unchanged"
assert_eq "$(pkg_state)" "$before_pkgs" "tui installed nothing"
assert_eq "$(cache_files)" "$before_cache" "tui left the cache unchanged"
assert_false "tui wrote no state" test -e /home/builder/.local/state/aur-prefetch/state.json

runuser -l builder -c 'aur-prefetch tui </dev/null' >"$pipe_err" 2>&1
status=$?

assert_eq "$status" "2" "interactive tui rejects a pipe"
assert_contains "$(cat "$pipe_err")" "use --render-once" "the error points at render-once"

if ! command -v script >/dev/null 2>&1; then
    fail "script is missing; the pty checks did not run"
else
    (sleep 1; printf 'q') | script -qec 'aur-prefetch tui; stty -a | grep -q icanon' /dev/null >/dev/null 2>&1
    assert_eq "$?" "0" "quit exits 0 and the terminal is restored"

    (sleep 1; printf '\003') | script -qec 'aur-prefetch tui' /dev/null >/dev/null 2>&1
    assert_eq "$?" "130" "ctrl-c exits 130 under a pty"
fi

finish
