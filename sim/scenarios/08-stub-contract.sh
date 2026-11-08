#!/usr/bin/env bash
# Harness self-test: stubs replay configured responses, a configured paccache
# failure does not forward to the real tool, builder can write the logs, and
# yay rejects root on every path.
source /usr/local/lib/sim/lib.sh

echo "scenario: the harness stubs behave as configured"

out=$(mktemp)
err=$(mktemp)

printf 'NEXT\n' >/etc/sim/systemctl.out
printf 'boom\n' >/etc/sim/systemctl.err
printf '7\n' >/etc/sim/systemctl.exit

systemctl --user list-timers >"$out" 2>"$err"
status=$?

assert_eq "$status" "7" "stub applied the configured exit status"
assert_eq "$(cat "$out")" "NEXT" "stub replayed stdout before exiting"
assert_eq "$(cat "$err")" "boom" "stub replayed stderr before exiting"
assert_contains "$(cat /var/log/sim-systemctl.log)" "--user list-timers" "stub logged argv"

runuser -u builder -- systemctl --user list-timers >"$out" 2>"$err" || true
assert_eq "$(wc -l < /var/log/sim-systemctl.log | tr -d ' ')" "2" "builder can append to the stub log"

printf '9\n' >/etc/sim/paccache.exit
printf 'cache failure\n' >/etc/sim/paccache.err
paccache -d >"$out" 2>"$err"
status=$?

assert_eq "$status" "9" "paccache applied the configured failure"
assert_eq "$(cat "$err")" "cache failure" "paccache replayed the error"
assert_eq "$(cache_files)" "" "a configured failure did not forward to the real tool"

yay -Qua >"$out" 2>"$err"
status=$?

assert_eq "$status" "98" "yay refused root even for a query"
assert_contains "$(cat "$err")" "refusing to run yay as root" "the refusal is loud"
assert_contains "$(cat /var/log/sim-yay.log)" "-Qua" "the root attempt was recorded"
assert_false "a root refusal is not a build" yay_built_something

finish
