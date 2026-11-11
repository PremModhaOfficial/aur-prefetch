# Maintainer: the aur-prefetch maintainers
# AUR package: https://aur.archlinux.org/packages/aur-prefetch
#
# This PKGBUILD ships the same binary `cargo install` builds and the same two
# user units `aur-prefetch init` writes, system-wide under
# /usr/lib/systemd/user so a fresh account gets `aur-prefetch.timer`
# without running init first. Enabling the timer is still a user decision:
#   systemctl --user daemon-reload && systemctl --user enable --now aur-prefetch.timer
# `aur-prefetch init --apply` remains the path that also installs the sudoers
# rule and points the unit at a $HOME binary instead.

pkgname=aur-prefetch
pkgver=0.1.0
pkgrel=1
pkgdesc='Split Arch updates: download in the background, install in the foreground'
arch=('x86_64' 'aarch64')
url='https://github.com/PremModhaOfficial/aur-prefetch'
license=('MIT' 'Apache-2.0')
depends=('pacman' 'glibc' 'gcc-libs')
makedepends=('cargo' 'git')
optdepends=(
  'aurto: build AUR packages in a clean chroot before install'
  'yay: AUR update list and the foreground install step'
)
# The tag must exist before this PKGBUILD is pushed to the AUR; makepkg pins
# the commit it resolves. SKIP is the AUR norm for a git source, which has no
# reusable tarball checksum.
source=("git+$url.git#tag=v$pkgver")
sha256sums=('SKIP')

build() {
  cd "$pkgname"
  # --locked: the committed Cargo.lock is the dependency set this release was
  # tested with. The `sim-hooks` feature is test-only and never enabled here.
  cargo build --release --locked
}

package() {
  cd "$pkgname"
  install -Dm755 target/release/aur-prefetch "$pkgdir/usr/bin/aur-prefetch"
  install -Dm644 packaging/aur-prefetch.service \
    "$pkgdir/usr/lib/systemd/user/aur-prefetch.service"
  install -Dm644 packaging/aur-prefetch.timer \
    "$pkgdir/usr/lib/systemd/user/aur-prefetch.timer"
  install -Dm644 LICENSE-MIT "$pkgdir/usr/share/licenses/$pkgname/LICENSE-MIT"
  install -Dm644 LICENSE-APACHE "$pkgdir/usr/share/licenses/$pkgname/LICENSE-APACHE"
}
