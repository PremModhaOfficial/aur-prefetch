# sim: Arch simulation harness

A disposable Arch container that can run the CLI against a real pacman without
touching your machine.

`ci.sh` only proves the Rust compiles and lints. It cannot tell you whether
`fetch` upgraded the system by mistake. This harness can.

## Run it

```bash
./sim/run.sh                    # build image, run every scenario
./sim/run.sh --build-only       # just build
./sim/run.sh sim/scenarios/01-download-only-keeps-usr.sh
```

Requires Docker or Podman. The container runs with `--network none`.

After any source change, rebuild the image without the cache before running:

```bash
docker build --no-cache -f sim/Dockerfile -t aur-prefetch-sim:local .
```

The classic docker builder caches `COPY --from=builder`, so a plain
`./sim/run.sh` can silently test the previous binary. There is no buildx on the
current host, so `--no-cache` is the only reliable way to get a fresh image.

## What the image contains

- `archlinux:base-devel`
- A local pacman repo at `/srv/simrepo` served over `file://`
- `simapp 1.0.0-1` installed, `simapp 2.0.0-1` available as the pending update
- An empty download cache
- `builder`, a non-root user whose only passwordless sudo command is
  `/usr/bin/pacman -Syuw --noconfirm`
- `installer`, a sim-only user whose only sudoers rule is
  `/usr/local/bin/sim-upgrade ""`, so the command and an empty argument list
  are both pinned. The helper always performs exactly `pacman -Syu
  --noconfirm`; no direct pacman argv is allowed through sudo, and scenario 19
  proves the rule is exactly that narrow
- `/usr/local/bin/yay`, a fake helper that logs every call to
  `/var/log/sim-yay.log`, upgrades through `sudo -n sim-upgrade`, and fails
  with exit 97 if asked to build from the AUR or exit 98 if run as root
- `/usr/local/bin/sudo`, a proxy that logs the exact argv to
  `/var/log/sim-sudo.log` and then execs the real sudo, so a scenario can prove
  which escalations actually happened
- Stubs for `systemctl`, `journalctl`, `loginctl`, `aurto`, `paccache`, and
  `pacdiff`, plus `snapper` for the optional doctor checks. Each logs argv to
  `/var/log/sim-<tool>.log`; a scenario sets the
  reply with `/etc/sim/<tool>.out`, `.err`, and `.exit`. `paccache` forwards to
  the real tool; `pacdiff` never runs for real.
- Pacman config fixtures for default, multiple, empty, and moved `CacheDir`
  under `/usr/local/lib/sim/fixtures`
- A slow `XferCommand` fixture (`sim-xfer` plus `pacman-slow-xfer.conf`) that
  writes a package `.part` in two chunks and carries the optional `kill` and
  `fill` faults. Scenarios 34, 41, 43 and 46 install it. pacman still chooses
  the argv, verifies the package, and renames the final file
- `/usr/local/bin/make-pkg.sh`, which publishes an ad-hoc package with a blob
  of any size into the sim repo, for the chaos scenarios that need a payload
  worth interrupting
- `/usr/local/bin/aur-prefetch`, built inside the image so the glibc matches

The sim binary is built with the `sim-hooks` feature. It adds one env-gated
forced panic after the terminal guard is active, used to prove the panic hook
restores the pty. A packaged build does not enable the feature.

No mirrors, no keys, no network at scenario time. If a scenario needs another
package, add it to `sim/make-repo.sh`, do not open the network.

## Scenarios

| File | Proves |
|---|---|
| `01-download-only-keeps-usr.sh` | `pacman -Syuw` downloads `simapp-2.0.0` and leaves `/usr` byte-identical |
| `02-full-upgrade-changes-usr.sh` | `pacman -Syu` does change `/usr`, so the detector in 01 is not blind |
| `03-binary-readonly-safety.sh` | `check`/`doctor` write nothing, `fetch` never touches `/usr`, nothing triggers an AUR build |
| `04-sudo-contract.sh` | `builder` may run exactly `pacman -Syuw --noconfirm`; another argv and blanket sudo are refused |
| `05-fetch-download-only.sh` | `fetch` as `builder` downloads, records state, and leaves `/usr` and the package DB untouched |
| `06-fetch-lock-gate.sh` | a live pacman blocks with exit 1; a stale lock warns, proceeds, and is never deleted |
| `07-fetch-free-space-gate.sh` | a floor larger than the filesystem refuses before pacman runs |
| `08-stub-contract.sh` | stubs replay `.out`/`.err`/`.exit`, `builder` can append to logs, `paccache` honors a configured failure, and `yay` rejects root on every path |
| `09-check-pending.sh` | `check` exits 1, names the pending package, and writes nothing |
| `10-check-clean.sh` | `check` exits 0 and writes nothing when the system is current |
| `11-doctor-missing-setup.sh` | `doctor` exits 1 and names the timer, sudoers, trust, and repo problems |
| `12-doctor-configured.sh` | `doctor` exits 0 on a configured container, as root and as the unprivileged user reading the rule through `sudo -l` |
| `13-clean-dry-run.sh` | `clean` changes nothing without `--apply`; with it, only an installed package's old build is removed |
| `14-install-upgrades.sh` | `install --yes` upgrades `/usr` through yay and runs no AUR build |
| `15-install-gate.sh` | `install` without `--yes` on a non-TTY stops at the news gate |
| `16-init-setup.sh` | `init` renders without writing, applies with `visudo` approval, and never runs `aurto` |
| `17-install-noop.sh` | `install` on a current repo exits 0, changes nothing, and builds nothing |
| `18-yay-build-loud.sh` | a direct yay build and `install` both surface the AUR failure loudly; install exits 2 and does not report success |
| `19-installer-sudo-scope.sh` | `installer` may run only `/usr/local/bin/sim-upgrade` through sudo; direct pacman argv and helper arguments are refused |
| `20-init-as-builder.sh` | `init --apply` as `builder` escalates only `visudo`, `install`, and `mv` with the recorded argv, writes a `builder` rule, and refuses to run under `sudo`; the fixture grants no wildcard sudo |
| `21-doctor-wildcard-sudoers.sh` | a broader sudoers grant is not accepted as the exact rule |
| `22-doctor-low-disk.sh` | a disk floor larger than the filesystem makes `doctor` exit 1 |
| `23-doctor-snapper-timers.sh` | snapper without scheduled timers makes `doctor` exit 1 |
| `24-doctor-cache-permissions.sh` | a cache the `alpm` user cannot write makes `doctor` exit 1 |
| `25-check-missing-cache-dir.sh` | a missing `CacheDir` makes `check` exit 1 instead of failing on `df` |
| `26-install-root-refuses.sh` | `install` exits 1 with no command run and `/usr` unchanged for plain root and for the sudo shape (real UID user, effective UID root) |
| `27-doctor-journald-dropin.sh` | a `journald.conf.d` drop-in sets persistence and the size cap |
| `28-doctor-symlinked-cache.sh` | a symlinked `CacheDir` is resolved before checking `alpm` traversal |
| `29-init-visudo-root.sh` | root `visudo -cf` accepts the user-owned temp rule that `init` validates through sudo |
| `30-doctor-missing-aurto.sh` | an `Include` line whose aurto file is missing makes `doctor` exit 1 |
| `31-tui-readonly.sh` | `tui --render-once` is read-only, a pipe is rejected, and pty quit, Ctrl-C, forced panic, and Ctrl-Z all restore the terminal |
| `32-tui-install-gate.sh` | `i` opens the install gate, `space` attests the news for the session, `esc` cancels, and `/usr`, the package DB, the cache, the state file and the sudo log are untouched; `yay -Syuu` never runs |
| `33-tui-prefetch.sh` | the prefetch frame reads the real cache: missing, cached, stale, partial and built, with no writes |
| `34-tui-live-fetch.sh` | `F` runs the real fetch under a pty, shows the PRE-FLIGHT gates, opens the captured log with a real pacman line, shows real `.part` bytes and a bar, finishes from the final artifact, leaves `/usr` and the package DB unchanged, writes only the state file, and escalates only the allowed argv |
| `35-tui-failure-freeze.sh` | a corrupted repo artifact behind the paced transfer makes the real fetch fail: the running row freezes, the repair card names the evidence and the hint, `/usr` is unchanged, no state is written, and only the allowed argv is escalated |
| `36-tui-install-attached.sh` | the attested install gate runs the exact attached `yay -Syuu` path as the sim installer: the package upgrades, the scoped helper is the only escalation, no AUR build happens, the state file records the install, the terminal is restored, and the read-only post-install receipt shows the exit, kernel and pacnew state |
| `37-fetch-refuses-during-a-build.sh` | `fetch` refuses with exit 1 while any of `pacman`, `yay`, `makepkg`, `aurto` runs, even with no `db.lck`, and `check` reports the live process |
| `38-check-and-fetch-share-the-floor.sh` | `check` refuses on the same 5 GiB snapshot floor `fetch` refuses on, so its verdict matches fetch's |
| `39-fetch-state-unwritable.sh` | a download that completed exits 0 with a loud warning when the state file cannot be written, instead of turning the timer red |
| `41-fill-cache-mid-download.sh` | the `CacheDir` fills during the transfer (a 2 MiB tmpfs and a fault that writes until the kernel refuses): `fetch` exits 2, never claims success, leaves no final artifact and no lock, still delivers the lists, writes nothing to `/usr` or either DB, and completes on a retry after the user frees the space |
| `42-kill-mid-install.sh` | a file-size rlimit SIGKILLs pacman inside the extraction of a package upgrade: `install` exits 2 and names the killed command, the payload is truncated at the limit, the local DB records the new version with no file list, `pacman -Qk` calls that clean and `pacman -Qkk` reports size and checksum mismatches (it is the honest one), a retry exits 0 without repairing it, and clearing the broken entry plus `--overwrite` does |
| `43-delete-db-lck-under-pacman.sh` | `db.lck` is deleted under a live, paced `pacman -Syuw`: `check` and `fetch` still name the running pacman because their gate is the process list, and nothing else broke. **Pins a gap:** the last assertion says `doctor` does *not* name the lock, because it has no lock check; it goes red the day `doctor` gains one |
| `44-partial-upgrade-window.sh` | one package is installed by hand between fetch and install, so the system is half upgraded and the prefetched artifact for the rest is still cached: `check` names the pending update, reads `pacman.log`, names the partial upgrade and the evidence it read, and leaves everything read-only |
| `45-stale-aurto-foreground-build.sh` | a local aurto repo one version behind the AUR with an AUR update pending: `install` exits 0 and asks for the full upgrade. **Pins a gap:** the last assertion says `install` does *not* warn about the foreground build, because that sentence exists only in the TUI |
| `46-kill-mid-download.sh` | the paced transfer SIGKILLs pacman after the first half: the `.part` is a real prefix of the repo artifact, no final artifact exists, the lock and the refreshed lists are left behind, the next fetch refuses on the stale lock without removing it, and the retry after the user removes it fetches the real bytes |

41-46 are the Phase 4 chaos level from `NEXT.md`; they need the fault
injection below. 43, 44 and 45 pin a known gap rather than a fixed behaviour:
they assert the missing behaviour is *still* missing, so they pass today and go
red on the day someone implements it — which is when the assertion should be
inverted, in the same commit as the fix.

01 and 02 are a pair. Passing 01 alone would prove nothing if the harness could
not detect a real install at all.

## Adding a scenario

```bash
#!/usr/bin/env bash
source /usr/local/lib/sim/lib.sh

echo "scenario: what this proves"

assert_eq "$(installed_version)" "1.0.0-1" "still on the old version"

finish
```

Each scenario runs in a fresh container, so no cleanup is needed. Helpers live
in `sim/lib.sh`: `assert_eq`, `assert_ne`, `assert_contains`, `assert_true`,
`assert_false`, `installed_version`, `payload_version`, `pkg_state`,
`cache_files`, `usr_state`, `tree_state`, `local_db_state`, `sync_db_state`,
`yay_log`, `yay_built_something`, `sudo_log`, `finish`.

For a state that is large, assert on a digest (`local_db_state | sha256sum`),
not on the state itself: a failing `assert_eq` prints both sides, and the whole
local package DB is thousands of lines of noise.

## Fault injection

The chaos scenarios need a real pacman to misbehave, and the harness provides
three faults rather than a scenario faking the evidence:

- `sim/fixtures/sim-xfer` is the sim-only `XferCommand`. It paces a package
transfer (`/etc/sim/xfer-delay`, seconds) so a poll can see real `.part` growth,
and it injects one fault per run when a scenario writes one word to
`/etc/sim/xfer-fault`: `kill` SIGKILLs the running pacman after the first half
of the package, `fill` writes into the cache until the kernel refuses (capped by
`/etc/sim/xfer-fill-max`, default 64 MiB) and then tries to append the rest,
which is a real `ENOSPC`. The fault disarms itself, so the retry a scenario runs
next is a clean run.
- `# sim-run-args: <flags>` in a scenario's header comments passes extra
container flags to that one scenario. The `fill` fault needs a bounded
filesystem to fill, so scenario 41 asks for `--tmpfs
/var/cache/pacman/pkg:size=2m`. **A scenario that fills a disk must carry that
header**: without it the fill runs to the fill cap or, if the cap is raised, to
the container's root filesystem and the host's docker storage.
- `sim/make-pkg.sh NAME VERSION SIZE_BYTES` publishes an ad-hoc package with one
blob of `SIZE_BYTES` zero bytes into the sim repo, because the two `simapp`
versions are too small to interrupt. A `ulimit -f` in the invoking shell then
kills pacman mid-extraction (scenario 42).

## Limits

This is a real pacman with a synthetic repo. It does not simulate:

- systemd timers, journald, or linger
- the AUR build path or `makepkg`
- aurto's own repository and build chroots
- btrfs/snapper behaviour (a container has no snapshots)

Those need the VM level from `context.md`. The container level catches the
expensive mistakes: accidentally installing during prefetch, running `yay` as
root, deleting a lock while pacman runs, and treating a read-only command as
safe when it is not.

## Why the fake yay fails loudly

A silent AUR stub would make "install did not rebuild" pass for the wrong
reason. Exit 97 plus `/var/log/sim-yay-build.log` turns the gap into a visible
failure instead of false coverage.
