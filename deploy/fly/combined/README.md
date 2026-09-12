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
