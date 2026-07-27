# syntax=docker/dockerfile:1
#
# Workspacer, headless. The image is the standalone server bundle the release
# workflow already ships (workspacer + hub + brain + claudemon + the built web
# app, all in one directory) with an OCI wrapper around it — `workspacer serve`
# supervises the daemons exactly as it does from the tarball.
#
# Build context is the repo root:  docker build -t workspacer .
#
# Three build stages run in parallel (web / Go / Rust), then a slim Node runtime
# — Node is not optional at runtime: plugin sidecars are Node, and the agent
# CLIs the daemon spawns (claude, codex, …) are npm packages.

# ---------------------------------------------------------------- web renderer
# The same React app the Electron build produces, built for the browser (base
# /app/). The hub serves it at /app for full remote parity. src/renderer builds
# standalone against its own lockfile; src/main comes along because the renderer
# type-imports ipcTypes from it.
FROM node:22-bookworm-slim AS web
WORKDIR /src/apps/desktop
COPY apps/desktop/src/renderer/package.json apps/desktop/src/renderer/package-lock.json ./src/renderer/
RUN --mount=type=cache,target=/root/.npm \
    cd src/renderer && npm ci
COPY apps/desktop/src ./src
RUN cd src/renderer && npx vite build --config vite.config.web.ts
# → /src/apps/desktop/dist/web

# ------------------------------------------------------------------- Go daemons
# hub (bus + web clients), brain (headless capability provider), mcp (MCP
# bridge), workspacer (the launcher/supervisor this image runs as PID 1).
FROM golang:1.25-bookworm AS go
WORKDIR /src/hub
COPY services/hub/go.mod services/hub/go.sum ./
RUN --mount=type=cache,target=/go/pkg/mod go mod download
COPY services/hub/ ./
RUN --mount=type=cache,target=/go/pkg/mod \
    --mount=type=cache,target=/root/.cache/go-build \
    mkdir -p /out && \
    go build -trimpath -o /out/hub        ./cmd/hub && \
    go build -trimpath -o /out/brain      ./cmd/brain && \
    go build -trimpath -o /out/mcp        ./cmd/mcp && \
    go build -trimpath -o /out/workspacer ./cmd/workspacer

# ----------------------------------------------------------------- Rust daemon
# claudemon: session state, PTY/stream transports, SQLite store. rusqlite is
# `bundled` (needs cc, present here) and reqwest is rustls — no system OpenSSL.
FROM rust:1-bookworm AS rust
WORKDIR /src/claudemon
COPY services/claudemon/Cargo.toml services/claudemon/Cargo.lock services/claudemon/build.rs ./
COPY services/claudemon/src ./src
# The cargo target dir is a cache mount, so the binary must be copied OUT of it
# in the same RUN — it does not persist into the layer.
RUN --mount=type=cache,target=/usr/local/cargo/registry \
    --mount=type=cache,target=/src/claudemon/target \
    mkdir -p /out && \
    cargo build --release --bin claudemon && \
    cp target/release/claudemon /out/claudemon

# --------------------------------------------------------------------- runtime
FROM node:22-bookworm-slim AS runtime

# git: agents work in git repos (and git-review reads them). ripgrep: code
# search. tini: PID-1 reaping for the PTY grandchildren agents leave behind.
# procps/curl: healthcheck + `workspacer status`.
RUN apt-get update && \
    apt-get install -y --no-install-recommends \
        ca-certificates curl git ripgrep procps tini openssh-client less && \
    rm -rf /var/lib/apt/lists/*

# Agent CLIs. Claude Code is installed by default because an agent control
# plane with no agent installed is a dead container; build with
# `--build-arg AGENT_CLIS=""` to ship a bare image and mount your own.
ARG AGENT_CLIS="@anthropic-ai/claude-code"
RUN --mount=type=cache,target=/root/.npm \
    if [ -n "$AGENT_CLIS" ]; then npm install -g $AGENT_CLIS; fi

# The node:slim image already owns uid/gid 1000 as `node`; rename rather than
# add, so the container user is uid 1000 (what a host bind-mount usually wants).
RUN usermod -l wks -d /home/wks -m node && groupmod -n wks node

# The server bundle, one directory: `workspacer` resolves its daemons as
# siblings and auto-serves a sibling web/ at /app, so no flags are needed.
COPY --from=go   /out/hub /out/brain /out/mcp /out/workspacer /opt/workspacer/
COPY --from=rust /out/claudemon                                /opt/workspacer/
COPY --from=web  /src/apps/desktop/dist/web                    /opt/workspacer/web
# selfDir() resolves symlinks, so the link on PATH still finds /opt/workspacer.
RUN ln -s /opt/workspacer/workspacer /usr/local/bin/workspacer

COPY docker/entrypoint.sh /usr/local/bin/workspacer-entrypoint
RUN chmod 755 /usr/local/bin/workspacer-entrypoint

# XDG config dir — the token, config.yaml and plugins/ live here, and it is the
# SAME path the desktop app uses, so a paired phone keeps working across both.
ENV HOME=/home/wks \
    XDG_CONFIG_HOME=/home/wks/.config \
    WORKSPACER_IN_DOCKER=1
RUN mkdir -p /home/wks/.config/workspacer/plugins /home/wks/.claude /workspace && \
    chown -R wks:wks /home/wks /workspace

USER wks
WORKDIR /workspace

# The WHOLE home is one volume, deliberately. The stack scatters durable state
# across four sibling directories — .config/workspacer (token, config, plugins),
# .config/workspacer-hub (layout, VAPID push keypair), .claudemon (the session
# SQLite store), .claude (agent credentials + transcripts) — and .workspacer
# (handoff briefs, model-rate overrides) appears on first use. Enumerating them
# means a new one silently becomes ephemeral; a home volume cannot miss any.
# A bind-mount of a host ~/.claude still nests over it normally.
VOLUME ["/home/wks", "/workspace"]

# Only the hub is published. claudemon's API (7891) and hook (7890) ports stay
# on loopback inside the container — they are unauthenticated by design.
EXPOSE 7895

HEALTHCHECK --interval=30s --timeout=5s --start-period=30s --retries=3 \
    CMD curl -fsS http://127.0.0.1:7895/health || exit 1

# --host 0.0.0.0 is required for the port publish to be reachable at all; the
# hub is token-authenticated, but see docs/docker.md — put this behind Tailscale
# or a private network, never straight onto the internet.
ENTRYPOINT ["/usr/bin/tini", "--", "/usr/local/bin/workspacer-entrypoint"]
CMD ["serve", "--host", "0.0.0.0"]
