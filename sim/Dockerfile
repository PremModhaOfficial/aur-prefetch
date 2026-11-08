# syntax=docker/dockerfile:1
#
# Simulation image: a disposable Arch system with a local pacman repo and a
# fake AUR helper. Scenarios run with --network none, so nothing here can reach
# the internet or the host.

FROM archlinux:base-devel AS builder
# Rust from the Arch repo, not rustup: the point is a binary that matches the
# container's glibc.
# -Syu, not -Sy: the base image can be weeks old, and a partial upgrade in a
# rolling-release distro is exactly the failure this project documents.
RUN pacman -Syu --noconfirm --needed rust
WORKDIR /src
# Manifest first so dependency compilation survives src-only changes.
COPY Cargo.toml Cargo.lock ./
COPY src ./src
RUN cargo build --release --locked

FROM archlinux:base-devel

# Phase 2: a non-root user with exactly one NOPASSWD command. No blanket sudo,
# no wildcards. The visudo check makes a malformed rule fail the build, not a
# scenario.
RUN pacman -Syu --noconfirm --needed sudo pacman-contrib \
    && useradd --create-home --shell /bin/bash builder \
    && printf 'builder ALL=(root) NOPASSWD: /usr/bin/pacman -Syuw --noconfirm\n' > /etc/sudoers.d/sim \
    && chmod 0440 /etc/sudoers.d/sim \
    && visudo -cf /etc/sudoers.d/sim \
    && mkdir -p /etc/sim \
    && touch /var/log/sim-yay.log /var/log/sim-yay-build.log \
        /var/log/sim-systemctl.log /var/log/sim-journalctl.log \
        /var/log/sim-loginctl.log /var/log/sim-aurto.log \
        /var/log/sim-paccache.log /var/log/sim-pacdiff.log \
        /var/log/sim-sudo.log \
    && chmod 0666 /var/log/sim-*.log

# Sim-only: the install scenario needs yay's one escalation to succeed without
# a password prompt. Scope it to the exact argv the fake yay passes, with no
# wildcard, so a regression cannot escalate anything else. Scenario 19 asserts
# that no other command is accepted. `init` never writes this rule.
RUN useradd --create-home --shell /bin/bash installer \
    && printf 'installer ALL=(root) NOPASSWD: /usr/bin/pacman -Syu --noconfirm\n' >> /etc/sudoers.d/sim \
    && visudo -cf /etc/sudoers.d/sim

# Replace mirrors with the offline sim repo. Done after the builder stage so
# only runtime has no network.
COPY sim/pacman.conf /etc/pacman.conf
COPY sim/yay /usr/local/bin/yay
COPY sim/make-repo.sh /usr/local/bin/make-repo.sh
COPY sim/lib.sh /usr/local/lib/sim/lib.sh
COPY sim/stubs/ /usr/local/bin/
COPY sim/fixtures /usr/local/lib/sim/fixtures
RUN chmod 755 /usr/local/bin/yay /usr/local/bin/make-repo.sh \
        /usr/local/bin/systemctl /usr/local/bin/journalctl \
        /usr/local/bin/loginctl /usr/local/bin/aurto \
        /usr/local/bin/paccache /usr/local/bin/pacdiff \
        /usr/local/bin/sudo \
    && /usr/local/bin/make-repo.sh

COPY --from=builder /src/target/release/aur-prefetch /usr/local/bin/aur-prefetch

WORKDIR /root
