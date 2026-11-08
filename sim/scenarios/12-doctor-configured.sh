#!/usr/bin/env bash
# doctor on a configured container: exit 0.
source /usr/local/lib/sim/lib.sh

echo "scenario: doctor passes on a configured system"

printf '1\n' >/etc/sim/snapper.exit

mkdir -p /etc/aurto
# pacman 7 downloads as alpm; the configured cache must be writable by it.
chown alpm:alpm /var/cache/pacman/pkg
chmod 0755 /var/cache/pacman/pkg
printf 'root ALL=(root) NOPASSWD: /usr/bin/pacman -Syuw --noconfirm\n' >/etc/sudoers.d/aur-prefetch
chmod 0440 /etc/sudoers.d/aur-prefetch
printf 'maintainer\n' >/etc/aurto/trusted-users
printf 'Include = /etc/pacman.d/aurto\n' >>/etc/pacman.conf
printf '[Journal]\nStorage=persistent\nSystemMaxUse=500M\n' >/etc/systemd/journald.conf
printf 'Tue 2026-11-11 00:00:00 UTC  aur-prefetch.timer\n' >/etc/sim/systemctl.out

log=$(mktemp)
before_usr=$(usr_state)

aur-prefetch doctor >"$log" 2>&1
status=$?

assert_eq "$status" "0" "doctor exited 0"
assert_contains "$(cat "$log")" "doctor: ok" "doctor reported ok"
assert_eq "$(usr_state)" "$before_usr" "doctor left /usr unchanged"

# The product runs as the user, and a correct 0440 root-owned rule is
# unreadable. Doctor must verify it through sudo -l and still pass.
printf 'builder ALL=(root) NOPASSWD: /usr/bin/pacman -Syuw --noconfirm\n' >/etc/sudoers.d/aur-prefetch
chmod 0440 /etc/sudoers.d/aur-prefetch
runuser -l builder -c 'aur-prefetch doctor' >"$log" 2>&1
status=$?

assert_eq "$status" "0" "doctor as builder exited 0"
assert_contains "$(cat "$log")" "sudo -l matches" "doctor verified the unreadable rule through sudo -l"
assert_eq "$(usr_state)" "$before_usr" "doctor as builder left /usr unchanged"

finish
