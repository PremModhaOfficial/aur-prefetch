# HANDOFF.md

Written 2026-11-07. Orientation for a fresh agent. The plan itself is
[`NEXT.md`](NEXT.md), the rules are [`AGENTS.md`](AGENTS.md). This file only
covers where things stand and the first move. Do not re-plan.

## Where things stand

- Repo: `PremModhaOfficial/aur-prefetch`, branch `main`, clean at `420b842`
  before this file. This handoff is the commit after it.
- `CI` and `Sim` are green on GitHub.
- Six subcommands exist as stubs. Nothing has run against a real Arch system.
- **No Phase 1 code has been written yet.** The scaffold is the next task.

## Do this first: Phase 1 scaffolding

Read `NEXT.md` Phase 1 for the acceptance criteria. Then, in this order:

1. `src/runner.rs`: `CommandRunner` trait, `SystemRunner`, `FakeRunner`.
   Every external command must go through it. No `Command::new` elsewhere.
2. `src/error.rs`: `Blocked` maps to exit 1, `Failed` maps to exit 2.
3. `src/config.rs`: paths with env overrides for pacman, pacman.conf, CacheDir,
   trusted-users, state file, min free bytes.
4. `src/state.rs`: `last_fetch_unix`, `last_fetch_pending`,
   `last_install_unix`. Atomic write, manual parse, no new dependencies.
5. Split `src/lib.rs` from `src/main.rs`. Move the six stubs into
   `src/commands/{fetch,check,install,clean,doctor,init}.rs`.
6. Add `description`, `repository`, `keywords`, `categories` to `Cargo.toml`.

Then run both gates, tick the Phase 1 boxes in `NEXT.md`, commit, push.

One naming detail: `NEXT.md` suggests bare env names like `PACMAN_CONF`. Prefer
the `AUR_PREFETCH_` prefix in code to avoid clobbering system variables, and
update `NEXT.md` to match in the same commit.

## Hard rules

Read `AGENTS.md`. The short version:

- Never run `pacman`, `yay`, `makepkg`, `aurto`, `paccache`, or `pacdiff`
  against the development host.
- All runtime behaviour is tested through `./sim/run.sh`, a disposable Arch
  container with `--network none`.
- `fetch` never writes `/usr`. `check` and `doctor` write nothing.
- No skipped gates that still print success.
- A command is done only when a sim scenario proves its side effects.

## Decisions already made

`NEXT.md` Phase 0 has D1 to D6. They are locked with the recommended values.
Do not re-litigate unless the user asks. The one worth restating:

- The trust gate belongs in `install`, not `fetch`. `fetch` only handles
  official repo packages.
- Missing `/etc/aurto/trusted-users` means aurto trusts everyone. Empty means
  trust no one. Non-empty is an allowlist.

Order of implementation: `fetch`, `check`, `doctor`, `clean`, `install`,
`init`.

## Environment facts

- Host is Arch, pacman 7.1.0, `DownloadUser = alpm`.
- `yay` 13.1.0 is installed. `aurto` 0.14.7 exists upstream but is not
  installed here.
- `rust-toolchain.toml` pins 1.85. `cargo-deny` 0.20.2 is in `~/.cargo/bin`,
  which may not be on `PATH`.
- Docker is available. `archlinux:base-devel` is already pulled.
- Sim image build is about 60 seconds cold, scenarios about 2 seconds.

## Pitfalls already hit, do not repeat

- `repo-add` can land in the same second as `pacman -Sy`, and pacman then skips
  the `file://` database. `make-repo.sh` uses `-Syy` for this reason.
- An empty `.gpg/` directory appeared from a tool run. It is removed and
  `/.gpg/` is ignored.
- GitHub reports the license as Apache-2.0 because the repo is dual licensed.
  That is expected, not a mistake.
- The sim currently runs as root, so the real `sudo -n` path is untested.
  Phase 2 fixes that.

## Verify before handing back

```bash
cd ~/projects/aur-prefetch
./ci.sh          # fmt, clippy -D warnings, tests, cargo-deny
./sim/run.sh     # Arch container scenarios
git status       # expect clean
```

For `cargo-deny`, prepend `PATH="$HOME/.cargo/bin:$PATH"` if needed.

## Pointers

| File | Why |
|---|---|
| `NEXT.md` | The only plan. Phase order, decisions, acceptance criteria |
| `AGENTS.md` | Rules, definition of done, honesty rules |
| `spec.md` | User-facing flow, the five ways to break an Arch box |
| `context.md` | Edge cases, not a plan |
| `sim/README.md` | Harness layout and how to add a scenario |

## Suggested skills for the next agent

- `/implement` with `NEXT.md` Phase 1 as the ticket.
- `/tdd` for `runner`, `config`, and `state` unit tests.
- `/code-review` before the first commit.
- `/handoff` again at the end of the session.

## Note on T3 delegation

The previous session had no T3 orchestration tools exposed: no
`delegate_task`, no `orchestrator_capabilities`, and no ACP bridge environment.
No agents were spawned and no work was delegated. If the next session has those
tools, the clean split is `src/` scaffolding in one lane and `sim/` upgrades in
another, because the file sets do not overlap.
