# RUN-09: Phase 6 packaging and `--json`

Thread D. Owns `--json` for `check`/`doctor`, the `PKGBUILD`, the
`cargo install` docs, and the release dry-run. Does not touch `src/tui/**`,
`sim/**`, the README TUI section, the keymap, or `--no-motion`.

Goal: ship the packaging path for a real Arch user without changing any
behaviour `check` and `doctor` already prove.

Ledger: `/tmp/aur-prefetch-runs/09/ledger.tsv`.

## Steps

1. **Freeze today's `doctor` text before touching it.** `doctor` prints with
   `println!` and has no golden text test; `check` already has two frozen text
   tests (`clean_text_is_frozen`, `pending_text_is_frozen`).
   Check: a temporary baseline test run under `--nocapture` writes
   `/tmp/aur-prefetch-runs/09/doctor-baseline.txt`; every line starts
   `doctor: `; line count is recorded. This file is the byte contract for
   step 3.

2. **`--json` output for `check`.** Add `serde`/`serde_json`, derive
   `Serialize` on the existing `report.rs` types (no new schema types), add
   `src/json.rs` with one wrapper object per command, and `--json` on `check`.
   Check: `cargo test json` passes; the object is one line, keys are asserted
   in a test, and the serialized text contains no ESC byte. A second test
   proves the exit code is unchanged: the JSON is printed and
   `run_with_options` still returns `Blocked` for an actionable snapshot and
   `Failed` for a fatal one.

3. **`--json` output for `doctor`, with its text rendered from the report.**
   `doctor` builds a `DoctorReport` of `Check` records; the text renderer
   prints `doctor: {label}: {detail}` plus the summary; `--json` prints the
   object. Collection errors still print every line collected before them and
   still exit 2.
   Check: a new frozen text test plus `diff` against the step 1 baseline is
   empty; `check`'s two frozen tests are unchanged and pass.

4. **`PKGBUILD` for the AUR.** Builds with cargo, installs the binary to
   `/usr/bin` and the two existing user units to `/usr/lib/systemd/user`.
   Depends on `pacman`, optional `aurto`/`yay`. No new unit, no manpage.
   Check: `bash -n PKGBUILD`; a test in `init`'s own test module proves the
   packaged timer is byte-equal to `render_timer()` and the packaged service
   runs `/usr/bin/aur-prefetch fetch`, so the units cannot drift. `makepkg` is
   not run on this host (rules); the exact container command is handed to the
   human.

5. **`cargo install` path in the README,** in the existing voice, plus the
   `--json` flags and the AUR install route.
   Check: `grep` shows the `cargo install` and AUR blocks; the TUI section is
   untouched (`git diff` shows no hunk under `### tui`).

6. **Release check: `cargo publish --dry-run`.** A script and a CI job. The
   dry run needs no token, so nothing passes silently; the CI job fails if the
   command fails.
   Check: `scripts/publish-dry-run.sh` green locally; `cargo package --list`
   reviewed for the tarball contents.

7. **Close out.** `./ci.sh` green at head; `./sim/run.sh` green or the reason
   it could not run is stated with the command a human must run.

## Constraints carried from the brief

- Never run `pacman`, `yay`, `makepkg`, `aurto`, `paccache`, `pacdiff`,
  `systemctl` against this host. No `cargo publish` without `--dry-run`.
- No writes to `/usr`, `/etc`, `/var/lib/pacman/local`.
- One item per commit, conventional message, no push, no PR.
- `./ci.sh` green before every commit; baseline 276 tests.
- Tick the matching `NEXT.md` Phase 6 boxes in the same commit.
