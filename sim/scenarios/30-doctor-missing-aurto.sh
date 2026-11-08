#!/usr/bin/env bash
# doctor must check that the aurto include file exists, not just the Include line.
source /usr/local/lib/sim/lib.sh

echo "scenario: doctor reports a missing aurto include file"

printf '1\n' >/etc/sim/snapper.exit
mkdir -p /etc/aurto
chown alpm:alpm /var/cache/pacman/pkg
chmod 0755 /var/cache/pacman/pkg
printf 'root ALL=(root) NOPASSWD: /usr/bin/pacman -Syuw --noconfirm\n' >/etc/sudoers.d/aur-prefetch
chmod 0440 /etc/sudoers.d/aur-prefetch
printf 'maintainer\n' >/etc/aurto/trusted-users
printf 'Include = /etc/pacman.d/aurto\n' >>/etc/pacman.conf
printf '[Journal]\nStorage=persistent\nSystemMaxUse=500M\n' >/etc/systemd/journald.conf
printf 'Tue 2026-11-11 00:00:00 UTC  aur-prefetch.timer\n' >/etc/sim/systemctl.out
rm -f /etc/pacman.d/aurto

log=$(mktemp)
aur-prefetch doctor >"$log" 2>&1
status=$?

assert_eq "$status" "1" "doctor exited 1"
assert_contains "$(cat "$log")" "repo:" "doctor named the repo problem"
assert_contains "$(cat "$log")" "is missing" "doctor named the missing include file"

finish
