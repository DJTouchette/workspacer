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
| `Dockerfile` | Multi-stage: Go builders for `brain`/`workspacer`, Rust for `claudemon`, Debian trixie runtime with the Go/Ruby/Bun/Node toolchains and Tailscale |
| `Dockerfile.dockerignore` | Build context is the repo root; this keeps it to the two services |
| `fly.toml` | Region, sizing, volume, restart policy, and the `auto_start_machines` wake backstop — each with the reasoning inline |
| `entrypoint.sh` | PID 1: boot log → bootstrap → doorbell → tailscaled → `claudemon init` → claudemon → brain, plus signal handling and exit-reason recording |
| `bootstrap.sh` | Prepares the volume. Idempotent, correct on both an empty and a populated volume, and creates **zero symlinks** |
| `test-bootstrap.sh` | 63 assertions over `bootstrap.sh`. No root, no Docker, no Fly, ~1 second |

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
  -s bash -S style entrypoint.sh bootstrap.sh test-bootstrap.sh
docker run --rm -i hadolint/hadolint hadolint --failure-threshold info - < deploy/fly/node/Dockerfile
docker build --check -f deploy/fly/node/Dockerfile .
```

Nothing here has ever been deployed. RUNBOOK.md §12 lists exactly what that
leaves unverified.
