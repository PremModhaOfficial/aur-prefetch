#!/usr/bin/env bash
# Drive the Arch simulation harness.
#
#   ./sim/run.sh                          build image, run every scenario
#   ./sim/run.sh --build-only             build image only
#   ./sim/run.sh --no-cache               rebuild from scratch, no layer cache
#   ./sim/run.sh sim/scenarios/03-*.sh    run selected scenarios
#   SIM_IMAGE=name:tag ./sim/run.sh       override the image tag
#   SIM_BINARY=target/release/aur-prefetch ./sim/run.sh
#                                         reuse a host-built binary (same Arch glibc)
#
# Every scenario gets a fresh container with no network. Nothing is written to
# the host.
set -euo pipefail

cd "$(dirname "${BASH_SOURCE[0]}")/.."

RUNTIME=${SIM_RUNTIME:-}
if [[ -z $RUNTIME ]]; then
    for candidate in docker podman; do
        if command -v "$candidate" >/dev/null 2>&1; then
            RUNTIME=$candidate
            break
        fi
    done
fi
if [[ -z ${RUNTIME:-} ]]; then
    echo "sim: need docker or podman (or set SIM_RUNTIME)" >&2
    exit 1
fi

# One tag per worktree: two checkouts running the sim at once must not build
# into, or run from, each other's image. The hash separates two worktrees whose
# leaf directory name happens to match, and survives a name docker would reject.
slug=$(printf %s "${PWD##*/}" | tr -c 'A-Za-z0-9_.-' '-' | cut -c1-40)
IMAGE=${SIM_IMAGE:-aur-prefetch-sim:"$slug"-$(printf %s "$PWD" | sha256sum | cut -c1-8)}

build_image() {
    "$RUNTIME" build "${no_cache[@]}" -f sim/Dockerfile -t "$IMAGE" .
}

build_only=0
no_cache=()
while [[ ${1:-} == --* ]]; do
    case $1 in
    --build-only) build_only=1 ;;
    --no-cache) no_cache=(--no-cache) ;;
    *)
        echo "sim: unknown flag: $1" >&2
        exit 1
        ;;
    esac
    shift
done

if ((build_only)); then
    build_image
    exit 0
fi

scenarios=()
if (($# > 0)); then
    scenarios=("$@")
else
    while IFS= read -r file; do
        scenarios+=("$file")
    done < <(find sim/scenarios -maxdepth 1 -name '*.sh' | sort)
fi

if ((${#scenarios[@]} == 0)); then
    echo "sim: no scenarios found" >&2
    exit 1
fi

build_image

rc=0
for scenario in "${scenarios[@]}"; do
    if [[ ! -f $scenario ]]; then
        echo "sim: missing scenario: $scenario" >&2
        rc=1
        continue
    fi

    echo "=== $scenario ==="
    mount=(-v "$(realpath "$scenario"):/scenario.sh:ro")
    if [[ -n ${SIM_BINARY:-} ]]; then
        mount+=(-v "$(realpath "$SIM_BINARY"):/usr/local/bin/aur-prefetch:ro")
    fi

    # A scenario asks for extra container flags with a header comment, e.g.
    #   # sim-run-args: --tmpfs /var/cache/pacman/pkg:size=1m
    # Values must not contain spaces. A fault-injection scenario uses this to
    # get a filesystem it can fill without changing another scenario's.
    extra=()
    read -ra extra < <(sed -n 's/^# sim-run-args: //p' "$scenario") || true

    if ! "$RUNTIME" run --rm --network none \
        "${mount[@]}" \
        "${extra[@]}" \
        --entrypoint /bin/bash \
        "$IMAGE" /scenario.sh; then
        rc=1
    fi
done

exit "$rc"
