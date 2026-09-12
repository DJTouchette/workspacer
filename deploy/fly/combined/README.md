# One machine, stopped when idle

This topology runs hub + brain + claudemon together using `workspacer serve`.
Fly Proxy stays available while the machine is stopped. Opening `/m`, `/app/`,
or a desktop connection to `https://<app>.fly.dev` wakes it via HTTP, including
the WebSocket handshake. No separate hub or Fly token in the browser is needed.

Build from the repository root, using the same revision for every layer:

```sh
docker build -f deploy/fly/node/Dockerfile -t workspacer-node-base:dev .
docker build -f deploy/fly/hub/Dockerfile -t workspacer-hub:dev .
docker build -f deploy/fly/combined/Dockerfile -t workspacer-combined:dev .
```

Use `fly.toml` as a new deployment template: choose an app name, provision its
`wks_data` volume in the selected region, and deploy **one** machine. Publish the
locally built image to your registry and deploy that image (or make the base
images available to the remote builder). This does not migrate existing volumes
or alter an existing split hub/node deployment.

Set `FLY_API_TOKEN` as a Fly secret using an app-scoped token for this app. Fly
provides `FLY_APP_NAME` and `FLY_MACHINE_ID`. Set `HUB_TOKEN` as a secret for a
stable pairing credential, or read the generated token from
`/data/home/.config/workspacer/remote-token` over an authenticated console.
Never put tokens in `fly.toml`. The CLI ready banner is written to the private
`/data/logs/serve-ready.json`, not stdout. Install/sign in to agent providers as
with the node image; their home and repositories must live on `/data`.

For an existing combined deployment, rebuild its hub and web assets and set:

```toml
[env]
  WKS_MACHINE_POWER = "fly"
  WKS_MACHINE_WAKE = "http"
  WKS_MACHINE_IDLE_TIMEOUT = "15m"
  WKS_MACHINE_IDLE_MODE = "observe"

[http_service]
  internal_port = 7895
  force_https = true
  auto_start_machines = true
  auto_stop_machines = "off"
  min_machines_running = 0
```

The hub must listen on `0.0.0.0:7895` and trust the proxy hostname. A Tailscale
address on the sleeping machine does not pass through Fly Proxy and cannot wake
it. Flycast is an alternative private proxy route; clients must be able to reach
it. Public proxy requests can wake the machine **before** Workspacer checks the
pairing token, so an HTTP wake endpoint is not a promise that only your requests
can start billing. External uptime monitors and older reconnecting clients can
also keep it running.

## Stop and Wake

Operator clients get **Stop server** in desktop remote mode, `/app`, and `/m`.
Stop ends in-flight work; it is not suspend. The provider receives SIGTERM and a
45-second drain window. Open clients disconnect with WebSocket code 4001 and
pause reconnects until **Wake server** is selected. The main app is unmounted
while paused so its pollers and embedded plugins stop requesting the server.
Pause is remembered locally for that server. A fresh page navigation still goes
through the proxy and may wake the machine before JavaScript loads.

The disconnected screen confirms the stop *request*, not observed power state:
the hub cannot report its own completed shutdown. If the provider refuses the
stop after disconnecting, reconnect to see the error and retry. CPU/RAM charges
stop once the VM is stopped; root filesystem, volume and other allocated
resource charges remain. Stop/Wake restarts the services and restores persisted
history, not running agent processes.

## Automatic stop

`WKS_MACHINE_IDLE_TIMEOUT=15m` selects the quiet period. Set
`WKS_MACHINE_IDLE_MODE=observe` to measure without shutting down (the default),
`stop` to enable automatic shutdown, or `off` to disable the detector. Unset,
`off`, or `0` timeout also disables it; values below 10 minutes are rejected.

The server samples every 30 seconds and rechecks immediately before stopping.
It stays up for working/unknown sessions, approvals/questions, background tasks,
recent user input or commands, unreadable fleet/peer state, and **any running or
scheduled job**, including shell jobs. Jobs have no external wake scheduler yet.
Updated clients report taps, typing and scrolling with a throttled `activity`
frame. Allowlisted read-only calls do not refresh their interaction clock;
mutations and unknown methods still do. Old clients retain conservative activity
counting. The detector waits a sample interval after input, then measures the
quiet period, so 15 minutes is approximately 15–16 minutes after last activity.
Unknown state keeps the machine up. A restart begins a fresh quiet period.

Inspect the actual detector with **Idle** in mobile, the desktop/web idle status,
or `workspacer fleet idle --json`. The CLI reports mode, elapsed quiet time, and
blockers. Reading the status does not itself reset an updated client's clock.

This uses the hub's existing fleet-quiescence observations, not system CPU
usage. Processes outside Workspacer's tracked sessions/jobs are not covered.
The provider interface and client actions are OS-independent; this initial
implementation supplies Fly stop + HTTP wake, not laptop Wake-on-LAN.


## Upgrading the existing isolated personal deployment

The live `workspacer-node` image already has a combined root supervisor, hub
UID 10002 at `/data/hub/home`, worker UID 10001 at `/data/home`, and a migrated
Tailscale identity. **Do not replace that deployment with the generic entrypoint
above.** `build-upgrade.sh` layers new hub/brain/CLI/web assets on its existing image
and applies a narrow, checked supervisor patch. Its boot manifest and isolation
checks remain in place. The Fly machine ID/app are checked against the image's
power configuration to prevent a clone stopping the wrong target.

`stage-power-token.py APP` creates an app-scoped one-year credential and stages
it as a base64 Fly secret without printing it. `deploy-upgrade.py APP MACHINE
IMAGE` backs up the stopped machine's configuration and updates only its image,
restart policy, proxy autostart and a secret-file reference. The root supervisor
makes `/run/workspacer-power-token` readable only by the hub UID; worker/agent
environments receive no Fly credential. Resources and mounted volumes survive.

This deployment uses `WKS_MACHINE_WAKE_URL=https://APP.fly.dev/health`: the public
port 8080 serves only a wake doorbell, while `/m` and `/app` remain on Tailscale.
A shared public IPv4 is required for that doorbell. Clients remember its URL
while connected, issue a credential-free HTTP request on Wake, then reconnect to
Tailscale. For a first visit while stopped, open the public wake URL first.


### Deployment verified 2026-09-11

- App `workspacer-node`, machine `1857645df24448`, volume
  `vol_r1j3gge056epwxzr` (10 GB), existing 4 CPU / 8 GB size preserved.
- Image `registry.fly.io/workspacer-node:power-observe-20260911-v2`, digest
  `sha256:3e1bfbb59c65bcc915a00dd524aa0e678c1a4bca1d73fa2fe61c47e0acc2e3e7`.
- Idle mode **observe**, 15-minute quiet period. Automatic shutdown is disabled.
- HTTPS request to `https://workspacer-node.fly.dev/health` started the stopped
  machine and returned 200. Public `/m` returned 404.
- Hub and worker health checks returned 200. Power token is mode 0600, owner
  10002; hub UID can read it and worker UID 10001 cannot. Cloud state lookup
  with that token succeeded.
- Existing private app: `https://workspacer-hub.tail65dbc5.ts.net/app/`; mobile:
  `https://workspacer-hub.tail65dbc5.ts.net/m`.
- Old separate `workspacer-hub` machine remains stopped.

Run `python3 deploy/fly/combined/verify-upgrade.py workspacer-node 1857645df24448`
for these read-only checks. To inspect the idle report directly over SSH, use
`env -u HUB_TOKEN XDG_CONFIG_HOME=/data/hub/home/.config workspacer fleet idle --json`.
The `-u HUB_TOKEN` matters: Fly SSH inherits the old worker/provider app secret;
without clearing it the CLI overrides the hub's persisted host token and is
correctly denied `machine.power`.

The installed flyctl requires `--autostop=off` (the spaced form treats `off` as
another positional argument). Its image resolver also duplicates explicit
`@sha256:` references during machine update; use the unique versioned tag and
verify the resolved digest afterward. `flyctl auth docker` credentials expire
in five minutes, so authenticate immediately before pushing.


### Operator pairing update, 2026-09-11

The subsequent `operator-pairing-20260911` image (digest
`sha256:e3d76d305c5b196ea36da20295edac95c20d235b7315e79b0049c2742b9f497d`)
adds owner-authorized pairing management to the web phone dialog, correct `/m`
and `/app/` pairing URLs, and explicit token scope reporting. It preserves idle
observation mode. `verify-pairing.py APP MACHINE` checks that the persisted owner
credential reports operator scope and can manage pairings, without printing it.
Scoped operator tokens retain full operator actions but cannot administer the
owner's pairing store or gain extra grants by minting credentials.


### Mobile spawn update, 2026-09-11

Deployed `mobile-spawn-20260911-v2`, digest
`sha256:80ca5583361f680c9c3207d99d0b8c79426802929455f2cfdfc6b7ee8b7c0322`.
Mobile dispatch now offers configured projects, server folder browsing, provider
model/context-window selection, reasoning effort, editable instructions, and
operator full access using the router's `hello.spawnFullAccess` result.

Live read-only verification confirmed full access, three configured projects,
six Claude model aliases, and successful browsing of `/data/repos` with its two
project directories. The brain and desktop admit configured inactive projects
only for spawn folder/model discovery; general content access remains confined
to active workspaces. This update includes the brain binary for that fix.
Idle shutdown remains in observation mode. Manual Stop/Wake remains available
in the mobile top bar for operator pairings.

### Idle stop and web audit update, 2026-09-12

Automatic idle stop is now **enabled**, with the existing 15-minute quiet
period and 30-second sampling. Live work, pending attention and unknown state
continue to block stopping. Recent client input also defers the quiet period.

Current image: `registry.fly.io/workspacer-node:web-parity-20260912`, digest
`sha256:84619a2c8c5c86f0036438ba5d48c53b0925e0c51d15f9d7ca79d44f8381aa9c`.
It adds web plugin administration by replacing only browser assets on the
policy-only `idle-stop-20260912` layer. Hub/brain/worker binaries remain those
from `mobile-spawn-20260911-v2`.

The isolated supervisor sets idle mode from `/opt/combined/power.json`, after
clearing its environment; setting a Fly environment variable alone does not
override this policy. For a future policy change, build a layer with:

```sh
python3 deploy/fly/combined/build-idle-mode.py BASE_IMAGE NEW_IMAGE APP MACHINE stop
```

Use `observe` or `off` instead to change that policy. The builder verifies the
app/machine identity and preserves timeout/wake configuration. For a web-only
code update, `bash deploy/fly/combined/build-web-upgrade.sh BASE_IMAGE NEW_IMAGE`
builds and replaces only the browser bundle. Publish the resulting image and
use `deploy-upgrade.py ... --restart-idle` to apply it after the live-work guard.

`verify-web-capabilities.py APP MACHINE --expect-plugin-admin` checks the served
plugin bundle, reads actual method registrations, and runs an allowlist of
read-only probes without printing credentials or application content. See the
[web parity audit](../../../docs/web-parity-audit-2026-09-12.md) for the remaining
functional gaps; browser and headless desktop parity is not complete.

### Desktop parity upgrade

`build-parity-upgrade.sh CURRENT_IMAGE OUTPUT_IMAGE` preserves the existing
isolated supervisor, identities, persistent volume and power policy while shipping
current hub/brain/MCP/claudemon binaries, `desktop-host.cjs`, the web app and bundled
examples. The companion requires Node.js **22.13+** (the image build checks this).
The first-run defaults are the same webview-only editor and timeline as native;
existing plugin sets are preserved.

The root network helper exposes only this node's fixed Tailscale HTTPS proxy over
a socket owned by hub UID 10002, mode 0600, plus a random private credential
stored under the protected hub config directory. It runs with the supervisor's existing
capability-dropping launcher; the parent sets socket ownership. Worker UID 10001
cannot access it or the root Tailscale socket. The proxy's enabled state persists
across idle stops. Shell-job administration now requires the actual owner token.

Before deployment, run the disposable image/browser check:

```sh
node apps/desktop/scripts/verify-parity-image.mjs OUTPUT_IMAGE
python3 deploy/fly/combined/parity-preflight.py workspacer-node 1857645df24448
```

Once checks pass, `parity-preflight.py ... --provision` backs up and adds only the
approved operator MCP service's `facadeAuthority` provenance grant, and marks
an **empty** job registry for the new owner-only scheduler. The legacy jobs flag
stays disabled, so an older rollback image cannot accidentally enable its older
authorization policy. It refuses active/unknown work, non-operator MCP scope,
and dormant job definitions that could unexpectedly run. It never prints bearer
credentials and preserves token-store ownership. Profile-account grants remain
independent. Deploy with the existing idle-guarded `deploy-upgrade.py`; do not
restart an active or unknown fleet. Keep the old image/config backup for rollback.
