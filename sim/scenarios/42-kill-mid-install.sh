#!/usr/bin/env bash
# Kill pacman in the middle of an install and ask the two file checks which one
# tells the truth.
#
# The fault is a file-size rlimit, not a watcher racing a signal: the install
# runs with `ulimit -f 1024`, so the kernel kills pacman with SIGXFSZ exactly
# when it writes past 1 MiB. The package being upgraded carries an 8 MiB
# payload, so the kill lands inside the extraction of that one file, and it
# lands there every run.
#
# What that leaves is the classic interrupted-upgrade shape: the local DB has
# already recorded the new version, its file list was never written, and /usr
# holds a truncated payload. `pacman -Qk` trusts the file list, finds none, and
# reports a clean system; `pacman -Qkk` compares what is on disk and reports the
# damage. This scenario asserts both verdicts, so the honest one is recorded as
# a fact rather than a claim in a comment.
source /usr/local/lib/sim/lib.sh

echo "scenario: kill pacman mid-install, and record which file check is honest"

pkgname=bigpkg
v1=1.0.0
v2=2.0.0
v1_size=4194304
v2_size=8388608
limit=1024 # KiB, `ulimit -f`'s unit
cache=/var/cache/pacman/pkg

# A baseline the upgrade can damage: v1 installed, v2 published on top.
make-pkg.sh "$pkgname" "$v1" "$v1_size" >/dev/null
pacman -Syy --noconfirm >/dev/null
pacman -S --noconfirm "$pkgname" >/dev/null
make-pkg.sh "$pkgname" "$v2" "$v2_size" >/dev/null
pacman -Syy --noconfirm >/dev/null

assert_eq "$(pacman -Q "$pkgname")" "$pkgname $v1-1" "the baseline package is installed"
assert_eq "$(stat -c %s "/usr/share/$pkgname/blob")" "$v1_size" "the baseline payload is whole"
assert_eq "$(pacman -Qu "$pkgname")" "$pkgname $v1-1 -> $v2-1" "the upgrade is pending"

pending_simapp=$(installed_version)
before_usr=$(usr_state)
before_local=$(local_db_state)

log=$(mktemp)
runuser -l installer -c "ulimit -f $limit; aur-prefetch install --yes" >"$log" 2>&1
status=$?

# 1. The failure is loud: an execution error, no success line, and the tool
#    names the command that died rather than the command it ran.
assert_eq "$status" "2" "a killed install is an execution error (exit 2, D6)"
assert_false "install never claims success" \
    bash -c 'grep -q "install: ok" "$1"' _ "$log"
assert_contains "$(cat "$log")" "yay -Syuu exited -1" \
    "the failure names the killed command and its signal death"
assert_contains "$(cat "$log")" "upgrading $pkgname..." \
    "pacman had started processing the package when it died"

# 2. The kill landed inside the extraction: the payload exists, truncated at
#    exactly the limit, and the transfer that produced it had finished.
assert_eq "$(stat -c %s "/usr/share/$pkgname/blob")" "$((limit * 1024))" \
    "the payload was cut off at the file-size limit (SIGXFSZ, not a guess)"
assert_true "the download itself had completed" \
    test -f "$cache/$pkgname-$v2-1-x86_64.pkg.tar.zst"
assert_false "the aborted install left no partial download" \
    bash -c 'compgen -G "$1/*.part" >/dev/null' _ "$cache"

# 3. A hard kill leaves the lock. Removing it is the user's step, never ours:
#    scenario 06 owns that refusal.
assert_true "the killed install left db.lck behind" test -f /var/lib/pacman/db.lck

# 4. The local DB records the version that never landed, with no file list:
#    that half-written entry is the reason the two checks below disagree.
assert_eq "$(pacman -Q "$pkgname")" "$pkgname $v2-1" \
    "the local DB already records the new version"
assert_true "the local DB entry exists" \
    test -d "/var/lib/pacman/local/$pkgname-$v2-1"
assert_false "but its file list was never written" \
    test -e "/var/lib/pacman/local/$pkgname-$v2-1/files"

# 5. The two checks, asked about the same damage. -Qk is dishonest here: with no
#    file list it checks nothing and still exits 0.
qk=$(mktemp); qkk=$(mktemp)
pacman -Qk "$pkgname" >"$qk" 2>&1
qk_status=$?
pacman -Qkk "$pkgname" >"$qkk" 2>&1
qkk_status=$?

assert_eq "$qk_status" "0" "pacman -Qk calls the damaged system clean (exit 0)"
assert_contains "$(cat "$qk")" "0 total files, 0 missing files" \
    "pacman -Qk checked nothing at all"
assert_eq "$qkk_status" "1" "pacman -Qkk reports the damage (exit 1)"
assert_contains "$(cat "$qkk")" "Size mismatch" "pacman -Qkk names the truncated file"
assert_contains "$(cat "$qkk")" "altered file" "pacman -Qkk counts the damage"
assert_eq "$(grep -c "SHA256 checksum mismatch" "$qkk")" "1" \
    "pacman -Qkk compares the payload against the package's own checksum"

# 6. The damage stayed inside this package: the other pending upgrade never ran
#    and simapp's payload is byte-identical.
assert_eq "$(installed_version)" "$pending_simapp" \
    "the other pending package in the same transaction was not touched"
assert_eq "$(usr_state)" "$before_usr" "the killed transaction wrote nothing to simapp"

# 7. Retry the way a user must: remove the dead pacman's lock, then run the same
#    install with no limit. It exits 0 and does not repair anything, because the
#    half-written DB entry above already claims the new version is installed, so
#    pacman has nothing to do for this package. A green install over a damaged
#    system is the gap this step records.
rm -f /var/lib/pacman/db.lck
retry=$(mktemp)
runuser -l installer -c 'aur-prefetch install --yes' >"$retry" 2>&1
status=$?

assert_eq "$status" "0" "the retried install exits 0"
assert_contains "$(cat "$retry")" "install: done" "the retried install claims success"
assert_eq "$(stat -c %s "/usr/share/$pkgname/blob")" "$((limit * 1024))" \
    "yet the truncated payload is still truncated"
still=$(mktemp)
pacman -Qkk "$pkgname" >"$still" 2>&1
assert_eq "$?" "1" "and pacman -Qkk still reports the damage"

# 8. The repair is not `pacman -S $pkgname`: with the half-written entry in the
#    way pacman refuses to prepare the transaction, and with the entry gone the
#    truncated file is an unowned file it refuses to overwrite. A user has to
#    clear both. Afterwards -Qkk is quiet on the same package, so its red above
#    was the damage and not a check that is always red.
blocked=$(mktemp)
pacman -S --noconfirm "$pkgname" >"$blocked" 2>&1
assert_eq "$?" "1" "a plain reinstall is refused while the broken entry is there"
assert_contains "$(cat "$blocked")" "could not fully load metadata" \
    "pacman names the entry it cannot read"

rm -rf "/var/lib/pacman/local/$pkgname-$v2-1"
pacman -S --noconfirm --overwrite "/usr/share/$pkgname/*" "$pkgname" >/dev/null 2>&1
assert_eq "$(stat -c %s "/usr/share/$pkgname/blob")" "$v2_size" \
    "clearing the entry and overwriting the damaged file repairs the payload"
after=$(mktemp)
pacman -Qkk "$pkgname" >"$after" 2>&1
assert_eq "$?" "0" "pacman -Qkk is clean on the repaired system"
assert_contains "$(cat "$after")" "0 altered files" "pacman -Qkk finds nothing altered"

# 9. Invariants: yay ran the full upgrade, nothing built from the AUR, and the
#    only escalation was the scoped helper, on both runs.
assert_eq "$(grep -c -- '-Syuu' /var/log/sim-yay.log)" "2" "yay was asked twice for -Syuu"
assert_false "no install ran an AUR build" yay_built_something
assert_eq "$(grep -vc '^-n /usr/local/bin/sim-upgrade$' /var/log/sim-sudo.log)" "0" \
    "the scoped helper is the only escalation"

finish
