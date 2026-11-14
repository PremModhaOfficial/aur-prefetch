#!/usr/bin/env bash
# A stale aurto repo with an AUR update pending: the install that follows would
# build that package in the foreground, and the user has to be told.
#
# aurto builds AUR packages into a local pacman repo; the point of the whole
# prefetch/install split is that the slow part happens off the terminal. When
# the local repo is behind what the AUR offers, `yay -Syuu` cannot satisfy the
# pending AUR package from it and builds instead - in the foreground, with a
# PKGBUILD nobody has read and the terminal held for as long as the build takes.
#
# The harness makes that state real rather than mocking it: a local aurto repo
# holding auronly 1.0.0, an AUR list offering 2.0.0. The tool sees the AUR
# update - `check` counts it - so the fact is available to `install` too. Only
# the TUI says anything about it ("If an AUR package builds now, aurto is
# stale"), `src/tui/render.rs:425`; the CLI never does. That last assertion is
# the deliverable and fails on purpose until `install` carries the warning.
source /usr/local/lib/sim/lib.sh

echo "scenario: a stale aurto repo is a foreground build risk install never mentions"

pkgname=auronly
repo=/srv/aurto-repo
cache=/var/cache/pacman/pkg

# The local AUR repo, one version behind what the AUR offers. make-pkg.sh
# publishes into the sim repo; the artifact is then copied into a repo of its
# own, which is what aurto would have built.
make-pkg.sh "$pkgname" 1.0.0 4096 >/dev/null
mkdir -p "$repo"
cp "/srv/simrepo/$pkgname-1.0.0-1-x86_64.pkg.tar.zst" "$repo/"
repo-add "$repo/aurto.db.tar.gz" "$repo/$pkgname-1.0.0-1-x86_64.pkg.tar.zst" >/dev/null
printf '[aurto]\nServer = file://%s\n' "$repo" >/etc/pacman.d/aurto
printf 'Include = /etc/pacman.d/aurto\n' >>/etc/pacman.conf

# The AUR itself offers a newer version, so the local repo is stale.
printf '%s 1.0.0-1 -> 2.0.0-1\n' "$pkgname" >/etc/sim/yay-qua.out

# A configured trust file, so the install below is gated only by --yes and the
# warning under test is the one this scenario is about.
mkdir -p /etc/aurto
printf 'maintainer\n' >/etc/aurto/trusted-users

pacman -Syy --noconfirm >/dev/null

# The stale state, asserted rather than assumed: the repo pacman reads has the
# old version, and the tool's own view of the AUR has the new one.
assert_eq "$(pacman -Sl aurto | awk '{print $2, $3}')" "$pkgname 1.0.0-1" \
    "the local aurto repo only holds the old version"
check_log=$(mktemp)
runuser -l builder -c 'aur-prefetch check' >"$check_log" 2>&1
assert_contains "$(cat "$check_log")" "check: aur updates: 1" \
    "check sees the pending AUR update, so install can see it too"
assert_eq "$(installed_version)" "1.0.0-1" "nothing has been installed yet"

# The install. It is legal and it succeeds: the risk is not a failure, it is
# that a build may now hold the terminal and nothing said so.
log=$(mktemp)
runuser -l installer -c 'aur-prefetch install --yes' >"$log" 2>&1
status=$?

assert_eq "$status" "0" "install exits 0"
assert_contains "$(yay_log)" "-Syuu" "install asked yay for the full upgrade"

# GAP: the sentence exists only in the TUI; the CLI has no aurto staleness gate.
# When it gains one, this line goes red on purpose — invert it in the same commit.
assert_false "install still does not report a stale aurto repo" \
    grep -qi "aurto is stale" "$log"

# The other half of the claim: no build happened here, because the simulated
# helper cannot build. That is exactly why the warning is the only protection.
assert_false "the simulated helper ran no AUR build" yay_built_something
assert_eq "$(grep -vc '^-n /usr/local/bin/sim-upgrade$' /var/log/sim-sudo.log)" "0" \
    "the scoped helper is the only escalation"

finish
