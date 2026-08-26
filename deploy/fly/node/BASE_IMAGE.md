# `workspacer-node-base` — the contract

`deploy/fly/node/Dockerfile` builds a **base image**: a workspacer worker node
with the daemons, Tailscale, Claude Code, the entrypoint, and the `wks` user —
and **no language toolchain for anybody's application code**.

It is meant to be built `FROM`. This document is what a downstream Dockerfile
author needs, and it is the reason the rules below stopped being comments in one
file and became an executable check.

- **What it provides** — binaries, paths, user, entrypoint, volume expectations
- **What a downstream layer must not do** — [the rules](#the-rules)
- **How to extend it** — [the shape of a project image](#extending-this-image)
- **How to name and tag it** — [identity](#identity-and-tagging)

---

## Why there are two images

Before the split, one Dockerfile built the Go and Rust daemons **and** installed
Go, Ruby, bundler, bun, python3, sqlite and build-essential. Adding a Rails gem
invalidated a layer sitting above the builder stages, so a dependency bump
recompiled `brain`, `workspacer` and `claudemon` from scratch.

| | changes when | owns |
|---|---|---|
| **base** — `deploy/fly/node/Dockerfile`, in the workspacer repo | workspacer changes | the daemons, the node's operational behaviour |
| **project** — a `Dockerfile` you own, elsewhere | your dependencies change | language toolchains, system libs your app needs |

The base is part of the workspacer product. Your toolchain list is not, which is
why it lives in your repo and not in workspacer's.

---

## What the base provides

### Binaries

| Path | What |
|---|---|
| `/usr/local/bin/brain` | the node's control-plane client — connects out to the hub over the bus |
| `/usr/local/bin/workspacer` | the workspacer CLI |
| `/usr/local/bin/claudemon` | the agent supervisor daemon (Rust) |
| `/usr/local/bin/tailscaled`, `/usr/local/bin/tailscale` | kernel-mode Tailscale |
| `/usr/local/bin/claude` | Claude Code, via a global npm install under `/usr/local/lib/node_modules` |
| `/usr/bin/tini` | PID 1 |
| `/usr/local/bin/node`, `npm` | Node 22, from the `node:22-trixie-slim` base |

Plus the operational toolkit the entrypoint, bootstrap and agents assume:
`git`, `gh`, `openssh-client`, `curl`, `ripgrep`, `jq`, `less`, `unzip`,
`rsync`, `procps`, `psmisc`, `iptables`, `nftables`, `iproute2`,
`busybox-static`, `util-linux`.

### Scripts

| Path | What |
|---|---|
| `/usr/local/lib/wks/entrypoint.sh` | PID 1's payload: boot log → bootstrap → doorbell → tailscaled → `claudemon init` → claudemon → brain, plus signals and exit-reason recording |
| `/usr/local/lib/wks/bootstrap.sh` | prepares the volume; idempotent on empty, populated and damaged volumes; creates **zero symlinks** |
| `/usr/local/lib/wks/test-bootstrap.sh` | 106 assertions over `bootstrap.sh` |
| `/usr/local/lib/wks/verify-image.sh` | **the contract check.** See [the rules](#the-rules) |

### User

`wks`, **uid 10001, gid 10001**, home `/data/home`, shell `/bin/bash`, created
with `-M` (no home directory in the image — it is on the volume).

Non-root is load-bearing, not hygiene: **Claude Code refuses
`--dangerously-skip-permissions` at euid 0**, and the fleet's full-access grants
depend on that flag.

### Entrypoint and ports

```
ENTRYPOINT ["/usr/bin/tini", "--", "/usr/local/lib/wks/entrypoint.sh"]
EXPOSE 8080          # the wake doorbell
```

The entrypoint **starts as root** — it has to, to run `tailscaled` and to chown
the volume — and drops to `wks` with `setpriv` before starting the daemons.

### PATH

```
/usr/local/go/bin:/usr/local/bin:/usr/local/sbin:/usr/sbin:/usr/bin:/sbin:/bin
```

This is the final PATH for the finished node. `/usr/local/go/bin` is **empty in
the base and reserved for a downstream Go toolchain** — pre-declaring it means a
project layer never has to touch `PATH`, which is one fewer way to accidentally
drop `/usr/local/bin` and lose the daemons.

`PATH` is the **only** `ENV` in the image. See rule 3.

### What it expects to find at runtime

A volume mounted at **`/data`**, and these variables set by the platform
(`fly.toml [env]`) and re-exported by the entrypoint — never by a Dockerfile:

```
WKS_DATA=/data   WKS_HOME=/data/home   HOME=/data/home
XDG_CONFIG_HOME  XDG_DATA_HOME  XDG_STATE_HOME  XDG_CACHE_HOME
GOPATH  GOMODCACHE  GOCACHE  BUNDLE_PATH  BUN_INSTALL_CACHE_DIR  npm_config_cache
```

Note the shape: **toolchains in the image, their caches on the volume.** Caches
in the image would mean rebuilding it on every dependency bump; toolchains on
the volume would make the volume unreproducible. A downstream layer installs the
toolchain and inherits the cache location for free — `BUNDLE_PATH` and
`npm_config_cache` are already pointed at `/data`.

Secrets (`TAILSCALE_AUTHKEY`, `HUB_BUS_URL`, `HUB_TOKEN`) come from
`fly secrets set`. See [RUNBOOK.md](RUNBOOK.md).

---

## The rules

### 1. Nothing may be installed into `$HOME`. This is the one that bites.

`$HOME` is `/data/home`. `/data` is a **volume mount point**. The volume is
mounted *after* the image is built and it **shadows whatever the image put
there**.

So this builds green, passes a `docker run` smoke test, and then loses the tool
the first time the real machine boots:

```dockerfile
RUN curl -fsSL https://bun.sh/install | bash          # → ~/.bun          GONE
RUN bundle config set --local path ~/.gems            # → ~/.gems         GONE
RUN npm config set prefix ~/.npm-global && npm i -g x  # → ~/.npm-global  GONE
```

There is no error. The tool is simply not there on the next wake, and the
Dockerfile that installed it still says it did. That is a day to debug.

**Install everything to `/usr/local`.** Every installer has a way:

```dockerfile
ENV BUN_INSTALL=/usr/local                            # bun
RUN curl -fsSL https://bun.sh/install -o /tmp/i.sh && bash /tmp/i.sh
COPY --from=docker.io/golang:1.25-trixie /usr/local/go /usr/local/go
RUN npm install -g <pkg>                              # prefix is already /usr/local
```

**Make it loud.** End every downstream build with:

```dockerfile
RUN /usr/local/lib/wks/verify-image.sh
```

It fails the build, naming the directory, if anything landed under a home
directory, if `/data` is non-empty in the image, if a stateful `ENV` was baked
in, or if the base's own contract was broken. It costs about a second.

> `ONBUILD` would run this automatically, but it fires at the *start* of the
> child build, right after `FROM` — before the child has done anything worth
> checking. An explicit last `RUN` is the only placement that catches the child.

### 2. Do not add `USER`, and do not touch the `wks` user

The entrypoint must start as root (tailscaled, chown of the volume) and drops
privilege itself. A downstream `USER wks` produces a node that cannot bring up
Tailscale. Changing the uid or gid orphans every file `bootstrap.sh` chowned to
`10001:10001` on an existing volume.

### 3. Do not add stateful `ENV`

`HOME`, `XDG_*`, `GOPATH`, `GOCACHE`, `BUNDLE_PATH` and friends are set by the
entrypoint and `fly.toml`. A Dockerfile `ENV` **also applies during
`docker build`**, where `/data` does not exist — so it breaks the build it was
meant to configure, and at runtime it is overridden anyway.

The exceptions are build-scoped `ENV`s that exist only to redirect an installer
away from `$HOME` — `ENV BUN_INSTALL=/usr/local` is fine and is the point.
Anything naming a path under `/data` is not.

### 4. Do not override `PATH`

It is already correct, including the reserved Go slot. Symlink into
`/usr/local/bin` instead.

### 5. Do not pin Debian package versions

The image is rebuilt to *pick up* Debian security updates, and a pinned version
breaks the build the moment the archive rotates a point release. Reproducibility
comes from the base image digest. `hadolint`'s DL3008 is suppressed deliberately.

### 6. Stay on Debian trixie

Not Alpine: kernel-mode `tailscaled` needs nftables, and Alpine is where that
fails confusingly on Fly. Not bookworm: it ships Ruby 3.1 and Rails 8 needs
≥ 3.2. Both reasons are in the base Dockerfile's header. A downstream layer
inherits the distro and should not try to change it.

### 7. Do not remove things from `/usr/local/bin`

`verify-image.sh` checks that `brain`, `workspacer`, `claudemon`, `tailscale`,
`tailscaled`, `claude` and `tini` are all still resolvable.

---

## Extending this image

The shape of a project image:

```dockerfile
# syntax=docker/dockerfile:1
ARG WKS_BASE=workspacer-node-base:dev
FROM ${WKS_BASE}

# Root, no USER — see rule 2.

# System libs and language runtimes. Unpinned — see rule 5.
# hadolint ignore=DL3008
RUN apt-get update && \
    apt-get install -y --no-install-recommends \
      build-essential pkg-config sqlite3 libsqlite3-dev && \
    rm -rf /var/lib/apt/lists/*

# Toolchains that are not Debian packages: copy from an official image rather
# than downloading a tarball. This is a registry pull, not a build — it does not
# rebuild anything, and the version is yours to choose.
COPY --from=docker.io/golang:1.25-trixie /usr/local/go /usr/local/go

# MANDATORY last layer. Fails the build if anything landed in $HOME.
RUN /usr/local/lib/wks/verify-image.sh
```

`example.Dockerfile` next to this file is a working, buildable version of that —
it is what proves the base actually extends, and it is what CI should build.

**Build it in two steps**, base first:

```sh
docker build -f deploy/fly/node/Dockerfile -t workspacer-node-base:dev .   # repo root as context
docker build -f Dockerfile -t my-node:dev --build-arg WKS_BASE=workspacer-node-base:dev .
```

The project build needs **no repo context at all** — it only runs `apt-get` and
copies from public images — so its context can be its own directory.

---

## Identity and tagging

**The base has no published home yet. That is an open decision, not an
oversight.**

What exists today:

- The image builds from `deploy/fly/node/Dockerfile` with the **repo root** as
  context, and the conventional local tag is **`workspacer-node-base:dev`**.
- It carries OCI labels — `org.opencontainers.image.title=workspacer-node-base`
  and `dev.workspacer.node.role=base` — so a downstream image can be identified
  as descending from it (`docker inspect`).

What has **not** been decided, and needs to be before a downstream `FROM` line
can point at anything durable:

| Option | What a downstream `FROM` says | Cost |
|---|---|---|
| **Local only** (today) | `FROM workspacer-node-base:dev` | you must build the base yourself, in the workspacer checkout, before every project build. `fly deploy` must use `--local-only`. |
| **Fly registry** | `FROM registry.fly.io/<app>:base-<sha>` | private, free with the Fly account, needs `fly auth docker`. Remote builders work. |
| **GHCR** | `FROM ghcr.io/djtouchette/workspacer-node-base:<tag>` | public; makes the base a genuinely published artefact, and a supported one — which is a product commitment, not just a build change. |

Recommended tag story whichever is chosen: `:<git-sha>` as the immutable tag a
downstream pins, plus a moving `:latest` for humans. Workspacer already tags
releases `v*` and publishes nightlies, so `:v1.2.3` alongside would be
consistent — but that is the user's call, and until it is made, **local build
and tag is the only supported path.**
