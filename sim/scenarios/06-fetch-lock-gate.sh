#!/usr/bin/env bash
# The lock gate: a live pacman blocks; a stale lock does not, and is never
# deleted.
source /usr/local/lib/sim/lib.sh

echo "scenario: fetch refuses while pacman holds the lock"

touch /var/lib/pacman/db.lck

# A process whose comm is exactly "pacman", without shadowing the real binary.
cp /bin/sleep /tmp/pacman
/tmp/pacman 300 &
holder=$!
sleep 0.2

runuser -l builder -c 'aur-prefetch fetch' >/tmp/sim-lock.log 2>&1
status=$?

assert_eq "$status" "1" "fetch refused while the lock was held"
assert_contains "$(cat /tmp/sim-lock.log)" "pacman is running" "the refusal names the running process"
assert_false "nothing was downloaded" bash -c 'compgen -G "/var/cache/pacman/pkg/simapp-2.0.0*" >/dev/null'
assert_true "the lock file still exists" test -f /var/lib/pacman/db.lck

kill "$holder" 2>/dev/null || true
wait "$holder" 2>/dev/null || true

# The same file with no live pacman is stale. pacman would refuse anyway, so
# the gate must block with exit 1 and must not remove the file.
runuser -l builder -c 'aur-prefetch fetch' >/tmp/sim-stale-lock.log 2>&1
status=$?

assert_eq "$status" "1" "a stale lock blocked with exit 1"
assert_contains "$(cat /tmp/sim-stale-lock.log)" "stale lock" "the refusal calls the lock stale"
assert_false "the stale lock downloaded nothing" bash -c 'compgen -G "/var/cache/pacman/pkg/simapp-2.0.0*" >/dev/null'
assert_true "the stale lock was left alone" test -f /var/lib/pacman/db.lck

finish
