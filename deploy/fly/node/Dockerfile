# syntax=docker/dockerfile:1
#
# ===========================================================================
#  workspacer node BASE image — workspacer, and nothing project-specific.
# ===========================================================================
#
# This image is the answer to "how do I run a workspacer worker node". It
# contains the workspacer daemons, Tailscale, Claude Code, the entrypoint and
# bootstrap, and the `wks` user — and it deliberately contains NO language
# toolchain for anybody's application code. It changes when workspacer changes.
#
# It is also a BASE that other people's Dockerfiles build `FROM`. That makes it
# a contract, not an internal seam, and the contract is written down in
# BASE_IMAGE.md next to this file. Read that before changing anything visible
# from outside: the binary paths, the uid/gid, PATH, the entrypoint, or what
# the image expects to find on the volume.
#
# Build context is the REPO ROOT:
#
#   docker build -f deploy/fly/node/Dockerfile -t workspacer-node-base:dev .
#
# and it also deploys as-is, as a workspacer node with no project toolchains:
#
#   fly deploy --config deploy/fly/node/fly.toml \
#              --dockerfile deploy/fly/node/Dockerfile .
#
# ---------------------------------------------------------------------------
# WHY THE SPLIT
# ---------------------------------------------------------------------------
# Before the split this file also installed Go, Ruby, bundler, bun, python3,
# sqlite and build-essential. That meant adding a Rails gem or a new language
# invalidated a layer above the Go and Rust BUILDER stages, so every dependency
# bump recompiled brain, workspacer, mcp and claudemon from scratch.
#
# Now: this file is workspacer. A downstream image adds toolchains. Bumping Go
# for one project no longer rebuilds the hub, and — the more important half —
# the toolchain list is no longer workspacer's business to carry.
#
# See BASE_IMAGE.md § "Extending this image" for the downstream shape.
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
# Debian also buys glibc, which matters for what downstream layers will add:
# Ruby native gems (sqlite3, nokogiri) ship prebuilt glibc binaries, and Rust
# and Go both want a normal libc for cgo and for `bundled` sqlite. On musl
# every one of those becomes a source build. A downstream author inherits that
# for free and should not have to know why.
#
# trixie (Debian 13) rather than bookworm because bookworm ships Ruby 3.1 and
# Rails 8 needs >= 3.2. trixie ships Ruby 3.3. No Ruby is installed HERE any
# more — but the trixie choice is what makes a downstream `apt-get install
# ruby-full` correct, so the reason has to stay with the base that pins the
# distro.
#
# ---------------------------------------------------------------------------
# THE LAYER BOUNDARY
# ---------------------------------------------------------------------------
#   base image     = workspacer daemons + system packages.  Changes with workspacer.
#   project image  = language toolchains.                   Changes with dependencies.
#   volume         = repos, every package/build cache, every credential, $HOME.
#                    Changes constantly.
#
# One hard constraint follows from $HOME living on the volume: NOTHING in
# EITHER image may install into $HOME. A global npm package or a bun install
# under ~/.bun would be shadowed the instant the volume mounts. Everything
# installs to /usr/local.
#
# That rule used to be a comment in the one file that had to obey it. Now a
# different file, in a different repo, has to obey it too — written by someone
# who has not read this. So it is also an executable check: verify-image.sh,
# shipped into this image at /usr/local/lib/wks/verify-image.sh, run as the
# last RUN of this build AND required as the last RUN of every downstream
# build. It turns "the tool silently vanishes on the first wake" into a build
# error naming the directory.

# ---------------------------------------------------------------------------
# TWO WAYS TO GET THE DAEMONS IN
# ---------------------------------------------------------------------------
# WKS_INSTALL=source (the DEFAULT, and unchanged) compiles brain, workspacer,
# mcp and claudemon from this working tree, below. It is what preflight.sh runs,
# what a dev iterating on the daemons wants, and the only mode that can build a
# commit that was never released.
#
# WKS_INSTALL=artifact downloads `workspacer-server-<platform>.tar.gz` from a
# GitHub release and installs the SAME files to the SAME paths — no Go stage, no
# Rust stage, no toolchain images pulled at all. BuildKit only builds stages the
# final one actually depends on, and the artifact stage depends on neither
# builder, so that saving is real rather than aspirational: on a box deploy it
# turns ~10 minutes of compiling into ~20 seconds of downloading.
#
#   docker build -f deploy/fly/node/Dockerfile \
#     --build-arg WKS_INSTALL=artifact \
#     --build-arg WKS_RELEASE_TAG=nightly \
#     --build-arg WKS_RELEASE_SHA=$(gh release view nightly --json targetCommitish -q .targetCommitish) \
#     -t workspacer-node-base:nightly .
#
# WKS_RELEASE_SHA is optional and worth passing: `nightly` is a MUTABLE tag,
# rolled every night, so naming it is not naming a commit. Either way the build
# FAILS LOUDLY if the bundle's own build-stamp disagrees with what was asked for
# — see deploy/fly/fetch-release.sh, which is where that argument is written out.
#
# Declared before the first FROM because only a global ARG can be interpolated
# into a FROM line. Each stage that uses one re-declares it locally.
ARG WKS_INSTALL=source
ARG WKS_RELEASE_TAG=nightly
ARG WKS_RELEASE_SHA=
ARG WKS_RELEASE_REPO=DJTouchette/workspacer
ARG WKS_RELEASE_ASSET=workspacer-server-linux-x64.tar.gz
ARG WKS_RELEASE_BASE_URL=
# Source mode has no release to name, so the honest identity of a source-built
# image is the commit it was built from. Nothing in the build context can supply
# it (`**/.git` is in Dockerfile.dockerignore, deliberately), so the caller must:
# preflight.sh passes `git rev-parse HEAD`. Left unset it stamps `unknown`, which
# is at least not a lie.
ARG WKS_SOURCE_SHA=

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
    go build -trimpath -o /out/workspacer ./cmd/workspacer && \
    go build -trimpath -o /out/mcp ./cmd/mcp

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
# THE TWO ARMS. Both must end with the same file names under /out, so the
# runtime stage's COPY lines below never learn which one ran.
# ---------------------------------------------------------------------------
# The source arm. `FROM golang:1.25-trixie` and not a slim Debian purely because
# source mode has already pulled this image — a second base image here would add
# a download to the mode that is meant to be the cheap-and-local one. It compiles
# nothing; it collects, and writes the stamp.
FROM golang:1.25-trixie AS binaries-source
ARG WKS_SOURCE_SHA
COPY --from=gobuild   /out/brain      /out/brain
COPY --from=gobuild   /out/workspacer /out/workspacer
COPY --from=gobuild   /out/mcp        /out/mcp
COPY --from=rustbuild /out/claudemon  /out/claudemon
COPY deploy/fly/write-build-stamp.sh /usr/local/bin/write-build-stamp.sh
RUN WKS_STAMP_COMPONENT=server WKS_STAMP_INSTALL=source \
    WKS_STAMP_COMMIT="${WKS_SOURCE_SHA}" WKS_STAMP_PLATFORM=linux-x64 \
    bash /usr/local/bin/write-build-stamp.sh /out/build-stamp

# The artifact arm. Debian slim rather than the golang image on purpose: this
# arm exists to need NO toolchain, and inheriting a 900MB one to run curl would
# undercut the only thing it is for.
#
FROM debian:trixie-slim AS binaries-artifact
ARG WKS_RELEASE_TAG
ARG WKS_RELEASE_SHA
ARG WKS_RELEASE_REPO
ARG WKS_RELEASE_ASSET
ARG WKS_RELEASE_BASE_URL
# Unpinned, per base rule 5: this stage is rebuilt to pick up Debian security
# updates, and a pinned version breaks the moment the archive rotates.
# hadolint ignore=DL3008
RUN apt-get update && \
    apt-get install -y --no-install-recommends ca-certificates curl && \
    rm -rf /var/lib/apt/lists/*
COPY deploy/fly/fetch-release.sh /usr/local/bin/fetch-release.sh
# The token is a BuildKit SECRET and not a build ARG: an ARG is recorded in the
# image history for anyone who pulls the image. `required=false` keeps the public
# anonymous download — the normal case — working with no --secret flag at all.
RUN --mount=type=secret,id=gh_token,required=false \
    WKS_RELEASE_TAG="${WKS_RELEASE_TAG}" \
    WKS_RELEASE_SHA="${WKS_RELEASE_SHA}" \
    WKS_RELEASE_REPO="${WKS_RELEASE_REPO}" \
    WKS_RELEASE_ASSET="${WKS_RELEASE_ASSET}" \
    WKS_RELEASE_BASE_URL="${WKS_RELEASE_BASE_URL}" \
    WKS_RELEASE_TOKEN_FILE=/run/secrets/gh_token \
    WKS_RELEASE_REQUIRE="workspacer brain claudemon mcp build-stamp" \
    WKS_RELEASE_CHMOD="workspacer hub brain claudemon mcp" \
    bash /usr/local/bin/fetch-release.sh /out

# The "untagged image" warning is a false positive on a stage alias: both arms
# resolve to stages defined above, not to registry references. (Same trick, and
# the same suppression, as the hub image's webbuild-${WKS_WITH_WEBAPP}.)
# hadolint ignore=DL3006
FROM binaries-${WKS_INSTALL} AS binaries

# ---------------------------------------------------------------------------
FROM node:22-trixie-slim AS runtime

ARG WKS_UID=10001
ARG WKS_GID=10001

LABEL org.opencontainers.image.title="workspacer-node-base" \
      org.opencontainers.image.description="Workspacer headless worker node: daemons, Tailscale, Claude Code. No project toolchains." \
      org.opencontainers.image.source="https://github.com/djtouchette/workspacer" \
      org.opencontainers.image.licenses="MIT" \
      dev.workspacer.node.role="base" \
      dev.workspacer.node.contract="/usr/local/lib/wks/verify-image.sh"

# What is here and why — every one of these is needed to RUN a node, not to
# build anybody's application:
#
#   iptables, nftables   kernel-mode tailscaled programs its rules through these
#   iproute2             `ip link show tailscale0`, the thirty-second check that
#                        kernel mode actually came up
#   busybox-static       the wake doorbell
#   util-linux           setpriv, which the entrypoint uses to drop privilege
#   tini                 PID 1 that reaps the grandchildren agents leave behind
#   procps, psmisc       the entrypoint's process supervision and diagnostics
#   git, gh, openssh     the node clones and pushes work; the SSH key is on the
#                        volume
#   ca-certificates,curl TLS, and the install steps below
#   ripgrep              Claude Code shells out to it
#   jq, less, unzip, rsync  the operational toolkit agents and the runbook use
#
# NOT here on purpose: build-essential, pkg-config, sqlite3, libsqlite3-dev,
# ruby, bundler, python3, go, bun. Those build application code and belong in a
# downstream layer. If the base ever needs one of them it is because a
# workspacer daemon needs it, and that is worth a comment saying so.
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
      ripgrep jq less unzip rsync && \
    rm -rf /var/lib/apt/lists/*

# Tailscale, pinned by tag. `stable` tracks their releases; pin a concrete
# version (e.g. tailscale/tailscale:v1.86.0) if you want reproducible images.
COPY --from=docker.io/tailscale/tailscale:stable /usr/local/bin/tailscaled /usr/local/bin/tailscaled
COPY --from=docker.io/tailscale/tailscale:stable /usr/local/bin/tailscale  /usr/local/bin/tailscale

# Claude Code. Global npm prefix is /usr/local in the node image, so this lands
# in /usr/local/lib/node_modules and survives the volume mount. npm's own cache
# under /root is deleted in the same layer: nothing reads it at runtime and
# verify-image.sh flags it as dead weight.
#
# hadolint ignore=DL3016
# Unpinned by default on purpose — a worker node running a months-old Claude
# Code is its own bug class. Pin it for a reproducible image with
# `--build-arg CLAUDE_CODE_VERSION=1.2.3`.
ARG CLAUDE_CODE_VERSION=latest
RUN npm install -g "@anthropic-ai/claude-code@${CLAUDE_CODE_VERSION}" && \
    claude --version && \
    npm cache clean --force && \
    rm -rf /root/.npm

COPY --from=binaries /out/brain      /usr/local/bin/brain
COPY --from=binaries /out/workspacer /usr/local/bin/workspacer
COPY --from=binaries /out/mcp        /usr/local/bin/mcp
COPY --from=binaries /out/claudemon  /usr/local/bin/claudemon

# The one file on the box that says what the box IS. /usr/local, never $HOME,
# for the same reason as everything else here. verify-image.sh asserts it and
# prints it; entrypoint.sh prints it on every boot; deploy/fly/RUNBOOK.md Part D
# maps it back to a release tag. See deploy/fly/write-build-stamp.sh for why a
# file has to carry this and a `--version` flag does not.
COPY --from=binaries /out/build-stamp /usr/local/share/workspacer/build-stamp

COPY deploy/fly/node/entrypoint.sh      /usr/local/lib/wks/entrypoint.sh
COPY deploy/fly/node/bootstrap.sh       /usr/local/lib/wks/bootstrap.sh
COPY deploy/fly/node/test-bootstrap.sh  /usr/local/lib/wks/test-bootstrap.sh
COPY deploy/fly/node/verify-image.sh    /usr/local/lib/wks/verify-image.sh
RUN chmod 0755 /usr/local/lib/wks/*.sh

# The agent user. Non-root deliberately: Claude Code refuses
# --dangerously-skip-permissions when euid is 0, and this fleet's full-access
# grants depend on that flag. -M because $HOME is on a volume that does not
# exist until the machine boots; the entrypoint creates and chowns it.
# -l / --no-log-init: without it a five-figure uid makes useradd allocate a
# sparse lastlog that some storage drivers materialise in full.
#
# A downstream layer MUST NOT re-declare this user, change its uid/gid, or add
# a `USER` instruction: the volume's files are chowned to 10001:10001 by
# bootstrap.sh, and the entrypoint has to start as root to run tailscaled
# before it drops privilege with setpriv. verify-image.sh checks all three.
RUN groupadd -g "${WKS_GID}" wks && \
    useradd -l --no-log-init -u "${WKS_UID}" -g "${WKS_GID}" -M -d /data/home -s /bin/bash wks

# PATH only. Every stateful variable (HOME, XDG_*, GOPATH, the caches) is set
# by the entrypoint and by fly.toml [env], never here — a Dockerfile ENV would
# also apply during `docker build`, where /data does not exist.
#
# This is the FULL PATH for the finished node, base and downstream both, and a
# downstream layer must not need to change it:
#   /usr/local/go/bin   reserved for a downstream Go toolchain. Empty in the
#                       base. A dangling PATH entry costs nothing, and fixing
#                       it here means a project layer never edits PATH — which
#                       is one fewer way to drop /usr/local/bin and lose the
#                       daemons.
#   /usr/local/bin      the daemons, tailscale, and anything downstream installs
# Anything else a downstream wants on PATH should be symlinked into
# /usr/local/bin rather than appended here.
ENV PATH="/usr/local/go/bin:/usr/local/bin:/usr/local/sbin:/usr/sbin:/usr/bin:/sbin:/bin"

# Assert the base honours its own contract before anyone builds on it. If this
# fails, the image is wrong and no downstream should inherit it.
RUN /usr/local/lib/wks/verify-image.sh

EXPOSE 8080
ENTRYPOINT ["/usr/bin/tini", "--", "/usr/local/lib/wks/entrypoint.sh"]
