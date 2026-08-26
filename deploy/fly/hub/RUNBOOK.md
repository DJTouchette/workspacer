# Workspacer always-on hub on Fly.io — provisioning runbook

**Written:** 2026-08-25. **Artifacts:** `deploy/fly/hub/`.

> **Provisioning both machines? Start at [`../RUNBOOK.md`](../RUNBOOK.md).** This
> document is the reference for *why the hub is shaped the way it is*, and §8
> assumes a node that already exists. The combined runbook is the order, and it
> gathers the steps only a human can do into one sitting.

The exact sequence a human runs **once**, in order, to turn an empty Fly account
into an always-on hub that a sleeping worker node attaches to and a phone can
wake it from.

> The single most important instruction in this document: **step 10, before you
> depend on this machine for anything.** A hub that loses its volume does not
> fail — it comes up healthy and refuses every client you have ever paired.

---

## 0. What you are building

```
        phone (/m)          browser (/app)          desktop
             │                    │                    │
             └────────────── tailnet ──────────────────┘
                                │  https://hub.<tailnet>.ts.net
                                │  (tailscale serve → 127.0.0.1:7895)
                                ▼
                    ┌───────────────────────────┐
                    │  ALWAYS-ON HUB (this app) │      Fly Machines API
                    │  hub --brain-scope off    │ ───────────────────────┐
                    │  nodes.json  (topology)   │   (public internet,    │
                    │  $FLY_API_TOKEN (secret)  │    outbound only)      │
                    │  /data volume, 1GB        │                        ▼
                    └───────────────────────────┘            ┌──────────────────┐
                                ▲                            │  Fly worker node │
                                └── brain dials IN ──────────│  asleep by default│
                                    over the tailnet         └──────────────────┘
```

**One process.** `hub` and nothing else — no claudemon, no brain, no Claude Code.
The machine that is always on and holds a credential which spends money is
deliberately the machine with the least on it: it has **no Claude OAuth, no SSH
keys and no repo checkouts**, and `bootstrap.sh` does not even create the
directories for them.

**The node reaches this hub, not the other way round.** The node's `brain` dials
`wss://hub.<tailnet>.ts.net/bus` and registers as an ordinary capability
provider. This hub never dials the node; the only outbound call it makes to the
node's *world* is the Fly Machines API `start`, over the public internet.

---

## 1. Decisions already made, and why — do not re-litigate these

| Decision | Why |
|---|---|
| **`hub --brain-scope off`** | **Required, not preferred.** The node registry's liveness probe is `brain.info`, and it cannot tell a LOCAL brain from a REMOTE one. A hub supervising its own brain reports a stopped node as `available` forever, and every wake looks like it already happened. `cmd/hub/nodes.go` logs a warning about exactly this pairing and does not enforce it. Verified: running `--brain-scope full` with a registry present emits that warning; running `off` does not. |
| **Region `ord` (Chicago)** | Same as the node, measured: 35.4 ms median from Calgary, ahead of `yyz` (~40 ms) with no overlap in the min/max ranges. It also keeps the hub↔node hop inside one region. **`sea` and `den` no longer exist** — retired in Fly's September 2025 Region Consolidation. **The volume pins the region permanently.** |
| **Tailnet-only, no public IP** | See §5. The node's reason does not transfer; a different and stronger one replaces it. |
| **`tailscale serve` for TLS** | Not a convenience. `/m` is a PWA whose value here is background Web Push, and both `navigator.serviceWorker` and `PushManager` require a **secure context**. `tailscale serve` supplies a real Let's Encrypt certificate for the MagicDNS name with no public IP and nothing to renew. |
| **Non-ephemeral, reusable, pre-authorized, tagged auth key** | An ephemeral node is removed from the tailnet when it goes offline and gets a **new address** when it rejoins — and this hub's address is what the sleeping node's `HUB_BUS_URL` resolves to. Tagging is load-bearing: node keys expire after **180 days** by default, but a device tagged at first authentication has key expiry disabled. |
| **Debian base, not Alpine** | Inherited from the node base image. Kernel-mode `tailscaled` needs nftables, and Alpine is where that fails confusingly on Fly. |
| **`restart.policy = "always"`** | The **opposite** of the node's `on-failure`, deliberately. The node's design makes `stopped` a wanted state; this machine has no legitimate stopped state at all — it *is* the wake path, and a hub that is off cannot be woken by anything, including itself. |
| **`$FLY_API_TOKEN` rather than a token in `nodes.json`** | See §8. The credential then lives in Fly's secret store instead of on the volume, so a volume snapshot does not copy it and rotation does not mean editing a file inside a running machine. |
| **Non-root (`wks`, uid 10001)** | Every state file on the volume is 0600 owned by 10001, and an always-on daemon reachable by anything has no business being root. |

---

## 2. Prerequisites

- `flyctl` installed and authenticated (`fly auth whoami`).
- A Tailscale tailnet with admin access, and **HTTPS Certificates enabled**
  (admin console → DNS → HTTPS Certificates). `tailscale serve` cannot get a
  certificate without it, and this hub has no other way in.
- This repo checked out. **All commands below are run from the repo root.**

Build the base image and this one locally first, so a build failure is not
discovered during a deploy:

```sh
docker build -f deploy/fly/node/Dockerfile -t workspacer-node-base:dev .
docker build -f deploy/fly/hub/Dockerfile  -t workspacer-hub:dev .
```

---

## 3. Create the app — WITHOUT public IPs

**Do not use `fly launch`.** It allocates public IPs, and this app should not
have one. See §5.

```sh
fly apps create workspacer-hub
```

If you change the app name, change `app =` in `deploy/fly/hub/fly.toml` to match.

---

## 4. Tailscale: tag first, then the key

1. In the admin console → **Access controls**, add the tag owner and an ACL. The
   hub needs to be reachable *by* you and *by* the node, and needs to reach
   nothing on the tailnet itself:

   ```jsonc
   {
     "tagOwners": {
       "tag:workspacer-hub":  ["autogroup:admin"],
       "tag:workspacer-node": ["autogroup:admin"]
     },
     "acls": [
       // the node reaches the hub's bus, and nothing else
       { "action": "accept", "src": ["tag:workspacer-node"],
         "dst": ["tag:workspacer-hub:443,8443"] },
       // your own devices reach the hub — this is how the phone gets /m
       { "action": "accept", "src": ["autogroup:member"],
         "dst": ["tag:workspacer-hub:443,8443"] }
     ]
   }
   ```

   Note the ports: with `tailscale serve` terminating TLS, clients reach **443**
   (and **8443** for the plugin origin, §9), not 7895. The hub binds 7895 on
   loopback only, so nothing on the tailnet can reach that port directly.

2. **Settings → Keys → Generate auth key**, with:
   - **Reusable: yes**
   - **Ephemeral: NO** ← the one that matters
   - **Pre-approved: yes** (if device approval is on for your tailnet)
   - **Tags: `tag:workspacer-hub`**

3. After first boot, **confirm in the admin console that the device shows key
   expiry as disabled**, and that it has a certificate. One click against a
   failure that would otherwise surface in six months with no useful signal.

---

## 5. THE REACHABILITY DECISION, AND WHY IT IS NOT THE NODE'S

This is the one place where copying `deploy/fly/node` would have been wrong.

**The node's reasoning.** `deploy/fly/node/RUNBOOK.md` step 3 provisions no public
IP because the Fly proxy **starts a machine before routing the request**, so no
application-level auth on the doorbell can stop a stranger from spending your
money — the wake has already happened by the time your code sees anything. The
only real control is whether the app is publicly routable at all.

**That argument does not transfer.** This machine is always on. An inbound request
starts nothing and costs nothing extra, because it is already running and already
billing. The premise the node's rule rests on is simply absent here.

**A different argument replaces it, and for this machine it is stronger.** Three
things, in order of how concrete they are:

1. **The pairing token would be written into Fly's request logs.** Browsers cannot
   set headers on a WebSocket handshake, so `/bus`, `/m` and `/app` all accept
   `?token=<token>` — and `presentedToken` is built for exactly that. On a
   tailnet that query string traverses your own devices. Behind a public Fly IP it
   traverses Fly's proxy, which logs requests. That is the pairing credential for
   every client you own, in a third party's logs, on every visit.
2. **This is the only machine holding a credential that spends money.** The node's
   worst case for a public doorbell was "a stranger burns $0.06/hr". The hub's is
   "a stranger is talking to the control plane". `nodes.wake` and `nodes.sleep`
   are host-authority only — `nodesTrusted` refuses the view and triage tiers,
   though **not** an operator-tier scoped token, which `bus.go` marks trusted
   (see §11) — so the token is a real boundary, but it is one boundary, and the
   failure is expensive. Note that the sleep verb does not soften this: whoever
   can turn a machine on can also turn one off underneath whoever is using it.
3. **The unguarded surface is real, if small.** `/m`, the PWA manifest, the service
   worker, the icons, `/plugins/origin` and `/plugins/ui/<id>/` are served without
   the host token by design (a `<script>` URL cannot carry it). None of them
   discloses session state, but they are the attack surface, and they are exactly
   the part no token protects.

**And the tailnet is not a workaround — it is already a hard dependency.** The node
reaches this hub over it. Adding a public path for the phone means maintaining two
trust boundaries where one already exists and already works. It also supplies the
TLS certificate that `/m`'s service worker and Web Push **require**; a public Fly
endpoint would give you TLS too, but not for free in exposure.

**So: tailnet-only.** The phone runs the Tailscale app (always-on, negligible
battery), opens `https://hub.<tailnet>.ts.net/m?token=…`, installs it as a PWA,
and receives push. The browser opens `/app` the same way.

**How it is enforced, rather than merely intended.** `fly.toml` sets
`WKS_HUB_BIND = "127.0.0.1"` and has **no `[http_service]` block**. The Fly proxy
cannot route to a loopback listener, so public exposure takes a bind change **and**
an IP allocation **and** a `--trusted-host` — three deliberate acts, not one
forgotten flag. §11 is that path, written down, for an operator who wants it.

**What it costs.** Anyone who needs `/m` must be on your tailnet. Sharing a session
with someone who is not means adding them to the tailnet or using §11.

---

## 6. Set the secrets

```sh
fly secrets set --app workspacer-hub --stage \
  TAILSCALE_AUTHKEY='tskey-auth-…' \
  FLY_API_TOKEN='FlyV1 fm2_…'
```

`--stage` holds them until the first deploy rather than triggering one.

**`HUB_TOKEN` is optional and you have a choice to make.** If you do not set it,
`bootstrap.sh` mints one on first boot and prints it in the boot log. If you set
it, it is authoritative and gets mirrored onto the volume. **Setting it is
better**, for one reason: the credential then lives in Fly's secret store rather
than only on the volume, so losing the volume does not lose your pairing, and
rotating it is `fly secrets set` rather than an edit inside a running machine.

Either way, `bootstrap.sh` refuses to start if `$HUB_TOKEN` **disagrees** with a
credential already on the volume — that is the same silent identity change by
another route. `WKS_ALLOW_TOKEN_CHANGE=1` is the deliberate override.

**The Fly token.** Create it scoped to the NODE's app, not the hub's, and not
org-wide:

```sh
fly tokens create deploy --app workspacer-node --expiry 2160h --name wks-hub-wake
```

Three things about it, all confirmed by the infra scout:
- **There is no action-scoped token.** No start/stop-only token exists; read-only
  exists only at org level and cannot start a machine. Treat this as *full control
  of that app, including creating machines*.
- **The default expiry is 20 years.** Set one. `2160h` is 90 days.
- **Scope it to the node's app** so the blast radius is one app's worth of
  machines. That is still real money, but bounded.

---

## 7. First deploy — this creates the volume

```sh
fly deploy \
  --config deploy/fly/hub/fly.toml \
  --dockerfile deploy/fly/hub/Dockerfile \
  --app workspacer-hub \
  .
```

The build context is the repo root deliberately: the image builds `hub` from
`services/hub` and the `/app` bundle from `apps/desktop`. **It builds `FROM`
`workspacer-node-base:dev`, so `fly deploy` must use a builder that can see that
image** — `--local-only` with the base built locally, or push the base to a
registry first and pass `--build-arg WKS_BASE=…`. See `deploy/fly/node/BASE_IMAGE.md`
§ "Identity and tagging"; the base has no published home yet, and that is an open
decision, not an oversight.

To skip the `/app` bundle (a smaller, faster image; `/m` still works, since it is
compiled into the hub binary):

```sh
fly deploy … --build-arg WKS_WITH_WEBAPP=0
```

Set snapshot retention deliberately. **This volume is the fleet's single point of
failure** — it holds the pairing credential every client is paired against, the
Web Push keypair every phone subscribed to, the node registry that is the only
wake path, and the tailnet identity the node resolves:

```sh
fly volumes list --app workspacer-hub
fly volumes update <vol_id> --snapshot-retention 30
```

### Read the boot log

```sh
fly logs --app workspacer-hub
```

You want, in order:

```
entrypoint: BOOT <id>
bootstrap: FIRST BOOT on this volume — created 13 directories, layout v1.
bootstrap:   remote-token: absent, and nothing on this volume says it ever existed — first run.
bootstrap: state guard: no losses detected
bootstrap: FIRST RUN: minted a new pairing credential at /data/home/.config/workspacer/remote-token
bootstrap:   HUB_TOKEN=<32 chars>          ← WRITE THIS DOWN
entrypoint: TAILNET UP after Ns — ipv4=100.x.y.z
entrypoint: MagicDNS name: workspacer-hub.<tailnet>.ts.net
entrypoint: tailscale serve --bg --https=443 → http://127.0.0.1:7895
entrypoint: hub ready (pid …)
entrypoint:   bus     wss://workspacer-hub.<tailnet>.ts.net/bus
entrypoint: BOOT COMPLETE <id>
```

**Write down the token and the MagicDNS name.** The node's `HUB_BUS_URL` is the
bus line verbatim.

The same log is on the volume at `/data/logs/boot.log` (appended, capped at 5 MiB)
and `/data/logs/last-boot.log`. Fly retains logs for 7 days; a restart loop that
began nine days ago with one interesting line at the top is exactly what that
drops.

---

## 8. The node registry, and where the Fly token lives

`nodes.json` is hand-written and **the hub only ever reads it** (pinned upstream by
`TestTheHubNeverWritesTheNodeRegistry`). Create it as `wks`:

```sh
fly ssh console --app workspacer-hub
su - wks
cat > ~/.config/workspacer/nodes.json <<'EOF'
[
  {
    "id": "fly-node",
    "label": "Fly node (ord)",
    "fly": {
      "app": "workspacer-node",
      "machineId": "17811944b12345"
    }
  }
]
EOF
chmod 600 ~/.config/workspacer/nodes.json
exit
fly machine restart <machine_id> --app workspacer-hub
```

**Note what is not in that file: the token.** `nodes.ResolveToken` looks at the
entry's inline `token`, then `tokenFile`, then **`$FLY_API_TOKEN`** — and the env
is the right answer for a hub deployed *on* Fly:

- It is not on the volume, so a volume snapshot does not copy it.
- Rotation is `fly secrets set` + restart, not an edit inside a running machine.
- `nodes.json` is then pure topology, and losing 0600 on it is a much smaller
  event. (`bootstrap.sh` repairs the mode anyway, before the hub starts.)

Confirm it took, in `fly logs`:

```
nodes: 1 node(s) registered from …/nodes.json (1 wakeable)
```

**`(0 wakeable)` means the token did not resolve** and the hub can report the node
but never wake it — which is this machine's only job. The hub names the fix in the
preceding line; `entrypoint.sh` also warns before startup if a registry exists and
no credential is resolvable.

You should **not** see the `--brain-scope` warning. If you do, something is passing
a scope other than `off` and your node states will be wrong.

---

## 9. Plugins, and why none ship here

The hub is what loads plugins, so unlike on the node this surface is live —
`--plugins-dir` points at `~/.config/workspacer/plugins`, which is on the volume.
It is **empty by default**, and that is a decision rather than an omission: this is
the always-on machine holding a credential that spends money, and a plugin sidecar
is arbitrary code with a bus connection.

If you do install one, it runs confined. `WORKSPACER_PLUGIN_SANDBOX=enforce` is set
in `fly.toml`, and `enforce` **refuses to start a sidecar** on a platform with no
confinement mechanism rather than running it unconfined. The Linux mechanism is
bubblewrap, installed in this image for that reason.

**Unverified:** whether `bwrap` works inside a Fly Firecracker guest, which needs
unprivileged user namespaces. Because no plugin ships here, a wrong answer surfaces
as a clear refusal the first time someone installs one, not as a silent unconfined
sidecar. Fall back with `WORKSPACER_PLUGIN_SANDBOX=best-effort` if that happens.

`WKS_HUB_PLUGIN_ORIGIN_ENABLED=1` adds a second `tailscale serve` rule on **8443**
and passes `--plugin-origin`. Without a second origin, `/app` must frame a
hub-served plugin same-origin, which the browser sandboxes opaque — the plugin
paints and can talk to nothing. It is one of the three ports the ACL in §4 opens.

---

## 10. THE PROOF — restart, and check every claim

Nothing above is trustworthy until this passes.

```sh
# BEFORE
fly ssh console --app workspacer-hub -C 'tailscale ip -4'
fly ssh console --app workspacer-hub -C 'sha256sum /data/home/.config/workspacer/remote-token /data/home/.config/workspacer-hub/vapid.json'
fly ssh console --app workspacer-hub -C 'ls -l /data/state/seen/'

fly machine list --app workspacer-hub
fly machine restart <machine_id> --app workspacer-hub
```

| # | Check | Command | Pass |
|---|---|---|---|
| 1 | Tailnet IP is **identical** | `tailscale ip -4` | same 100.x.y.z |
| 2 | Same device in Tailscale | admin console | one device, key expiry disabled, cert present |
| 3 | Pairing credential unchanged | `sha256sum …/remote-token` | same hash |
| 4 | VAPID keypair unchanged | `sha256sum …/vapid.json` | same hash |
| 5 | Later boot took the right path | `grep 'populated volume' /data/logs/last-boot.log` | present; **no** `FIRST BOOT`, **no** `minted` |
| 6 | No state loss reported | `grep 'state guard' /data/logs/last-boot.log` | `no losses detected` |
| 7 | Registry still loaded | `fly logs` | `nodes: 1 node(s) … (1 wakeable)` |
| 8 | Phone still paired | open `/m` on the phone | no re-pair prompt |
| 9 | Push still works | trigger an agent-needs-you alert | phone receives it **without re-subscribing** |
| 10 | `/app` still loads | browser | renders without a new token |
| 11 | The node still attaches | wake it, watch `fly logs` | provider registers, `nodes.list` → `available` |

### Then prove the refusal, which is the whole point

Do this once, deliberately, on a hub with nothing depending on it yet:

```sh
fly ssh console --app workspacer-hub -C 'mv /data/home/.config/workspacer/remote-token /data/token.bak'
fly machine restart <machine_id> --app workspacer-hub
fly logs --app workspacer-hub          # expect STATE LOSS + REFUSING TO START, and a crash loop
fly ssh console --app workspacer-hub -C 'mv /data/token.bak /data/home/.config/workspacer/remote-token'
```

*(The machine is crash-looping, so `fly ssh console` may need a retry between
restarts. If it is unreachable, `fly machine stop`, then `fly machine start` after
restoring — or set `WKS_ALLOW_STATE_LOSS=1` as a secret to get a shell, and unset
it immediately after.)*

**Checks 3, 4 and the refusal are the ones that matter**, and they are the ones a
casual "does it come up?" test passes while being completely broken. A hub that
re-minted its token comes up healthy and refuses everything; a hub that regenerated
its VAPID keypair comes up healthy and every phone still says it is subscribed.

**Check 9 is genuinely hard to fake.** A push subscription negotiated against the
old public key is refused by the push service for those endpoints, and nothing
client-side reports an error. If you skip it you will not find out for weeks.

---

## 11. Optional: a public endpoint, deliberately

Only if you need `/m` or `/app` from a device that cannot join your tailnet, and
only after reading §5.

```sh
# 1. bind wider — the Fly proxy cannot reach a loopback listener
#    (edit fly.toml, or set it as a secret)
fly secrets set --app workspacer-hub WKS_HUB_BIND=0.0.0.0

# 2. add an [http_service] block to fly.toml:
#      [http_service]
#        internal_port = 7895
#        force_https = true
#        auto_stop_machines = "off"
#        auto_start_machines = true
#        min_machines_running = 1

# 3. tell the hub the name the proxy presents, or every route 403s
fly secrets set --app workspacer-hub WKS_HUB_TRUSTED_HOSTS=workspacer-hub.fly.dev

# 4. and only now, the IP
fly ips allocate-v6 --app workspacer-hub
fly ips allocate-v4 --shared --app workspacer-hub
```

**What authenticates it, precisely.** Not "it's behind HTTPS":

| Surface | Guard |
|---|---|
| `/bus` | `Server.Authorized` — the host token, or a scoped token whose grant is operator. Plus `originAllowed`. **This is the real boundary.** |
| `/app/` entry | `Authorized`. View/triage scoped tokens are refused. |
| `nodes.wake` | `nodesTrusted` — **host authority**. A view- or triage-tier token is refused, so a leaked phone-tier token cannot spend money. **An OPERATOR-tier scoped token is NOT refused**: `bus.go` marks operator-tier as `trusted`, and `nodesTrusted` asks `IsTrusted()`. Verified 2026-08-25 by calling `nodes.wake` with an operator-scoped token and watching the hub issue a real `POST /v1/apps/…/machines/…/start`. Mint operator tokens accordingly. |
| `nodes.sleep` | `nodesTrusted`, the same gate — and refused for a reason of its own rather than by symmetry: a stop lands on a machine somebody may be typing at and ends the work in flight on it. "It only turns things off" is destructive, not smaller. An operator-tier scoped token is likewise **not** refused, for the same reason as `nodes.wake`. |
| `/plugins/install`, `/plugins/examples/install`, `/plugins/reload` | `hostOnlyRoute` — **the host token itself**, not merely `Authorized`. These three run code on this machine (the manifest's install argv; a reload starts a sidecar from a caller-named directory), and an operator-tier scoped token — what every worker node carries — is answered 403. Interim gate: the durable fix is a provider tier so attaching as a capability provider stops implying host authority. |
| `nodes.list` | View tier. Discloses a label, a state and a timestamp — deliberately not the app, the machine id, the endpoint or the token. |
| `/m`, manifest, `sw.js`, icons, `/plugins/origin`, `/plugins/ui/<id>/` | **Unguarded by design.** Static shells; none discloses session state. This is the surface you are adding to the internet. |

**The token is 24 random bytes, base64url (192 bits).** Brute force is not the
risk. **Leakage is**, and a public endpoint adds a specific leak that the tailnet
does not have: `?token=` in Fly's proxy logs on every visit. If you take this path,
mint a **scoped** token for the phone (`workspacer token create --scope triage`)
rather than handing out the host token, and keep the host token off phones
entirely.

`min_machines_running = 1` and `auto_stop_machines = "off"` are not optional in
that block: without them the proxy may stop the machine that is your only wake
path.

---

## 12. What IS verified, and what needs a real machine

Unusually for `deploy/fly/`, the first list is not empty.

### Verified, by building and running the image on this machine

1. **The image builds**, from the merged `workspacer-node-base:dev`, and passes
   the base's `verify-image.sh` (`/data` empty, no `$HOME` toolchains, no stateful
   `ENV`, no `USER`, uid 10001, every daemon on PATH). 919MB.
2. **`hadolint --failure-threshold info`** and **`docker build --check`**: clean.
3. **`shellcheck -s bash -S style`** on all three scripts: clean.
4. **`test-bootstrap.sh`: 111 assertions, green**, run both on the host and inside
   the built image.
5. **`bootstrap.sh` on a real mountpoint** (a Docker volume at `/data`, running as
   root, chowning to 10001): first boot mints a 32-char base64url credential at
   mode 600 owned by `wks`, and reports `no losses detected`.
6. **The refusal fires.** Removing `remote-token` from that populated volume and
   re-running gives `STATE LOSS`, `REFUSING TO START`, **exit 2**.
7. **The hub starts and serves**, as `wks`, with the exact flags `entrypoint.sh`
   passes. Confirmed from its own startup log: `--brain-scope off` (and **no**
   registry warning), `nodes: 1 node(s) registered … (1 wakeable)` resolving the
   token from `$FLY_API_TOKEN`, `serving web app from … at /app/`,
   `layout document persisted at …`, `push: generated VAPID keypair at …`.
8. **The auth and Host pins behave**, against the loopback socket that
   `tailscale serve` will actually produce: `/health` 200, `/m` 200 unguarded,
   `/app/` **401 without a token and 200 with one**, `Host: hub.example.ts.net`
   (declared via `--trusted-host`) 200, `Host: evil.example.com` **403**.
9. **`--brain-scope full` really does warn** — the requirement in §1 is quoted
   from an observed log line, not asserted.
10. **The env-token design is load-bearing**: without `$FLY_API_TOKEN` the same
    registry reports `(0 wakeable)`.
11. **`nodes.json` mode repair** works (644 → 600, reported).
12. **The vapid pre-flight** warns and continues (exit 0), rather than refusing.

### NOT verified, because it needs a real machine

1. **Nothing has been deployed.** No machine, no Fly volume, no server-side
   `fly.toml` validation. The TOML parses and every key was checked against the
   node's working file, which is not the same as Fly accepting it.
2. **`tailscale serve` has never run here.** It cannot: it needs a real tailnet
   and a real certificate. This is the largest single unknown, and the hub is
   **unreachable if it fails** — which is why `entrypoint.sh` treats a serve
   failure as fatal and names the three likely causes. Specifically unconfirmed:
   whether a **tagged** device in your tailnet may fetch a Let's Encrypt
   certificate, and whether your `tailscale` build wants `--bg --https=443
   <target>` or the older `serve https:443 / <target>` syntax.
3. **Kernel-mode tailscaled on Fly** — inherited from the node's design, and
   settled by `ip link show tailscale0` on the first boot.
4. **Tailnet IP and MagicDNS stability across a restart.** §10 checks 1 and 2.
5. **Web Push end to end.** §10 check 9. A dead subscription is silent on both
   ends; this is the check most worth actually doing.
6. **Whether `bwrap` works in a Firecracker guest** (§9). Fails closed.
7. **Whether a stopped node's TCP connection is severed cleanly.** Inherited from
   the node's runbook §12.7 — if a dead machine goes silent rather than sending
   RST/FIN, the hub keeps a zombie provider registered. The wake contract's
   two-layer fix (hub-side eviction + brain re-registration) is built; it has not
   been exercised against a real stopped Fly machine.
8. **Cost.** The brief budgets ~$6/month for `shared-cpu-1x`/1GB always-on. **I
   did not verify Fly's current rate** — treat that as the brief's number, not a
   measured one. Add the volume (the node's runbook cites $0.15/GB/mo, so ~$0.15
   here) and note that snapshots bill separately.

### Day-one measurements

| Measurement | Why |
|---|---|
| `TAILNET UP after Ns` from the boot log | The one stage nobody publishes a number for, and it gates everything |
| Time from `fly machine restart` to `hub ready` | The hub's own downtime budget; the node is unreachable for all of it |
| Whether `tailscale serve` survives a restart | It should — the config is in `tailscaled.state` on the volume — but the entrypoint re-applies it every boot rather than trusting that |
| Memory at rest, and after a week | 1GB was chosen for the snapshot store, which grows with fleet size, not CPU |
| Whether check 9 (push) actually passes | Everything else fails loudly; this one does not |

---

## 13. Assumptions on work owned by other people

1. **The base image's identity.** `deploy/fly/node/Dockerfile` builds
   `workspacer-node-base:dev` and **has no published home yet** — an open decision
   recorded in `BASE_IMAGE.md`. Until it is made, `fly deploy` for this app must
   use a local builder or a base pushed to a registry (§7).
2. **`hub` is not in the base.** The base builds `brain`, `workspacer` and
   `claudemon`, because a worker node runs no hub. This image adds its own Go
   builder stage for it. If the base ever ships `hub`, that stage can be deleted.
3. **The client wake and sleep buttons.** `nodes.list`, `nodes.wake`,
   `nodes.sleep` and `node.state_changed` are built and this hub serves them, and
   the desktop, `/app` and `/m` buttons all exist. Note for anyone reading a
   `stopped` node: `sleptByHub` on the node view is IN-MEMORY ONLY, so a hub that
   has restarted honestly stops claiming it put the machine to sleep.
4. **A leaner hub base.** See README.md — this image carries `claudemon`, `brain`
   and Claude Code because the base does, and never runs any of them.
