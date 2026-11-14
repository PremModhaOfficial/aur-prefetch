#!/usr/bin/env bash
# Prove the sim image tag comes from the worktree, and that SIM_IMAGE wins.
#
# No container runtime runs here: a stub runtime records the argv that
# `run.sh --build-only` would have handed to docker or podman, so the assertion
# covers the real build path and needs nothing installed.
set -euo pipefail

repo=$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)
tmp=$(mktemp -d)
trap 'rm -rf "$tmp"' EXIT

cat >"$tmp/runtime" <<'STUB'
#!/usr/bin/env bash
printf '%s\n' "$*"
STUB
chmod +x "$tmp/runtime"

# build_argv <dir> <image-override|-> [run.sh flags...]
build_argv() {
    local dir=$1 image=$2
    shift 2
    local -a envs=("SIM_RUNTIME=$tmp/runtime")
    [[ $image != - ]] && envs+=("SIM_IMAGE=$image")
    ( cd "$dir" && env "${envs[@]}" ./sim/run.sh --build-only "$@" )
}

tag() { build_argv "$@" | sed -n 's/.* -t \([^ ]*\) .*/\1/p'; }

for name in alpha beta; do
    mkdir -p "$tmp/$name"
    cp -r "$repo/sim" "$tmp/$name/sim"
done

a=$(tag "$tmp/alpha" -)
b=$(tag "$tmp/beta" -)
[[ $a == "aur-prefetch-sim:alpha" ]] ||
    { echo "sim: alpha derived '$a', expected aur-prefetch-sim:alpha" >&2; exit 1; }
[[ $b == "aur-prefetch-sim:beta" ]] ||
    { echo "sim: beta derived '$b', expected aur-prefetch-sim:beta" >&2; exit 1; }
[[ $a != "$b" ]] ||
    { echo "sim: two worktrees share the tag '$a'" >&2; exit 1; }

override=$(tag "$tmp/alpha" example:override)
[[ $override == "example:override" ]] ||
    { echo "sim: SIM_IMAGE ignored, build used '$override'" >&2; exit 1; }

argv=$(build_argv "$tmp/alpha" - --no-cache | head -n 1)
[[ $argv == build\ --no-cache\ -f\ sim/Dockerfile\ -t\ * ]] ||
    { echo "sim: --no-cache not passed to the build: '$argv'" >&2; exit 1; }

echo "sim: per-worktree tags (alpha=$a beta=$b), SIM_IMAGE wins (override=$override), --no-cache reaches the build"
