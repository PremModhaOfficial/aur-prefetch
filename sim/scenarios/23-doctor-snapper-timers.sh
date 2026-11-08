#!/usr/bin/env bash
# doctor reports snapper installed without its timers scheduled.
source /usr/local/lib/sim/lib.sh

echo "scenario: doctor reports missing snapper timers"

mkdir -p /etc/aurto /etc/snapper/configs
chown alpm:alpm /var/cache/pacman/pkg
chmod 0755 /var/cache/pacman/pkg
printf 'root ALL=(root) NOPASSWD: /usr/bin/pacman -Syuw --noconfirm\n' >/etc/sudoers.d/aur-prefetch
chmod 0440 /etc/sudoers.d/aur-prefetch
printf 'maintainer\n' >/etc/aurto/trusted-users
printf 'Include = /etc/pacman.d/aurto\n' >>/etc/pacman.conf
printf '[Journal]\nStorage=persistent\nSystemMaxUse=500M\n' >/etc/systemd/journald.conf
printf 'SPACE_LIMIT=0.1-0.5\nFREE_LIMIT=0.1-0.5\n' >/etc/snapper/configs/root
printf 'Tue 2026-11-11 00:00:00 UTC  aur-prefetch.timer\n' >/etc/sim/systemctl.out

log=$(mktemp)
aur-prefetch doctor >"$log" 2>&1
status=$?

assert_eq "$status" "1" "doctor exited 1"
assert_contains "$(cat "$log")" "snapper: timers missing" "doctor named the missing snapper timers"

finish
