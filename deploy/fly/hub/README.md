# `deploy/fly/hub` — the always-on Fly.io hub

A single Fly Machine that runs `hub --brain-scope off`, never sleeps, holds the
Fly API token, and serves the node registry, `/m` and `/app` over Tailscale.

It is the other half of [`deploy/fly/node`](../node). The node sleeps; something
has to be awake to receive "wake node X" from a phone. That cannot be the node —
a sleeping hub cannot wake anything — and it should not be a desktop, because
then waking from a phone only works while the desktop is on.

**Start with [RUNBOOK.md](RUNBOOK.md).**

| File | What it is |
|---|---|
| `RUNBOOK.md` | Provisioning, the reachability argument, the persistence map, and what is *not* verified |
| `Dockerfile` | A short layer on the node base: the `hub` binary, `dist/web`, bubblewrap, and this entrypoint. `--build-arg WKS_INSTALL=artifact` takes `hub` + `web` out of a release bundle instead, skipping both the Go stage and the (slow) Vite build |
| `Dockerfile.dockerignore` | Build context is the repo root; this keeps it to `services/hub` + `apps/desktop` |
| `fly.toml` | Region, sizing, volume, `restart.policy = "always"`, and **no `[http_service]`** — each with the reasoning inline |
| `entrypoint.sh` | PID 1: boot log → bootstrap → tailscaled → `tailscale serve` → `hub`, plus a loopback health watchdog |
| `bootstrap.sh` | Prepares the volume **and decides FIRST RUN vs STATE LOSS per file**. Creates zero symlinks |
| `test-bootstrap.sh` | 113 assertions over `bootstrap.sh`. No root, no Docker, no Fly, ~1 second |

## The three things to know before editing anything here

**1. A hub that restarts without its state does not fail — it silently becomes a
different hub.** `remote-token` re-minted refuses every paired client while the
banner says healthy; `vapid.json` regenerated kills every push subscription while
every phone still reports itself subscribed; `nodes.json` missing is *not an
error* to the hub, which simply never registers `nodes.wake`. `bootstrap.sh`
exists to tell those apart from a genuine first run, and it **refuses to start**
for the first and third. A `bootstrap` exit 2 is the design working.

**2. It is reached over the tailnet only, and there is no public IP.** The node's
runbook refuses one because the Fly proxy wakes a machine before authenticating,
so a public doorbell spends money. That reasoning does **not** transfer — this
machine is always on, and an inbound request starts nothing. A different and
stronger reason replaces it: this is the machine holding a credential that spends
money, and a public endpoint would put the pairing token into a `?token=` query
string traversing Fly's proxy logs. RUNBOOK.md §5 works it through, §11 is the
deliberate opt-in.

**3. `--brain-scope off` is required, not preferred.** The node registry probes
liveness with `brain.info`, which cannot tell a local brain from a remote one, so
a hub supervising its own brain reports a stopped node as `available` forever.
The hub logs a warning and does not enforce it; these artifacts do.

## Verify locally

The base image must be built first — this is a downstream layer.

```sh
docker build -f deploy/fly/node/Dockerfile -t workspacer-node-base:dev .
docker build -f deploy/fly/hub/Dockerfile  -t workspacer-hub:dev .

./deploy/fly/hub/test-bootstrap.sh
python3 -c "import tomllib;tomllib.load(open('deploy/fly/hub/fly.toml','rb'))"
docker run --rm -v "$PWD/deploy/fly/hub:/mnt:ro" -w /mnt koalaman/shellcheck:stable \
  -s bash -S style entrypoint.sh bootstrap.sh test-bootstrap.sh
docker run --rm -i hadolint/hadolint hadolint --failure-threshold info - < deploy/fly/hub/Dockerfile
docker build --check -f deploy/fly/hub/Dockerfile .
```

Artifact mode has its own offline suite and its own preflight stage — see
[`../node/BASE_IMAGE.md` § Build arguments](../node/BASE_IMAGE.md#build-arguments)
and `deploy/fly/RUNBOOK.md` § D1b:

```sh
./deploy/fly/test-fetch-release.sh   # 51 assertions, no network
./deploy/fly/preflight.sh artifact   # rebuilds the node image from a fixture release
```

Build the base and the hub from the **same** `WKS_RELEASE_TAG`. Nothing forces
it — `WKS_BASE` is an opaque image reference — so `verify-image.sh` compares the
two stamps and fails the build when two artifact installs name different commits.

**This image has been built and smoke-tested**, unlike anything else under
`deploy/fly/`: the build passes the base's `verify-image.sh`, and the hub has
been started inside the container against a real volume mount. RUNBOOK.md §12
lists exactly what that did and did not establish.

## Known, and deliberately not fixed here

**The base is a worker-node base.** Inheriting it puts `claudemon`, `brain` and
Claude Code on the always-on machine, none of which this deployment runs and none
of which has a credential to find here (`bootstrap.sh` creates no `~/.claude`, no
`~/.claude.json`, no `~/.ssh`). It is ~900MB of the 919MB image. Accepted for now
because sharing the base is what keeps the Tailscale setup, the uid, the volume
contract and `verify-image.sh` identical across both machines — which are the
things that actually break when they drift. The fix is upstream: a build ARG on
`deploy/fly/node/Dockerfile`, or a `workspacer-hub-base` beside it.
