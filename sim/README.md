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
- `/usr/local/bin/aur-prefetch`, built inside the image so the glibc matches

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
`assert_false`, `installed_version`, `payload_version`, `cache_files`,
`usr_state`, `yay_log`, `yay_built_something`, `sudo_log`, `finish`.

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
