#!/usr/bin/env bash
# doctor acts on a disk floor larger than the filesystem.
source /usr/local/lib/sim/lib.sh

echo "scenario: doctor reports a low disk floor"

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

log=$(mktemp)
AUR_PREFETCH_MIN_FREE_BYTES=99999999999999 aur-prefetch doctor >"$log" 2>&1
status=$?

assert_eq "$status" "1" "doctor exited 1"
assert_contains "$(cat "$log")" "below the" "doctor named the low disk"

finish
