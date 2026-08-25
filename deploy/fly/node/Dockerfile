# syntax=docker/dockerfile:1
#
# Workspacer headless worker node for Fly.io.
#
# Build context is the REPO ROOT:
#   fly deploy --config deploy/fly/node/fly.toml \
#              --dockerfile deploy/fly/node/Dockerfile .
#
# ---------------------------------------------------------------------------
# WHY DEBIAN AND NOT ALPINE
# ---------------------------------------------------------------------------
# tailscaled runs here in KERNEL mode (Fly Machines are Firecracker microVMs
# with a real kernel, so there is no need for --tun=userspace-networking). In
# kernel mode tailscaled programs routes and firewall rules through
# nftables/iptables, and Tailscale's own Fly guide warns that on Alpine "you
# may need to update the machine to ensure that it has kernel support for
# nftables". That is the one realistic way kernel-mode tailscaled fails
# confusingly on Fly, and a Debian base makes it a non-event: the iptables and
# nftables packages below are ordinary, tested, nft-backed Debian packages.
#
# Debian also buys glibc, which matters for the rest of the workload: Ruby
# native gems (sqlite3, nokogiri) ship prebuilt glibc binaries, and Rust and Go
# both want a normal libc for cgo/`bundled` sqlite. On musl every one of those
# becomes a source build.
#
# trixie (Debian 13) rather than bookworm because bookworm ships Ruby 3.1 and
# Rails 8 needs >= 3.2. trixie ships Ruby 3.3.
#
# ---------------------------------------------------------------------------
# THE LAYER BOUNDARY
# ---------------------------------------------------------------------------
#   image  = toolchains + daemons + system packages.   Changes monthly.
#   volume = repos, every package/build cache, every credential, $HOME.
#            Changes constantly.
#
# One hard constraint follows from $HOME living on the volume: NOTHING in this
# image may install into $HOME. A global npm package or a bun install under
# ~/.bun would be shadowed the instant the volume mounts. Everything below
# installs to /usr/local.

# ---------------------------------------------------------------------------
FROM golang:1.25-trixie AS gobuild
# services/hub/go.mod requires go 1.25.4. No cgo anywhere in the module graph,
# so these link static and copy cleanly into the runtime image.
WORKDIR /src/hub
COPY services/hub/go.mod services/hub/go.sum ./
RUN --mount=type=cache,target=/go/pkg/mod go mod download
COPY services/hub/ ./
ENV CGO_ENABLED=0
RUN --mount=type=cache,target=/go/pkg/mod \
    --mount=type=cache,target=/root/.cache/go-build \
    mkdir -p /out && \
    go build -trimpath -o /out/brain ./cmd/brain && \
    go build -trimpath -o /out/workspacer ./cmd/workspacer

# ---------------------------------------------------------------------------
FROM rust:1-trixie AS rustbuild
# rusqlite is `features = ["bundled"]`, so this stage needs a C compiler; the
# rust image already has build-essential. reqwest is rustls-only, so no
# OpenSSL headers are needed.
WORKDIR /src/claudemon
COPY services/claudemon/Cargo.toml services/claudemon/Cargo.lock services/claudemon/build.rs ./
COPY services/claudemon/src ./src
COPY services/claudemon/tests ./tests
RUN --mount=type=cache,target=/usr/local/cargo/registry \
    --mount=type=cache,target=/src/claudemon/target \
    mkdir -p /out && \
    cargo build --release --bin claudemon && \
    cp target/release/claudemon /out/claudemon

# ---------------------------------------------------------------------------
FROM node:22-trixie-slim AS runtime

ARG WKS_UID=10001
ARG WKS_GID=10001

# iptables + nftables: kernel-mode tailscaled. iproute2: `ip link show
# tailscale0`, the thirty-second check that kernel mode actually came up.
# busybox-static: the wake doorbell. util-linux: setpriv, to drop privilege.
# tini: PID 1 that reaps the grandchildren agents leave behind.
#
# Versions are deliberately unpinned: this image is rebuilt to PICK UP Debian
# security updates, and pinned Debian versions break the build the moment the
# archive rotates a point release. Reproducibility comes from the base image
# digest, not from apt pins.
# hadolint ignore=DL3008
RUN apt-get update && \
    apt-get install -y --no-install-recommends \
      ca-certificates curl git gh openssh-client \
      iptables nftables iproute2 \
      busybox-static util-linux tini procps psmisc \
      ripgrep jq less unzip rsync \
      build-essential pkg-config sqlite3 libsqlite3-dev \
      ruby-full ruby-dev bundler \
      python3 && \
    rm -rf /var/lib/apt/lists/*

# The Go toolchain the node's own agents build with, lifted from the builder
# stage rather than downloaded again. Same Debian, same libc.
COPY --from=gobuild /usr/local/go /usr/local/go

# Tailscale, pinned by tag. `stable` tracks their releases; pin a concrete
# version (e.g. tailscale/tailscale:v1.86.0) if you want reproducible images.
COPY --from=docker.io/tailscale/tailscale:stable /usr/local/bin/tailscaled /usr/local/bin/tailscaled
COPY --from=docker.io/tailscale/tailscale:stable /usr/local/bin/tailscale  /usr/local/bin/tailscale

# Bun, installed to /usr/local — NOT ~/.bun, which the volume would shadow.
# Downloaded to a file rather than piped into bash so a failed fetch fails the
# build instead of feeding an empty script to a shell that exits 0.
ENV BUN_INSTALL=/usr/local
RUN curl -fsSL https://bun.sh/install -o /tmp/bun-install.sh && \
    bash /tmp/bun-install.sh && \
    rm -f /tmp/bun-install.sh && \
    bun --version

# Claude Code. Global npm prefix is /usr/local in the node image, so this lands
# in /usr/local/lib/node_modules and survives the volume mount.
#
# hadolint ignore=DL3016
# Unpinned by default on purpose — a worker node running a months-old Claude
# Code is its own bug class. Pin it for a reproducible image with
# `--build-arg CLAUDE_CODE_VERSION=1.2.3`.
ARG CLAUDE_CODE_VERSION=latest
RUN npm install -g "@anthropic-ai/claude-code@${CLAUDE_CODE_VERSION}" && \
    claude --version

COPY --from=gobuild   /out/brain      /usr/local/bin/brain
COPY --from=gobuild   /out/workspacer /usr/local/bin/workspacer
COPY --from=rustbuild /out/claudemon  /usr/local/bin/claudemon

COPY deploy/fly/node/entrypoint.sh  /usr/local/lib/wks/entrypoint.sh
COPY deploy/fly/node/bootstrap.sh   /usr/local/lib/wks/bootstrap.sh
COPY deploy/fly/node/test-bootstrap.sh /usr/local/lib/wks/test-bootstrap.sh
RUN chmod 0755 /usr/local/lib/wks/*.sh

# The agent user. Non-root deliberately: Claude Code refuses
# --dangerously-skip-permissions when euid is 0, and this fleet's full-access
# grants depend on that flag. -M because $HOME is on a volume that does not
# exist until the machine boots; the entrypoint creates and chowns it.
# -l / --no-log-init: without it a five-figure uid makes useradd allocate a
# sparse lastlog that some storage drivers materialise in full.
RUN groupadd -g "${WKS_GID}" wks && \
    useradd -l --no-log-init -u "${WKS_UID}" -g "${WKS_GID}" -M -d /data/home -s /bin/bash wks

# PATH only. Every stateful variable (HOME, XDG_*, GOPATH, the caches) is set
# by the entrypoint and by fly.toml [env], never here — a Dockerfile ENV would
# also apply during `docker build`, where /data does not exist.
ENV PATH="/usr/local/go/bin:/usr/local/bin:/usr/local/sbin:/usr/sbin:/usr/bin:/sbin:/bin"

EXPOSE 8080
ENTRYPOINT ["/usr/bin/tini", "--", "/usr/local/lib/wks/entrypoint.sh"]
