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
    (sleep 1; printf 'q') | timeout 10 script -qec 'aur-prefetch tui; stty -a | grep -q icanon' /dev/null >/dev/null 2>&1
    assert_eq "$?" "0" "quit exits 0 and the terminal is restored"

    (sleep 1; printf '\003') | timeout 10 script -qec 'aur-prefetch tui' /dev/null >/dev/null 2>&1
    assert_eq "$?" "130" "ctrl-c exits 130 under a pty"

    panic_out=$(mktemp)
    (sleep 1) | AUR_PREFETCH_TUI_TEST_PANIC=1 timeout 10 script -qec 'aur-prefetch tui' /dev/null >"$panic_out" 2>&1
    status=$?
    assert_eq "$status" "101" "a forced panic exits 101"
    assert_contains "$(cat "$panic_out")" "forced TUI panic" "the panic message is visible"

    if [[ $status == 101 ]]; then
        (sleep 1) | AUR_PREFETCH_TUI_TEST_PANIC=1 timeout 10 script -qec 'aur-prefetch tui; stty -a | grep -q icanon' /dev/null >/dev/null 2>&1
        assert_eq "$?" "0" "a forced panic leaves the terminal restored"
    else
        fail "sim-hooks is missing; the panic restore check did not run"
    fi

    resume_out=$(mktemp)
    (
        sleep 1
        printf '\032'
        sleep 1
        pid=$(pgrep -x aur-prefetch | head -1 || true)
        [[ -n ${pid:-} ]] && kill -CONT "$pid" 2>/dev/null || true
        sleep 1
        printf 'q'
    ) | timeout 15 script -qec 'aur-prefetch tui; rc=$?; stty -a | grep -q icanon && test "$rc" = 0' /dev/null >"$resume_out" 2>&1
    assert_eq "$?" "0" "ctrl-z resumes and the terminal is restored"
    enters=$(grep -o '\[?1049h' "$resume_out" | wc -l)
    assert_true "the tui re-enters the screen after resume" test "$enters" -ge 2
    clears=$(grep -o '\[2J' "$resume_out" | wc -l)
    assert_true "the tui clears and redraws after resume" test "$clears" -ge 1
fi

finish
