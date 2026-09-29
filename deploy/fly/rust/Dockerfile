# syntax=docker/dockerfile:1
# Opt-in Rust backend image; existing Fly Dockerfiles remain unchanged.
# Repo-root context: --build-arg WKS_ROLE=hub (or node).
ARG WKS_ROLE=node
ARG WKS_INSTALL=source
ARG WKS_WITH_WEBAPP=1
ARG RUST_IMAGE=rust:1-trixie
FROM ${RUST_IMAGE} AS rustbuild
WORKDIR /src
COPY services/hub-rs/ services/hub-rs/
COPY services/claudemon/ services/claudemon/
COPY contracts/ contracts/
RUN --mount=type=cache,target=/usr/local/cargo/registry \
    --mount=type=cache,target=/build/target \
    CARGO_TARGET_DIR=/build/target cargo build --locked --release --manifest-path services/hub-rs/Cargo.toml --bin workspacer-rust && \
    CARGO_TARGET_DIR=/build/target cargo build --locked --release --manifest-path services/claudemon/Cargo.toml --bin claudemon && \
    mkdir -p /out && cp /build/target/release/workspacer-rust /build/target/release/claudemon /out/
ARG WKS_SOURCE_SHA
ARG TARGETARCH
COPY deploy/fly/write-build-stamp.sh /usr/local/bin/write-build-stamp.sh
RUN case "${TARGETARCH}" in amd64) stamp_platform=linux-x64 ;; arm64) stamp_platform=linux-arm64 ;; *) exit 1 ;; esac && \
    WKS_STAMP_COMPONENT=rust-server WKS_STAMP_INSTALL=source \
    WKS_STAMP_COMMIT="${WKS_SOURCE_SHA}" WKS_STAMP_PLATFORM="$stamp_platform" \
    bash /usr/local/bin/write-build-stamp.sh /out/build-stamp

# Frontend build tooling. Runtime Node is separately retained for external CLIs/plugins.
FROM node:22-trixie-slim AS webbuild-1
WORKDIR /src/desktop
COPY apps/desktop/src/renderer/package.json apps/desktop/src/renderer/package-lock.json ./src/renderer/
RUN npm ci --prefix src/renderer --no-audit --no-fund
COPY apps/desktop/ ./
WORKDIR /src/desktop/src/renderer
RUN npx vite build --config vite.config.web.ts
RUN test -f /src/desktop/dist/web/index.html && mkdir -p /out && cp -a /src/desktop/dist/web /out/web
FROM debian:trixie-slim AS webbuild-0
RUN mkdir -p /out/web
FROM webbuild-${WKS_WITH_WEBAPP} AS webapp

FROM rustbuild AS binaries-source
FROM debian:trixie-slim AS binaries-artifact
ARG WKS_RELEASE_TAG=nightly
ARG WKS_RELEASE_SHA
ARG WKS_RELEASE_REPO=DJTouchette/workspacer
ARG WKS_RELEASE_ASSET
ARG TARGETARCH
ARG WKS_RELEASE_BASE_URL
RUN apt-get update && apt-get install -y --no-install-recommends ca-certificates curl && rm -rf /var/lib/apt/lists/*
COPY deploy/fly/fetch-release.sh /usr/local/bin/fetch-release.sh
RUN --mount=type=secret,id=gh_token,required=false \
    case "${TARGETARCH}" in amd64) release_arch=x64 ;; arm64) release_arch=arm64 ;; *) exit 1 ;; esac && \
    WKS_RELEASE_ASSET="${WKS_RELEASE_ASSET:-workspacer-server-linux-${release_arch}.tar.gz}" \
    WKS_RELEASE_TOKEN_FILE=/run/secrets/gh_token \
    WKS_RELEASE_REQUIRE="workspacer-rust claudemon web/index.html build-stamp" \
    WKS_RELEASE_CHMOD="workspacer-rust claudemon" \
    bash /usr/local/bin/fetch-release.sh /out
FROM binaries-${WKS_INSTALL} AS binaries
FROM webapp AS webfiles-source
FROM binaries-artifact AS webfiles-artifact
ARG WKS_WITH_WEBAPP
RUN if [ "${WKS_WITH_WEBAPP}" = 0 ]; then rm -rf /out/web && mkdir -p /out/web; fi
FROM webfiles-${WKS_INSTALL} AS webfiles

# Node/npm remain third-party provider/plugin runtimes, never our backend.
FROM node:22-trixie-slim AS common
RUN apt-get update && apt-get install -y --no-install-recommends \
    ca-certificates curl git gh openssh-client iptables nftables iproute2 \
    busybox-static util-linux tini procps psmisc ripgrep jq less unzip rsync libstdc++6 libatomic1 && \
    rm -rf /var/lib/apt/lists/* && \
    groupadd -g 10001 wks && useradd -u 10001 -g 10001 --no-create-home --home-dir /data/home --shell /bin/bash wks
COPY --from=docker.io/tailscale/tailscale:stable /usr/local/bin/tailscaled /usr/local/bin/tailscaled
COPY --from=docker.io/tailscale/tailscale:stable /usr/local/bin/tailscale /usr/local/bin/tailscale
COPY --from=binaries /out/workspacer-rust /usr/local/bin/workspacer-rust
RUN ln -s workspacer-rust /usr/local/bin/workspacer
COPY --from=binaries /out/build-stamp /usr/local/share/workspacer/build-stamp
COPY deploy/fly/rust/launch.sh deploy/fly/rust/provision-worker-caller.sh deploy/fly/rust/verify-image.sh /usr/local/lib/wks-rust/
COPY --chmod=0755 deploy/fly/rust/image-contract.sh /usr/local/lib/wks/verify-image.sh
LABEL dev.workspacer.node.contract="/usr/local/lib/wks/verify-image.sh"
RUN chmod 0755 /usr/local/lib/wks-rust/*.sh
ENV WKS_RUST_BACKEND=1

FROM common AS runtime-node
ENV WKS_RUST_IMAGE_ROLE=node
LABEL dev.workspacer.node.role="rust-node" org.opencontainers.image.title="workspacer-rust-node"
ARG CLAUDE_CODE_VERSION=latest
RUN npm install -g "@anthropic-ai/claude-code@${CLAUDE_CODE_VERSION}" && \
    claude --version && npm cache clean --force && rm -rf /root/.npm
# Hook forwarding and user diagnostics still need the existing Rust CLI. The
# engine itself runs inside workspacer-rust, never as a sibling daemon process.
COPY --from=binaries /out/claudemon /usr/local/bin/claudemon
COPY deploy/fly/node/entrypoint.sh deploy/fly/node/bootstrap.sh deploy/fly/node/test-bootstrap.sh /usr/local/lib/wks/
RUN chmod 0755 /usr/local/lib/wks/*.sh
RUN /usr/local/lib/wks-rust/verify-image.sh node
ENTRYPOINT ["/usr/bin/tini", "--", "/usr/local/lib/wks/entrypoint.sh"]

FROM common AS runtime-hub
ENV WKS_RUST_IMAGE_ROLE=hub
LABEL dev.workspacer.node.role="rust-hub" org.opencontainers.image.title="workspacer-rust-hub"
COPY --from=webfiles /out/web /usr/local/share/workspacer/web/
COPY services/hub/examples/ /usr/local/share/workspacer/examples/
COPY deploy/fly/hub/entrypoint.sh deploy/fly/hub/bootstrap.sh deploy/fly/hub/test-bootstrap.sh /usr/local/lib/wks-hub/
RUN chmod 0755 /usr/local/lib/wks-hub/*.sh
RUN /usr/local/lib/wks-rust/verify-image.sh hub
ENTRYPOINT ["/usr/bin/tini", "--", "/usr/local/lib/wks-hub/entrypoint.sh"]

# Generic single-user combined template. The protected existing deployment uses
# the separate UID-preserving upgrade-supervisor.py path instead.
FROM runtime-node AS runtime-combined
ENV WKS_RUST_IMAGE_ROLE=combined
LABEL dev.workspacer.node.role="rust-combined" org.opencontainers.image.title="workspacer-rust-combined"
COPY --from=webfiles /out/web /usr/local/share/workspacer/web/
COPY services/hub/examples/ /usr/local/share/workspacer/examples/
COPY deploy/fly/combined/entrypoint.sh /usr/local/lib/wks-combined/entrypoint.sh
RUN chmod 0755 /usr/local/lib/wks-combined/entrypoint.sh
RUN /usr/local/lib/wks-rust/verify-image.sh combined
ENTRYPOINT ["/usr/bin/tini", "--", "/usr/local/lib/wks-combined/entrypoint.sh"]

FROM runtime-${WKS_ROLE} AS runtime
