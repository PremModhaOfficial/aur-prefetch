#!/usr/bin/env bash
# The lock gate is the process list, not only db.lck: yay releases the lock
# between pacman calls and spends most of a build inside makepkg, so no process
# is named pacman and no lock file exists. fetch must still refuse.
source /usr/local/lib/sim/lib.sh

echo "scenario: fetch refuses while yay, makepkg or aurto is running"

assert_true "no lock file to begin with" test ! -f /var/lib/pacman/db.lck

for name in pacman yay makepkg aurto; do
    log=$(mktemp)

    # A process whose comm is exactly "$name", without shadowing a real binary.
    cp /bin/sleep "/tmp/$name"
    "/tmp/$name" 300 &
    holder=$!
    sleep 0.2

    runuser -l builder -c 'aur-prefetch fetch' >"$log" 2>&1
    status=$?

    assert_eq "$status" "1" "$name: fetch refused with exit 1"
    assert_contains "$(cat "$log")" "$name is running" "the refusal names $name"
    assert_false "$name: nothing was downloaded" \
        bash -c 'compgen -G "/var/cache/pacman/pkg/simapp-2.0.0*" >/dev/null'
    assert_true "$name: fetch left no lock behind" test ! -f /var/lib/pacman/db.lck

    kill "$holder" 2>/dev/null || true
    wait "$holder" 2>/dev/null || true
    rm -f "/tmp/$name"
done

assert_eq "$(installed_version)" "1.0.0-1" "nothing was installed"

# The read-only health check must report the same live process instead of
# "lock: none", or it sends the user nowhere when a fetch refuses.
check_log=$(mktemp)
cp /bin/sleep /tmp/makepkg
/tmp/makepkg 300 &
holder=$!
sleep 0.2

runuser -l builder -c 'aur-prefetch check' >"$check_log" 2>&1

assert_contains "$(cat "$check_log")" "lock: held by makepkg" \
    "check names the live build process"

kill "$holder" 2>/dev/null || true
wait "$holder" 2>/dev/null || true

finish
