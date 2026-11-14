#!/usr/bin/env bash
# #33: doctor's disk row is the need fetch refuses on, not min_free_bytes.
# The configured floor here is 1 MiB and the CacheDir filesystem holds 64 MiB,
# so only the 5 GiB snapshot floor can refuse. Before the shared floor, doctor
# compared against the 1 MiB configured value and exited 0 while fetch refused
# on the same disk.
source /usr/local/lib/sim/lib.sh

echo "scenario: doctor refuses on the floor fetch refuses on"

# The floor keys on the snapper config dir, not on the snapper binary: the
# stub below reports snapper as not installed, as on a machine that removed it.
mkdir -p /etc/snapper/configs
mkdir -p /dev/shm/simcache
chown alpm:alpm /dev/shm/simcache
chmod 0755 /dev/shm/simcache
printf '[options]\nCacheDir = /dev/shm/simcache\nInclude = /etc/pacman.d/aurto\n' >/tmp/sim-pacman.conf

mkdir -p /etc/aurto
printf 'root ALL=(root) NOPASSWD: /usr/bin/pacman -Syuw --noconfirm\n' >/etc/sudoers.d/aur-prefetch
chmod 0440 /etc/sudoers.d/aur-prefetch
printf 'maintainer\n' >/etc/aurto/trusted-users
printf '[Journal]\nStorage=persistent\nSystemMaxUse=500M\n' >/etc/systemd/journald.conf
printf 'Tue 2026-11-11 00:00:00 UTC  aur-prefetch.timer\n' >/etc/sim/systemctl.out
printf '1\n' >/etc/sim/snapper.exit

envs='AUR_PREFETCH_PACMAN_CONF=/tmp/sim-pacman.conf AUR_PREFETCH_MIN_FREE_BYTES=1048576'
before_usr=$(usr_state)

doctor_log=$(mktemp)
env $envs aur-prefetch doctor >"$doctor_log" 2>&1
doctor_status=$?

fetch_log=$(mktemp)
env $envs aur-prefetch fetch >"$fetch_log" 2>&1
fetch_status=$?

assert_eq "$doctor_status" "1" "doctor refused, not healthy"
assert_contains "$(cat "$doctor_log")" "disk /dev/shm/simcache:" \
    "doctor named the CacheDir filesystem"
assert_contains "$(cat "$doctor_log")" "below the" "doctor named the floor"
assert_contains "$(cat "$doctor_log")" "GiB floor" \
    "doctor used the snapshot floor, not the 1 MiB configured one"
assert_contains "$(cat "$doctor_log")" "action needed (1 problem(s))" \
    "the disk row was the only problem"

# The same machine, the same floor: the two verdicts cannot disagree.
assert_eq "$fetch_status" "1" "fetch refused on the same disk"
assert_contains "$(cat "$fetch_log")" "/dev/shm/simcache" \
    "fetch named the same filesystem"

assert_false "nothing was downloaded" \
    bash -c 'compgen -G "/dev/shm/simcache/simapp-2.0.0*" >/dev/null'
assert_eq "$(usr_state)" "$before_usr" "doctor and fetch left /usr unchanged"

finish
