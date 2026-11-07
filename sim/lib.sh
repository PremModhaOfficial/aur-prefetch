# Shared assertions for sim scenarios. Source this, do not execute it.
# Scenarios intentionally avoid `set -e`: every assertion must run so one
# failure cannot hide the others.

FAILS=0

pass() { printf '  ok   %s\n' "$*"; }
fail() {
    printf '  FAIL %s\n' "$*" >&2
    FAILS=$((FAILS + 1))
}

assert_eq() {
    local actual=$1 expected=$2 label=$3
    if [[ $actual == "$expected" ]]; then
        pass "$label"
    else
        fail "$label (expected [$expected], got [$actual])"
    fi
}

assert_ne() {
    local actual=$1 unexpected=$2 label=$3
    if [[ $actual != "$unexpected" ]]; then
        pass "$label"
    else
        fail "$label (value is unexpectedly [$unexpected])"
    fi
}

assert_contains() {
    local haystack=$1 needle=$2 label=$3
    if [[ $haystack == *"$needle"* ]]; then
        pass "$label"
    else
        fail "$label (missing [$needle])"
    fi
}

assert_true() {
    local label=$1
    shift
    if "$@"; then pass "$label"; else fail "$label"; fi
}

assert_false() {
    local label=$1
    shift
    if "$@"; then fail "$label"; else pass "$label"; fi
}

installed_version() { pacman -Q simapp 2>/dev/null | awk '{print $2}'; }
payload_version() { /usr/bin/simapp 2>/dev/null | awk '{print $2}'; }

# Every installed package and version. Any install, removal, or upgrade changes
# this, so it catches a stray install of some package other than simapp.
pkg_state() { pacman -Q 2>/dev/null | sort; }

cache_files() {
    find /var/cache/pacman/pkg -maxdepth 1 -type f -printf '%f\n' 2>/dev/null | sort
}

# Content digest of the package payload. Catches an upgrade that somehow kept
# the old version string but replaced files.
usr_state() {
    find /usr/bin/simapp /usr/share/simapp -type f -exec sha256sum {} + 2>/dev/null | sort
}

yay_log() { cat /var/log/sim-yay.log 2>/dev/null || true; }
# The log is pre-created writable at image build, so existence proves nothing.
yay_built_something() { [[ -s /var/log/sim-yay-build.log ]]; }

finish() {
    if ((FAILS > 0)); then
        printf 'FAILED: %d assertion(s)\n' "$FAILS" >&2
        exit 1
    fi
    printf 'PASS\n'
}
