# sim

## Overview

The disposable Arch simulation harness. It builds a container image that runs the
real `aur-prefetch` binary against a real pacman with a synthetic local repo and a
fake AUR helper, so writing behaviours can be proven without touching the
development host. Every scenario runs in a fresh container with `--network none`.
The simulation is the only place the project exercises side effects; the host is
never a test machine.

The directory holds direct source files (`run.sh`, `lib.sh`, `make-repo.sh`,
`sim-upgrade`, `yay`, `Dockerfile`, `pacman.conf`) and three pruned asset
directories — `scenarios/` (test scenarios), `stubs/` (fake tools), and
`fixtures/` (pacman config fixtures) — which are out of scope for this contract
and have no `pipeline.md` of their own.

## Interface

- `run.sh` — harness driver. Flags `--build-only`, `--no-cache`; env vars
  `SIM_RUNTIME` (docker/podman), `SIM_IMAGE` (image tag, default
  `aur-prefetch-sim:<worktree directory>-<8 hex of its path>`), `SIM_BINARY`
  (host-built binary to mount); function `build_image`. [run.sh:--build-only] [run.sh:--no-cache] [run.sh:SIM_RUNTIME] [run.sh:SIM_IMAGE] [run.sh:build_image]
- `check-image-tag.sh` — proves the tag is derived per worktree, that
  `SIM_IMAGE` overrides it, and that the build and the run use one tag; runs a
  stub runtime instead of a real build, and is part of `./ci.sh`.
  [check-image-tag.sh:argv]
- `lib.sh` — shared assertion and probe library sourced by scenarios. Assertions:
  `assert_eq`, `assert_ne`, `assert_contains`, `assert_true`, `assert_false`;
  probes: `installed_version`, `payload_version`, `pkg_state`, `cache_files`,
  `usr_state`, `yay_log`, `yay_built_something`, `sudo_log`; counters/helpers
  `pass`, `fail`, `finish`, and the `FAILS` counter. [lib.sh:assert_eq] [lib.sh:installed_version] [lib.sh:usr_state] [lib.sh:finish]
- `make-repo.sh` — builds the deterministic offline pacman repo; function
  `build_pkg`; produces `simapp` 1.0.0-1 (installed) and 2.0.0-1 (pending). [make-repo.sh:build_pkg]
- `sim-upgrade` — sim-only sudo stand-in for the escalation a real yay performs;
  runs exactly `pacman -Syu --noconfirm`, no arguments. [sim-upgrade]
- `yay` — fake AUR helper. Handles `-Qua` (query) and `-Syu | -Syuu | -Syyu |
  -Syyuu` (upgrade, delegated to `sudo -n /usr/local/bin/sim-upgrade`); exits 97
  on any AUR build path, 98 when run as root. [yay:-Qua] [yay:-Syu]
- `Dockerfile` — two-stage image build (builder stage compiles the Rust binary,
  runtime stage assembles the sim system). [Dockerfile]
- `pacman.conf` — simulation-only pacman config; `[sim]` repo at
  `file:///srv/simrepo`, `SigLevel = Never`, no Arch mirrors. [pacman.conf:[sim]]

## Invariants

- Every scenario runs in a fresh container with `--network none`, so nothing in
  the container can reach the internet or the host. [run.sh:--network none]
- The scenario script and any `SIM_BINARY` are mounted read-only (`:ro`). [run.sh:mount]
- Scenarios deliberately avoid `set -e`; every assertion runs so one failure
  cannot hide the others. [lib.sh:FAILS]
- `finish` exits 1 when `FAILS > 0`, else prints `PASS`. [lib.sh:finish]
- The repo is built with no network, no keys, no AUR; packages are hand-built tar
  archives with `.PKGINFO` and `.MTREE` in the shape makepkg produces. [make-repo.sh:build_pkg]
- Deterministic end state inside the image: `simapp 1.0.0-1` installed,
  `simapp 2.0.0-1` available as the pending upgrade, download cache empty.
  [make-repo.sh:build_pkg]
- `sim-upgrade` accepts no arguments (exits 64 otherwise) and performs only
  `pacman -Syu --noconfirm`; no direct pacman argv is allowed through sudo. [sim-upgrade]
- `yay` rejects root on every path (exit 98) before any branch can succeed, and
  the AUR build path fails loudly (exit 97) rather than silently succeeding. [yay:EUID] [yay:exit 97]
- The sudoers rules are pinned exactly — `builder` may run only
  `/usr/bin/pacman -Syuw --noconfirm`, `installer` only
  `/usr/local/bin/sim-upgrade ""` — and each rule is validated with
  `visudo -cf` at image build time. [Dockerfile:visudo -cf]
- `pacman.conf` has no Arch mirrors; every package must come from the local
  `file://` repo, and `SigLevel = Never`. [pacman.conf]

## Side Effects

- `run.sh` builds the container image and runs the runtime containers via the
  selected docker/podman runtime; it writes nothing into the checkout (the
  image and its layers stay in the runtime's own store).
  [run.sh:build_image] [run.sh:RUNTIME]
- `make-repo.sh` removes and rebuilds `/srv/simrepo` and `/srv/simbuild`, installs
  `simapp` into the image, and clears `/var/cache/pacman/pkg/*`. [make-repo.sh:rm -rf]
- `yay` appends argv to `/var/log/sim-yay.log` and `/var/log/sim-yay-build.log`,
  and on an upgrade path execs `sudo -n /usr/local/bin/sim-upgrade`. [yay:log]
- `sim-upgrade` execs `pacman -Syu --noconfirm`, mutating the container's package
  database and `/usr`. [sim-upgrade:exec]
- `Dockerfile` runs `pacman -Syu` during image build, creates the `builder` and
  `installer` users, writes `/etc/sudoers.d/sim`, and pre-creates the
  `/var/log/sim-*.log` files world-writable. [Dockerfile:RUN pacman]
- `lib.sh` probes are read-only; `finish` prints `PASS`/`FAILED` to stdout/stderr
  and exits 1 when any assertion failed. [lib.sh:finish]

## Verification

- Run: `./sim/run.sh` (requires Docker or Podman; see `./README.md`).
- Rust build/lint/test: `cargo test` at the repository root.
