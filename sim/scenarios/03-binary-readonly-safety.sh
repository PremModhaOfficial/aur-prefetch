#!/usr/bin/env bash
# Invariants that must hold for every future implementation, not just today.
#
#   - check and doctor are read-only: no /usr, no cache, no AUR build.
#   - fetch never writes to /usr and never builds from the AUR.
#
# Positive assertions ("fetch downloaded this", "install upgraded that") are
# added with each command's implementation. This scenario is not a substitute
# for them.
source /usr/local/lib/sim/lib.sh

echo "scenario: read-only commands stay read-only, fetch never touches /usr"

cmd_log=$(mktemp)
fetch_log=$(mktemp)

before_usr=$(usr_state)
before_pkgs=$(pkg_state)
before_cache=$(cache_files)

for cmd in check doctor; do
    aur-prefetch "$cmd" >"$cmd_log" 2>&1 || true
    assert_eq "$(usr_state)" "$before_usr" "$cmd leaves /usr unchanged"
    assert_eq "$(pkg_state)" "$before_pkgs" "$cmd installs nothing"
    assert_eq "$(cache_files)" "$before_cache" "$cmd leaves download cache unchanged"
done

# fetch may write the sync DB and the cache, but never installed files.
aur-prefetch fetch >"$fetch_log" 2>&1 || true
assert_eq "$(usr_state)" "$before_usr" "fetch leaves /usr unchanged"
assert_eq "$(pkg_state)" "$before_pkgs" "fetch installs nothing"
assert_false "fetch never runs an AUR build" yay_built_something

assert_contains "$(aur-prefetch --help 2>&1)" "fetch" "help lists subcommands"

finish
