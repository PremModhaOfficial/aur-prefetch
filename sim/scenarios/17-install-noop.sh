#!/usr/bin/env bash
# install on a current repo: exit 0, no change, no AUR build.
source /usr/local/lib/sim/lib.sh

echo "scenario: install on a current repo changes nothing"

pacman -Syu --noconfirm >/dev/null

# The helper runs as installer; make its log writable for this container.
touch /var/log/sim-yay.log
chmod 0666 /var/log/sim-yay.log

log=$(mktemp)
before_usr=$(usr_state)

runuser -l installer -c 'aur-prefetch install --yes' >"$log" 2>&1
status=$?

assert_eq "$status" "0" "install exited 0"
assert_eq "$(installed_version)" "2.0.0-1" "the installed version is unchanged"
assert_eq "$(usr_state)" "$before_usr" "/usr is unchanged"
assert_false "no AUR build ran" yay_built_something

finish
