#!/usr/bin/env bash
# doctor must read journald.conf.d drop-ins, not just the main file.
source /usr/local/lib/sim/lib.sh

echo "scenario: doctor reads journald drop-ins"

printf '1\n' >/etc/sim/snapper.exit
mkdir -p /etc/aurto /etc/systemd/journald.conf.d
chown alpm:alpm /var/cache/pacman/pkg
chmod 0755 /var/cache/pacman/pkg
printf 'root ALL=(root) NOPASSWD: /usr/bin/pacman -Syuw --noconfirm\n' >/etc/sudoers.d/aur-prefetch
chmod 0440 /etc/sudoers.d/aur-prefetch
printf 'maintainer\n' >/etc/aurto/trusted-users
printf 'Include = /etc/pacman.d/aurto\n' >>/etc/pacman.conf
printf '[Journal]\nStorage=volatile\n' >/etc/systemd/journald.conf
printf '[Journal]\nStorage = persistent\nSystemMaxUse = 500M\n' \
    >/etc/systemd/journald.conf.d/50-persistent.conf
printf 'Tue 2026-11-11 00:00:00 UTC  aur-prefetch.timer\n' >/etc/sim/systemctl.out

log=$(mktemp)
aur-prefetch doctor >"$log" 2>&1
status=$?

assert_eq "$status" "0" "doctor exited 0"
assert_contains "$(cat "$log")" "journald: Storage=persistent" "doctor used the drop-in storage setting"
assert_contains "$(cat "$log")" "journald: a size cap is set" "doctor used the drop-in size cap"

finish
