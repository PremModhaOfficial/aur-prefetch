#!/usr/bin/env bash
# doctor rejects a sudoers file that grants more than the exact rule.
source /usr/local/lib/sim/lib.sh

echo "scenario: doctor rejects a wildcard sudoers grant"

printf '1\n' >/etc/sim/snapper.exit
mkdir -p /etc/aurto
chown alpm:alpm /var/cache/pacman/pkg
chmod 0755 /var/cache/pacman/pkg
printf 'builder ALL=(ALL : ALL) NOPASSWD: ALL\n' >/etc/sudoers.d/aur-prefetch
printf 'builder ALL=(root) NOPASSWD: /usr/bin/pacman -Syuw --noconfirm\n' >>/etc/sudoers.d/aur-prefetch
chmod 0440 /etc/sudoers.d/aur-prefetch
visudo -cf /etc/sudoers.d/aur-prefetch >/dev/null || {
    echo "sim: the wildcard fixture is not valid sudoers" >&2
    exit 1
}
printf 'maintainer\n' >/etc/aurto/trusted-users
printf 'Include = /etc/pacman.d/aurto\n' >>/etc/pacman.conf
printf '[Journal]\nStorage=persistent\nSystemMaxUse=500M\n' >/etc/systemd/journald.conf
printf 'Tue 2026-11-11 00:00:00 UTC  aur-prefetch.timer\n' >/etc/sim/systemctl.out

log=$(mktemp)
runuser -l builder -c 'aur-prefetch doctor' >"$log" 2>&1
status=$?

assert_eq "$status" "1" "doctor exited 1"
assert_contains "$(cat "$log")" "sudoers:" "doctor reported the sudoers problem"

finish
