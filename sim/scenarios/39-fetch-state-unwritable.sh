#!/usr/bin/env bash
# A download that completed must not be reported as a failure because the state
# file could not be written. The daily timer reads that exit code.
#
# The packages are already in the cache when the write is attempted, so exit 2
# here means "the unit went red for a fetch that worked".
source /usr/local/lib/sim/lib.sh

echo "scenario: a completed fetch exits 0 when the state file cannot be written"

state_dir=/home/builder/state-readonly
install -d -o builder -g builder "$state_dir"
chmod 555 "$state_dir"

before_usr=$(usr_state)
before_pkgs=$(pkg_state)
log=$(mktemp)

runuser -l builder -c "AUR_PREFETCH_STATE=$state_dir/state.json aur-prefetch fetch" >"$log" 2>&1
status=$?

assert_eq "$status" "0" "a completed download is not a failure"
assert_contains "$(cat "$log")" "fetch: warning:" "the warning is loud"
assert_contains "$(cat "$log")" "$state_dir/state.json" "the warning names the state file"
assert_contains "$(cache_files)" "simapp-2.0.0" "the download really happened"
assert_eq "$(installed_version)" "1.0.0-1" "fetch did not install the download"
assert_eq "$(usr_state)" "$before_usr" "fetch left /usr unchanged"
assert_eq "$(pkg_state)" "$before_pkgs" "fetch changed no package"
assert_false "the read-only directory stayed empty" test -e "$state_dir/state.json"

finish
