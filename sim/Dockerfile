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

# Replace mirrors with the offline sim repo. Done after the builder stage so
# only runtime has no network.
COPY sim/pacman.conf /etc/pacman.conf
COPY sim/yay /usr/local/bin/yay
COPY sim/make-repo.sh /usr/local/bin/make-repo.sh
COPY sim/lib.sh /usr/local/lib/sim/lib.sh
RUN chmod 755 /usr/local/bin/yay /usr/local/bin/make-repo.sh \
    && /usr/local/bin/make-repo.sh

COPY --from=builder /src/target/release/aur-prefetch /usr/local/bin/aur-prefetch

WORKDIR /root
