# syntax=docker/dockerfile:1
#
# A minimal project image built FROM the workspacer node base.
#
# This is scaffolding, not a real environment: it exists to prove that the base
# extends the way BASE_IMAGE.md says it does, and to give CI something cheap to
# build. A real project image lives in YOUR repo, not this one — your toolchain
# list is not part of workspacer.
#
#   docker build -f deploy/fly/node/Dockerfile -t workspacer-node-base:dev .
#   docker build -f deploy/fly/node/example.Dockerfile \
#                --build-arg WKS_BASE=workspacer-node-base:dev \
#                -t workspacer-node-example:dev deploy/fly/node
#
# Note the context: `deploy/fly/node`, not the repo root. A project image needs
# no repo context at all — it only runs apt-get and copies from public images.

ARG WKS_BASE=workspacer-node-base:dev
FROM ${WKS_BASE}

# ===========================================================================
#  READ THIS BEFORE ADDING ANYTHING
# ===========================================================================
#
#  $HOME IS A VOLUME MOUNT POINT (/data/home). Anything installed under it is
#  SHADOWED the instant the machine boots and its volume attaches. There is no
#  error — the tool is just gone on the next wake.
#
#      WRONG                                       RIGHT
#      curl .../install | bash        (→ ~/.bun)   ENV BUN_INSTALL=/usr/local
#      bundle config path ~/.gems     (→ ~/.gems)  it is already /data/bundle
#      npm config set prefix ~/...                 the prefix is already /usr/local
#
#  Install to /usr/local. Do not add USER. Do not override PATH. Do not add an
#  ENV naming a path under /data. Full contract: BASE_IMAGE.md in this repo.
#
#  The last RUN in this file enforces all of that. Do not delete it.
# ===========================================================================

# System packages. Unpinned deliberately — see BASE_IMAGE.md rule 5.
# hadolint ignore=DL3008
RUN apt-get update && \
    apt-get install -y --no-install-recommends \
      build-essential pkg-config sqlite3 libsqlite3-dev && \
    rm -rf /var/lib/apt/lists/*

# A toolchain that is not a Debian package: copy it from the official image
# rather than downloading a tarball. This is a registry pull, not a build — it
# does not rebuild the base's Go or Rust builder stages, and the version is
# yours to pick. /usr/local/go/bin is already on the PATH the base set.
COPY --from=docker.io/golang:1.25-trixie /usr/local/go /usr/local/go

# MANDATORY, and it must be last: fails the build if anything landed under a
# home directory, if /data is non-empty, if a stateful ENV was baked in, or if
# the base's own contract was broken.
RUN /usr/local/lib/wks/verify-image.sh
