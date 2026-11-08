#!/usr/bin/env bash
# doctor must resolve a symlinked CacheDir before checking alpm traversal.
source /usr/local/lib/sim/lib.sh

echo "scenario: doctor checks the real CacheDir target"

printf '1\n' >/etc/sim/snapper.exit
mkdir -p /etc/aurto /etc/pacman.d /srv
chmod 0700 /srv
printf '[aurto]\nServer = file:///srv/empty-repo\n' >/etc/pacman.d/aurto

# The symlink is traversable by anyone, but alpm cannot enter /srv.
mkdir -p /srv/real-cache
chown alpm:alpm /srv/real-cache
chmod 0755 /srv/real-cache
ln -s /srv/real-cache /tmp/cache-link

printf 'root ALL=(root) NOPASSWD: /usr/bin/pacman -Syuw --noconfirm\n' >/etc/sudoers.d/aur-prefetch
chmod 0440 /etc/sudoers.d/aur-prefetch
printf 'maintainer\n' >/etc/aurto/trusted-users
printf '[Journal]\nStorage=persistent\nSystemMaxUse=500M\n' >/etc/systemd/journald.conf
printf 'Tue 2026-11-11 00:00:00 UTC  aur-prefetch.timer\n' >/etc/sim/systemctl.out

log=$(mktemp)
AUR_PREFETCH_PACMAN_CONF=/usr/local/lib/sim/fixtures/pacman-symlink-cache.conf \
    aur-prefetch doctor >"$log" 2>&1
status=$?

assert_eq "$status" "1" "doctor exited 1"
assert_contains "$(cat "$log")" "cache: /tmp/cache-link" "doctor named the symlinked cache"
assert_contains "$(cat "$log")" "not writable by alpm" "doctor checked the real target's parent"

finish
