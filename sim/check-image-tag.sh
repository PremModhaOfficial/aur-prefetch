#!/usr/bin/env bash
# Prove the sim image tag comes from the worktree, and that SIM_IMAGE wins.
#
# No container runtime runs here: a stub runtime records the argv that
# `run.sh` would have handed to docker or podman, so the assertion covers the
# real build AND run path and needs nothing installed.
set -euo pipefail

repo=$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)
tmp=$(mktemp -d)
trap 'rm -rf "$tmp"' EXIT

cat >"$tmp/runtime" <<'STUB'
#!/usr/bin/env bash
printf '%s\n' "$*"
STUB
chmod +x "$tmp/runtime"

# argv <dir> <image-override|-> [run.sh flags...]
argv() {
    local dir=$1 image=$2
    shift 2
    local -a envs=("SIM_RUNTIME=$tmp/runtime")
    [[ $image != - ]] && envs+=("SIM_IMAGE=$image")
    ( cd "$dir" && env "${envs[@]}" ./sim/run.sh "$@" )
}

tag() { argv "$@" --build-only | sed -n 's/.* -t \([^ ]*\) .*/\1/p'; }

scenario=sim/scenarios/01-download-only-keeps-usr.sh
# The image is the argument before the scenario path on the `run` line.
run_tag() { argv "$@" "$scenario" | awk '/^run /{print $(NF - 1)}'; }

for name in alpha beta; do
    mkdir -p "$tmp/$name"
    cp -r "$repo/sim" "$tmp/$name/sim"
done

a=$(tag "$tmp/alpha" -)
b=$(tag "$tmp/beta" -)
[[ $a =~ ^aur-prefetch-sim:alpha-[0-9a-f]{8}$ ]] ||
    { echo "sim: alpha derived '$a', expected aur-prefetch-sim:alpha-<8 hex>" >&2; exit 1; }
[[ $b =~ ^aur-prefetch-sim:beta-[0-9a-f]{8}$ ]] ||
    { echo "sim: beta derived '$b', expected aur-prefetch-sim:beta-<8 hex>" >&2; exit 1; }
[[ $a != "$b" ]] ||
    { echo "sim: two worktrees share the tag '$a'" >&2; exit 1; }

# Two paths with the same leaf name are two worktrees, and must not collide.
mkdir -p "$tmp/same/one/repo" "$tmp/same/two/repo"
for dir in "$tmp/same/one/repo" "$tmp/same/two/repo"; do cp -r "$repo/sim" "$dir/sim"; done
c=$(tag "$tmp/same/one/repo" -)
d=$(tag "$tmp/same/two/repo" -)
[[ $c != "$d" ]] ||
    { echo "sim: sibling paths with one leaf name share the tag '$c'" >&2; exit 1; }

override=$(tag "$tmp/alpha" example:override)
[[ $override == "example:override" ]] ||
    { echo "sim: SIM_IMAGE ignored, build used '$override'" >&2; exit 1; }

built=$(argv "$tmp/alpha" - --build-only --no-cache)
[[ $built == build\ --no-cache\ -f\ sim/Dockerfile\ -t\ * ]] ||
    { echo "sim: --no-cache not passed to the build: '$built'" >&2; exit 1; }

ran=$(run_tag "$tmp/alpha" -)
[[ $ran == "$a" ]] ||
    { echo "sim: build used '$a' but the container ran '$ran'" >&2; exit 1; }

echo "sim: per-worktree tags (alpha=$a beta=$b, same-leaf c=$c d=$d), SIM_IMAGE wins (override=$override), --no-cache reaches the build, build and run agree"
