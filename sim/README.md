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
- `/usr/local/bin/yay`, a fake helper that logs every call to
  `/var/log/sim-yay.log` and fails with exit 97 if asked to build from the AUR
- `/usr/local/bin/aur-prefetch`, built inside the image so the glibc matches

No mirrors, no keys, no network at scenario time. If a scenario needs another
package, add it to `sim/make-repo.sh`, do not open the network.

## Scenarios

| File | Proves |
|---|---|
| `01-download-only-keeps-usr.sh` | `pacman -Syuw` downloads `simapp-2.0.0` and leaves `/usr` byte-identical |
| `02-full-upgrade-changes-usr.sh` | `pacman -Syu` does change `/usr`, so the detector in 01 is not blind |
| `03-binary-readonly-safety.sh` | `check`/`doctor` write nothing, `fetch` never touches `/usr`, nothing triggers an AUR build |

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
`usr_state`, `yay_log`, `yay_built_something`, `finish`.

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
