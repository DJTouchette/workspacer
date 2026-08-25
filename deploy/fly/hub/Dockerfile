# syntax=docker/dockerfile:1
#
# ===========================================================================
#  workspacer ALWAYS-ON HUB — a downstream layer on the node base image.
# ===========================================================================
#
# Build context is the REPO ROOT, and the BASE MUST BE BUILT FIRST:
#
#   docker build -f deploy/fly/node/Dockerfile -t workspacer-node-base:dev .
#   docker build -f deploy/fly/hub/Dockerfile  -t workspacer-hub:dev .
#
#   fly deploy --config deploy/fly/hub/fly.toml \
#              --dockerfile deploy/fly/hub/Dockerfile .
#
# ---------------------------------------------------------------------------
# WHY THIS FILE IS SHORT
# ---------------------------------------------------------------------------
# deploy/fly/node/Dockerfile is the workspacer BASE image: Debian trixie,
# kernel-mode Tailscale, tini, the `wks` user at 10001:10001, the operational
# toolkit, and the daemons. Its contract is deploy/fly/node/BASE_IMAGE.md. This
# layer adds exactly three things the hub needs and the node does not:
#
#   1. the `hub` binary          — the base builds brain, workspacer and
#                                  claudemon, but not this. See the note below.
#   2. dist/web                  — the full React renderer, served at /app/
#   3. bubblewrap                — the Linux plugin-sandbox mechanism
#
# plus its own entrypoint and bootstrap, at /usr/local/lib/wks-hub/, beside the
# node's rather than over them. The base's copies stay exactly where they are:
# verify-image.sh checks for them by path, and overwriting a file another
# worker owns to make my own check pass would be the wrong kind of clever.
#
# NOTHING ELSE IS ADDED. The base deliberately carries no project toolchains —
# no Ruby, no bun, no Go, no Rust — and the hub needs none of them either. It
# relays bus messages and calls one HTTP API.
#
# ---------------------------------------------------------------------------
# WHAT THIS IMAGE CARRIES THAT IT DOES NOT NEED, AND WHY THAT IS ACCEPTED
# ---------------------------------------------------------------------------
# The base is a WORKER NODE base. Inheriting it puts `claudemon`, `brain` and
# Claude Code itself on the always-on machine, none of which this deployment
# ever runs: entrypoint.sh starts `hub` and nothing else, and bootstrap.sh
# creates no ~/.claude, no ~/.claude.json and no ~/.ssh, so there is no
# credential for any of them to find.
#
# That is unused code on the more sensitive of the two machines, and it is worth
# naming rather than leaving as a silent inheritance. It is accepted here for
# one image because sharing the base is what keeps the Tailscale setup, the uid,
# the volume contract and verify-image.sh identical across both machines — and
# those are the things that actually go wrong when they drift. The right fix is
# upstream (a build ARG on the base, or a `workspacer-hub-base` beside it) and
# belongs to whoever owns that file, not to this one. README.md records it.

ARG WKS_BASE=workspacer-node-base:dev
# Declared HERE, before the first FROM, because only a global ARG is available
# for interpolation in a FROM line. Selects between the two webapp stages below.
ARG WKS_WITH_WEBAPP=1

# ---------------------------------------------------------------------------
FROM golang:1.25-trixie AS gobuild
# The hub itself. The base builds brain/workspacer/claudemon from the same
# module but not `hub`, because a worker node does not run one — the node's
# topology is provider-attach, with the hub a thousand miles away.
#
# Same stage shape as the base's so the module cache mount is shared between the
# two builds: on a machine that has just built the base this is seconds.
WORKDIR /src/hub
COPY services/hub/go.mod services/hub/go.sum ./
RUN --mount=type=cache,target=/go/pkg/mod go mod download
COPY services/hub/ ./
ENV CGO_ENABLED=0
RUN --mount=type=cache,target=/go/pkg/mod \
    --mount=type=cache,target=/root/.cache/go-build \
    mkdir -p /out && \
    go build -trimpath -o /out/hub ./cmd/hub

# ---------------------------------------------------------------------------
# The web app served at /app/ — the SAME React renderer the desktop runs, with
# window.electronAPI polyfilled over the hub bus. /m (the phone PWA) is
# go:embed'ed into the hub binary and needs none of this; /app is a real build.
FROM node:22-trixie-slim AS webbuild-1
# WORKDIR is /src/desktop and the sources go under it because the config's outDir
# is RELATIVE — `../../dist/web` from src/renderer — so the tree shape decides
# where the bundle lands. Building anywhere else silently produces nothing at the
# path the runtime stage copies from, which is why `test -f` below is a step of
# its own rather than a `&&`.
WORKDIR /src/desktop

# ONLY the renderer's dependencies. apps/desktop/package.json is the ELECTRON
# app: its postinstall runs electron-rebuild for better-sqlite3, which downloads
# an Electron binary and rebuilds a native module against it. The web build needs
# none of that — it is Vite over src/renderer, which carries its own package.json
# and lockfile — so installing the parent would be a large download, a native
# toolchain requirement, and several new ways to fail, for nothing.
COPY apps/desktop/src/renderer/package.json apps/desktop/src/renderer/package-lock.json ./src/renderer/
RUN npm ci --prefix src/renderer --no-audit --no-fund

COPY apps/desktop/ ./
# Exactly the repo's own `build:renderer:web` script: run FROM src/renderer, so
# the config resolves and its relative outDir (../../dist/web) lands under
# /src/desktop. vite takes its root positionally, not as --root.
WORKDIR /src/desktop/src/renderer
RUN npx vite build --config vite.config.web.ts

WORKDIR /src/desktop
# A step of its own, not an `&&`: the outDir is relative, so the failure mode
# worth catching is "the build succeeded and put the bundle somewhere else".
RUN test -f dist/web/index.html
RUN mkdir -p /out && cp -a dist/web /out/web

# The conditional-stage trick: --build-arg WKS_WITH_WEBAPP=0 selects the empty
# stage instead, and the hub then serves /m only. Worth having because this is
# by far the slowest part of the build and an operator who only wants a phone
# wake button should not pay for it.
# The disabled arm. NOT `FROM scratch`: both arms must expose the SAME path, so
# that the runtime COPY below can name /out/web rather than the stage root. It
# named the root once, and copied the entire 570MB builder filesystem into the
# image — a mistake that builds green and is invisible until you weigh it.
FROM node:22-trixie-slim AS webbuild-0
RUN mkdir -p /out/web

# The "untagged image" warning is a false positive on a stage alias: both arms
# resolve to stages defined above, not to registry references.
# hadolint ignore=DL3006
FROM webbuild-${WKS_WITH_WEBAPP} AS webapp

# ---------------------------------------------------------------------------
FROM ${WKS_BASE} AS runtime

LABEL org.opencontainers.image.title="workspacer-hub" \
      org.opencontainers.image.description="Workspacer always-on hub: bus, node registry, /m and /app. No agents." \
      org.opencontainers.image.source="https://github.com/djtouchette/workspacer" \
      org.opencontainers.image.licenses="MIT" \
      dev.workspacer.node.role="hub"

# bubblewrap is the Linux mechanism internal/sandbox uses to confine plugin
# sidecars (exec.LookPath("bwrap")), and without it WORKSPACER_PLUGIN_SANDBOX=
# enforce refuses to start any sidecar at all — which on this machine is the
# correct fail-closed behaviour but a poor default. One ~100KB package makes
# `enforce` a real setting rather than a blanket refusal. It is a security
# mechanism the hub needs, not an application toolchain, so it belongs here and
# not in the base.
#
# Unpinned, per base rule 5: this image is rebuilt to pick up Debian security
# updates, and a pinned version breaks the moment the archive rotates.
# hadolint ignore=DL3008
RUN apt-get update && \
    apt-get install -y --no-install-recommends bubblewrap && \
    rm -rf /var/lib/apt/lists/*

COPY --from=gobuild /out/hub /usr/local/bin/hub

# /usr/local, never $HOME — $HOME is /data/home and the volume shadows it at
# boot. verify-image.sh below enforces that; this is what obeying it looks like.
COPY --from=webapp /out/web /usr/local/share/workspacer/web/

COPY deploy/fly/hub/entrypoint.sh      /usr/local/lib/wks-hub/entrypoint.sh
COPY deploy/fly/hub/bootstrap.sh       /usr/local/lib/wks-hub/bootstrap.sh
COPY deploy/fly/hub/test-bootstrap.sh  /usr/local/lib/wks-hub/test-bootstrap.sh
RUN chmod 0755 /usr/local/lib/wks-hub/*.sh

# No ENV, no USER, no PATH override — base rules 2, 3 and 4. Every stateful
# variable is set by entrypoint.sh and fly.toml, and the entrypoint must start
# as root to run tailscaled and chown the volume before it drops to `wks`.

# MANDATORY LAST RUN. Ships inside the base at this path and is the contract
# check for every downstream layer: it fails the build, naming the directory, if
# anything landed under a home dir, if /data is non-empty in the image, if a
# stateful ENV resolves under /data, if a USER instruction was added, if the wks
# user was renumbered, or if a daemon fell off PATH. It is what keeps the
# $HOME-on-a-volume rule from rotting now that it crosses a file boundary.
#
# Deliberately NOT merged with the chmod above, which is what DL3059 asks for:
# this must be the LAST RUN of the build, and folding it into a neighbour makes
# it possible for a later edit to add a layer after it without anyone noticing.
# hadolint ignore=DL3059
RUN /usr/local/lib/wks/verify-image.sh

# The hub is reached through `tailscale serve`, which forwards to 127.0.0.1 from
# inside this container — nothing is published to the machine's network
# interface, so there is nothing to EXPOSE. The base EXPOSEs 8080 for the node's
# wake doorbell; this machine has no doorbell, because it is never asleep.
ENTRYPOINT ["/usr/bin/tini", "--", "/usr/local/lib/wks-hub/entrypoint.sh"]
