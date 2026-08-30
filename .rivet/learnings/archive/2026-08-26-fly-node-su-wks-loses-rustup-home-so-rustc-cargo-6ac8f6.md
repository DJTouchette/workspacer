---
title: "Fly node: `su - wks` loses RUSTUP_HOME, so rustc/cargo fail in the shell the runbook tells you to use"
date: 2026-08-26
promoted: true
promoted_to: fly-node-deploy
---

# Fly node: `su - wks` loses RUSTUP_HOME, so rustc/cargo fail in the shell the runbook tells you to use

## Observation

On `workspacer-node`:

    su - wks -c 'rustc --version'   -> error: rustup could not choose a version of rustc to run,
                                       because one wasn't specified explicitly, and no default is configured
    su   wks -c 'rustc --version'   -> rustc 1.98.0 / cargo 1.98.0

Rust is installed correctly. `/usr/local/rustup/toolchains/1.98.0-x86_64-unknown-linux-gnu`
exists and `/usr/local/rustup/settings.toml` names it as `default_toolchain`. The
image sets `ENV RUSTUP_HOME=/usr/local/rustup`.

A LOGIN shell sources `~/.profile` -> `~/.wks-env`, which rebuilds the
environment from the node `fly.toml [env]` list. It exports `HOME`, `XDG_*`,
`GOPATH`, `GOMODCACHE`, `GOCACHE`, `BUNDLE_PATH`, `BUN_INSTALL_CACHE_DIR`,
`npm_config_cache`, `HISTFILE`, `PATH`, `WKS_STATE_DB` — and **no
`RUSTUP_HOME`**. rustup then falls back to `$HOME/.rustup` = `/data/home/.rustup`,
which is on the volume and empty. `CARGO_HOME` is likewise unset at runtime.

**Agents are unaffected.** The `claudemon` process carries
`RUSTUP_HOME=/usr/local/rustup`, so a dispatched worker on the node can
`cargo build`. Go is unaffected because `/usr/local/go/bin/go` needs no env var
and `.wks-env` puts it on `PATH`.

## Impact

Every interactive check in `deploy/fly/*/RUNBOOK.md` uses `su - wks` — a login
shell, deliberately, so `~/.wks-env` is sourced. So a human verifying the node
concludes Rust is missing when it is present and working for the workloads that
matter. Any script driven through a login shell also cannot build Rust.

## Recommendation

Add `RUSTUP_HOME='/usr/local/rustup'` and `CARGO_HOME` to whatever generates
`~/.wks-env`, alongside `GOPATH`. Until then use `su wks -c` (non-login) for
Rust checks, or export `RUSTUP_HOME` by hand.

## Related paths

- `deploy/fly/node/bootstrap.sh`
- `deploy/fly/node/fly.toml`
- `~/Work/fly-node/Dockerfile` (installs Rust; outside this repo)

## Confidence

high — observed directly on machine 1857645df24448, 2026-08-26.
