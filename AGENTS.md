# AGENTS.md

Rules for anyone (human or agent) changing this repo. They exist because this
tool changes pacman state, and because an earlier session burned a run printing
"Done" instead of stopping.

## Hard safety rules

1. Never run `pacman`, `yay`, `makepkg`, `aurto`, `paccache`, `pacdiff`, or
   `systemctl enable/start/restart` against the development host. The dev
   machine is not a test machine.
2. Exercise runtime behaviour only through `./sim/run.sh`: a disposable Arch
   container, `--network none`, local pacman repo, fake `yay`.
3. Never install a sudoers rule, timer, or service on the host while developing.
4. Do not commit key material, `.gpg/`, container images, or `target/`.
5. Invariants that must hold after every change:
   - `fetch` writes only to the pacman sync DB and the download cache. It never
     writes to `/usr`, `/etc`, or `/var/lib/pacman/local`.
   - `check` and `doctor` write nothing at all.
   - `install` never rebuilds a package that was already built.
   - Nothing runs `yay`/`makepkg` as root.

## Definition of done

- `./ci.sh` green: fmt, clippy with `-D warnings`, tests, cargo-deny.
- `./sim/run.sh` green.
- Any change that touches pacman behaviour has a scenario in `sim/scenarios/`.
  Unit tests with a fake runner are not enough for side-effect claims.
- `README.md` updated when a command, flag, or path changes.

## Honesty rules

- Never turn a gate into a silent skip that still prints success. If a check
  cannot run, fail loudly. `ci.sh` already does this for cargo-deny.
- Do not call stubbed behaviour "done". Say it is a stub and name what is
  missing.
- Do not pad a finished answer with repeated acknowledgements. One closing
  line, then stop.
- If blocked, state exactly what blocks you once. Do not loop.

## Layout

| Path | Role |
|---|---|
| `spec.md` | User-facing, plain-language flow and failure modes |
| `NEXT.md` | Working plan: decisions, phases, acceptance criteria |
| `context.md` | Implementation notes and edge cases |
| `src/main.rs` | CLI surface (subcommands) |
| `ci.sh` | Local mirror of the GitHub checks |
| `sim/` | Arch simulation harness (see `sim/README.md`) |
| `.github/workflows/ci.yml` | Build, lint, test, audit |
| `.github/workflows/sim.yml` | Arch container scenarios |

## Current status

All six subcommands are stubs. `NEXT.md` is the authoritative order of work.
`sim/` proves the harness works and enforces the read-only safety invariants;
positive assertions (fetch actually downloads, install actually installs) get
added with each command's implementation.
