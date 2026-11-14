#!/usr/bin/env bash
# Scenario numbers are hand-assigned in a namespace every worktree shares, so
# two PRs both adding 47 never collide in git: the clash shows up later, in
# sim/README.md (#61). Fail here instead of at review.
#
#   scripts/check-sim-numbers.sh               # the real directory
#   scripts/check-sim-numbers.sh <dir>         # another directory of NN-*.sh
#   scripts/check-sim-numbers.sh --self-test   # prove the check can fail
set -euo pipefail

cd "$(dirname "${BASH_SOURCE[0]}")/.."

check() {
    local dir=$1 status=0 count=0 number name path
    local -A seen=()
    local -a files

    shopt -s nullglob
    for path in "$dir"/*.sh; do
        count=$((count + 1))
        name=${path##*/}
        if [[ ! ${name%%-*} =~ ^[0-9]+$ ]]; then
            echo "::error::$path has no numeric prefix; name a scenario NN-slug.sh"
            status=1
            continue
        fi
        # 10# so 01-a.sh and 1-b.sh are the same number, not two.
        number=$((10#${name%%-*}))
        seen[$number]=${seen[$number]:+${seen[$number]}$'\n'}$name
    done
    shopt -u nullglob

    # nullglob makes a renamed or empty directory look like a clean one.
    if ((count == 0)); then
        echo "::error::no scenario files under $dir"
        return 1
    fi

    if ((${#seen[@]})); then
        while IFS= read -r number; do
            mapfile -t files <<<"${seen[$number]}"
            if ((${#files[@]} > 1)); then
                echo "::error::sim scenario number $number is not unique: ${files[*]}"
                status=1
            fi
        done < <(printf '%s\n' "${!seen[@]}" | sort -n)
    fi

    return "$status"
}

if [[ ${1:-} == --self-test ]]; then
    tmp=$(mktemp -d)
    trap 'rm -rf "$tmp"' EXIT
    mkdir -p "$tmp/clean" "$tmp/planted" "$tmp/unpadded" "$tmp/unprefixed" "$tmp/empty"
    : >"$tmp/clean/01-a.sh"
    : >"$tmp/clean/02-b.sh"
    : >"$tmp/planted/01-a.sh"
    : >"$tmp/planted/01-b.sh"
    : >"$tmp/unpadded/01-a.sh"
    : >"$tmp/unpadded/1-b.sh"
    : >"$tmp/unprefixed/a.sh"

    check "$tmp/clean" >/dev/null || { echo "self-test: a clean set was rejected"; exit 1; }
    check "$tmp/planted" >/dev/null && { echo "self-test: a duplicate number passed"; exit 1; }
    check "$tmp/unpadded" >/dev/null && { echo "self-test: 01 and 1 passed as different"; exit 1; }
    check "$tmp/unprefixed" >/dev/null && { echo "self-test: an unprefixed name passed"; exit 1; }
    check "$tmp/empty" >/dev/null && { echo "self-test: an empty directory passed"; exit 1; }
    echo "self-test: ok"
    exit 0
fi

check "${1:-sim/scenarios}" || exit 1
echo "sim scenario numbers: unique"
