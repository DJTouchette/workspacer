# `deploy/fly/node` — the Fly.io headless worker node

A single Fly Machine that runs `claudemon` plus `brain --hub <wss url> --token`,
sleeps when idle, and is reachable over Tailscale. **Provider-attach topology:**
no hub on this machine, no federation.

**Start with [RUNBOOK.md](RUNBOOK.md).** Provisioning is a one-time,
order-dependent procedure with three interactive logins that cannot be scripted,
and the runbook is the only place the order is written down.

| File | What it is |
|---|---|
| `RUNBOOK.md` | The provisioning procedure, the persistence map, and what is *not* verified |
| `BASE_IMAGE.md` | **The base image contract** — what it provides, what a downstream layer must not do, how to extend it, how it is named |
| `Dockerfile` | Multi-stage: Go builders for `brain`/`workspacer`, Rust for `claudemon`, Debian trixie runtime with Tailscale and Claude Code. **No project toolchains** |
| `example.Dockerfile` | A minimal project image `FROM` the base. Scaffolding that proves the base extends |
| `Dockerfile.dockerignore` | Build context is the repo root; this keeps it to the two services |
| `fly.toml` | Region, sizing, volume, restart policy, and the `auto_start_machines` wake backstop — each with the reasoning inline |
| `entrypoint.sh` | PID 1: **replay the previous boot to stdout** → boot log → bootstrap → doorbell → tailscaled → `claudemon init` → claudemon → brain, plus signal handling and exit-reason recording |
| `bootstrap.sh` | Prepares the volume **and decides FIRST RUN vs STATE LOSS per file**. Idempotent, correct on both an empty and a populated volume, and creates **zero symlinks** |
| `test-bootstrap.sh` | 106 assertions over `bootstrap.sh`. No root, no Docker, no Fly, ~1 second |
| `verify-image.sh` | Shipped into the image. The **last `RUN` of every build**, base and downstream: fails the build if anything was installed into `$HOME` |

## This is a base image

`Dockerfile` builds **workspacer and nothing project-specific**. Language
toolchains for application code belong in a small image built `FROM` this one,
in whatever repo owns that code — see [BASE_IMAGE.md](BASE_IMAGE.md).

It used to be one file that also installed Go, Ruby, bundler, bun, python3,
sqlite and build-essential. That put a frequently-changing layer *above* the Go
and Rust builder stages, so adding a Rails gem recompiled `brain`, `workspacer`
and `claudemon` from scratch. It no longer does.

## Two things to know before editing anything here

**Persist directories, never individual files.** Every atomic writer in this
stack does tmp-write + `rename(2)`, which replaces a *file* symlink with a
regular file on the ephemeral rootfs — working for that boot, gone on the next
wake. This design sidesteps the class entirely by putting `$HOME` on the volume,
and `test-bootstrap.sh` asserts that no symlinks exist anywhere under it.

**The rootfs is rebuilt from the image on every start.** Anything not on `/data`
is destroyed on every wake, silently.

## Verify locally

```sh
./deploy/fly/node/test-bootstrap.sh
python3 -c "import tomllib;tomllib.load(open('deploy/fly/node/fly.toml','rb'))"
docker run --rm -v "$PWD/deploy/fly/node:/mnt:ro" -w /mnt koalaman/shellcheck:stable \
  -s bash -S style entrypoint.sh bootstrap.sh test-bootstrap.sh verify-image.sh
docker run --rm -i hadolint/hadolint hadolint --failure-threshold info - < deploy/fly/node/Dockerfile
docker build --check -f deploy/fly/node/Dockerfile .

# and the images themselves — both of these build
docker build -f deploy/fly/node/Dockerfile -t workspacer-node-base:dev .
docker build -f deploy/fly/node/example.Dockerfile \
  --build-arg WKS_BASE=workspacer-node-base:dev \
  -t workspacer-node-example:dev deploy/fly/node
```

The **image builds green** as of 2026-08-24 (base 900 MB), and `test-bootstrap.sh`
passes 63/63 both on the host and inside the image as `wks` on an empty volume.
It has still never been **deployed** — RUNBOOK.md §12 lists exactly what that
leaves unverified.
