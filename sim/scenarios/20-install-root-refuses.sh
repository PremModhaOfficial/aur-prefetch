#!/usr/bin/env bash
# install must never run yay or a PKGBUILD as root.
source /usr/local/lib/sim/lib.sh

echo "scenario: install refuses to run as root"

touch /var/log/sim-yay.log
chmod 0666 /var/log/sim-yay.log

log=$(mktemp)
before_usr=$(usr_state)

# Plain root: real and effective UID are both 0.
aur-prefetch install --yes >"$log" 2>&1
status=$?

assert_eq "$status" "1" "install refused with exit 1"
assert_contains "$(cat "$log")" "must run as the user, not root" "the refusal says why"
assert_eq "$(yay_log)" "" "yay never ran"
assert_eq "$(installed_version)" "1.0.0-1" "nothing was installed"
assert_eq "$(usr_state)" "$before_usr" "/usr is unchanged"

# sudo shape: real UID is the user, effective UID is root. `id -u` must report
# the effective UID; /proc/self metadata would report the real UID and let this
# through.
setpriv --ruid "$(id -u installer)" --rgid "$(id -g installer)" --clear-groups \
    --euid 0 /usr/local/bin/aur-prefetch install --yes >"$log" 2>&1
status=$?

assert_eq "$status" "1" "the sudo shape refused with exit 1"
assert_contains "$(cat "$log")" "must run as the user, not root" \
    "the sudo shape refuses by effective UID"
assert_eq "$(yay_log)" "" "yay never ran under the sudo shape"
assert_eq "$(installed_version)" "1.0.0-1" "the sudo shape installed nothing"
assert_eq "$(usr_state)" "$before_usr" "/usr is unchanged under the sudo shape"

finish
