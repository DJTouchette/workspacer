# Remote Sharing — Threat Model & Security Notes

Workspacer can optionally expose its control plane (the hub bus + a mobile/web
client) beyond the local machine so you can drive your agents from your phone or
another PC. This document describes what that feature does, how it is meant to
be deployed, and its honest limitations.

**Scope and audience.** Workspacer is a personal / small-team tool, not a public
SaaS. The design below is calibrated to that: the protection model assumes a
trusted private network (a Tailscale tailnet) rather than a hardened public
endpoint. If your threat model is broader than "me and people I trust on my own
tailnet," read the [If you ever want to expose this beyond a
tailnet](#if-you-ever-want-to-expose-this-beyond-a-tailnet) section first —
the current feature does not meet that bar.

Relevant code:

- `apps/desktop/src/main/services/hubDaemon.ts` — the remote-share toggle, bind
  address, token, and `getRemoteShareInfo`.
- `services/hub/cmd/hub/main.go` + `services/hub/internal/bus/bus.go` — the bus,
  token auth, and which routes are guarded.
- `services/hub/internal/plugin/manager.go` +
  `services/hub/internal/sandbox/sandbox.go` — plugin sidecar confinement.
- `services/hub/cmd/mcp/main.go` — the MCP facade (loopback-only).

---

## 1. Default posture: loopback only

With remote sharing **off** (the default), the hub binds `127.0.0.1:7895`. It is
reachable only from the local machine — there is no network exposure. This is
the entire footprint until you explicitly opt in.

A bus token is still created and required even in this default mode (see §4), but
its job there is internal: it lets the bus distinguish the trusted host process
from plugin sidecars and webviews, each of which carries its own narrower
per-plugin token. Loopback binding, not the token, is what keeps the default
posture closed to the network.

The **MCP facade** (`services/hub/cmd/mcp`, `127.0.0.1:7897`) is always
loopback-only and is *not* affected by the remote-sharing toggle. It is intended
for a local MCP client (e.g. Claude Code via `--mcp-config`) on the same
machine.

---

## 2. What enabling remote sharing does

Toggling sharing on (UI: Remote control → Start sharing, persisted to
`<config>/remote-share-enabled`; or `WORKSPACER_REMOTE_SHARE=1` to force it on)
restarts the hub with three changes:

- **Binds `0.0.0.0:7895`** instead of loopback, so other hosts can reach it.
  You can pin the bind to a specific interface with
  `WORKSPACER_REMOTE_ADDR=host:port` (e.g. your tailnet IP) to avoid listening
  on your whole LAN.
- **Requires the shared token on `/bus`** and on the sensitive routes. Binding
  off loopback is meaningless without auth, so the token becomes mandatory for
  any remote client.
- **Serves the client pages**: `/m` (the mobile-first client, the default phone
  entry) and `/remote` (lightweight client). When a built web bundle is present
  (`--webapp-dir`), it also serves `/app/` — the *full* React renderer for true
  remote parity.

### What is guarded vs. not

Auth is checked by `Server.Authorized`, which accepts either an
`Authorization: Bearer <token>` header or a `?token=<token>` query param (browser
WebSocket handshakes can't set headers, so the query form is what clients use).

| Route | Guarded? | Note |
|---|---|---|
| `/bus` (WebSocket) | **Yes** | The real security boundary. Every event sub/pub and capability call flows here; the connection is rejected without a valid token. |
| `/m`, `/remote` | **Yes** | Token-guarded entry documents. |
| `/app/` entry (`index.html`) | **Yes** | Token-guarded. |
| `/plugins/install`, `/remove`, `/inspect`, `/setEnabled`, `/examples/install`, `/plugins/tokens`, `/plugins/pane-token[/revoke]` | **Yes** | Mutating / sensitive plugin routes. |
| `/health` | No | Liveness only (status + counts). |
| `/plugins`, `/plugins/examples` | No | List manifests that already ship in the app; no secrets. |
| `/plugins/ui/<id>/…` | No | Static plugin assets; `http.Dir` confines reads to the `ui` dir (no `..` escape, manifest/token never served). The real boundary remains `/bus`. |
| `/app/` hashed assets, `/xterm.js`, `/xterm.css`, `/addon-fit.js` | No | Public library/bundle code; `<script>`/`<link>` tags can't carry the token. Same rationale: `/bus` is where authorization actually happens. |

The model is deliberate: the **token gate on `/bus`** is the security boundary.
Anyone who reaches `/bus` with the **host token** is treated as the **trusted
host** and may call any capability. This is no longer the *only* option, though:
scoped **capability tokens** (see §6) let you hand out a narrower credential.
The default share link still embeds the host (operator) token, so the caveat
below still holds for the scan-and-go path unless you deliberately mint a scoped
token instead.

---

## 3. Intended deployment: over a Tailscale tailnet

Remote sharing is designed to run **over a Tailscale tailnet** (WireGuard-based
mesh VPN). The hub itself speaks plain HTTP/WS — **there is no TLS on the hub.**
The tailnet is what provides:

- **Transport encryption.** WireGuard encrypts traffic between your devices, so
  the token and all bus traffic are protected on the wire even though the hub
  serves `ws://`/`http://`.
- **Network-level access control.** Only devices in your tailnet can reach the
  hub's tailnet IP. `advertiseHost()` even prefers a Tailscale `100.64.0.0/10`
  address when building the share URL.

**Explicit guidance:**

- **Do** run it over a tailnet, ideally pinned to your tailnet IP via
  `WORKSPACER_REMOTE_ADDR`.
- **Do NOT** port-forward it, put it behind a public reverse proxy, or otherwise
  expose it to the public internet.
- **Do NOT** enable it on an untrusted LAN (café / coworking / conference Wi-Fi).
  Without the tailnet, the token and bus traffic travel in cleartext and anyone
  on that network who captures the link gains full control of the host.

---

## 4. The shared token

- **Strength & storage.** A 192-bit (24-byte) cryptographically random token,
  base64url-encoded, stored at `<config>/remote-token` with `0o600`
  permissions. Created once on first run and reused.
- **It is embedded in the link.** For convenience the QR code / share URL embeds
  the token as `?token=…` (e.g. `http://<host>:7895/m?token=…`). This is what
  lets you scan-and-go on a phone.
- **Treat the QR/link like a password.** Anyone who has it has full control of
  the host (see §5). In particular, because the token rides in the URL, it can
  land in **browser history, server logs, or referrer headers** on the receiving
  device. Don't paste it into shared chats, don't leave the QR on screen, and be
  aware that the receiving browser remembers it.
- **Rotation.** Toggling sharing off then on does not by itself mint a new token
  (the same `<config>/remote-token` is reused). To force a fresh token, delete
  `<config>/remote-token` and restart; a new 192-bit token is generated, which
  invalidates every previously shared link.

---

## 5. Honest caveats

These are real limitations, stated plainly:

- **No TLS on the hub.** Confidentiality and integrity on the wire rely entirely
  on the tailnet (WireGuard). Outside a tailnet the connection is cleartext.
- **Token-in-URL.** Convenient, but it leaks more readily than a header-only
  secret (history, logs, referrers). See §4.
- **The default share link is full operator.** Capability tokens (§6) exist and
  can scope a client to read-only or triage, but the QR/scan-and-go link still
  embeds the host token, which is operator-equivalent. A client that presents the
  host token is the *trusted host* on the bus and can call every capability;
  narrowing that requires deliberately minting and handing out a scoped token.
- **Driving an agent is code execution on the host.** The remote surface can
  spawn agents and terminals, send input, approve permission prompts, set the
  approval gate, read and write files (`fs.read` / `fs.write`), and run searches
  on the host — see the MCP facade tool list in `cmd/mcp/main.go` for the full
  capability surface, which mirrors what the bus exposes. **Anyone with the link
  effectively has a shell on your machine.** This is the point of the feature,
  but it means the link is as sensitive as SSH access.
- **Windows plugin sidecars run UNSANDBOXED.** Plugin sidecars are launched
  under OS filesystem confinement on Linux (`bwrap` / bubblewrap) and macOS
  (`sandbox-exec` / Seatbelt), which restrict a sidecar to writing only its own
  plugin directory. **Windows has no such mechanism** — `sandbox.Wrap` returns
  the command unchanged ("no filesystem sandbox mechanism on windows"), so on
  Windows a sidecar runs with the full privileges of the workspacer process.
  - The confinement mode is set by `WORKSPACER_PLUGIN_SANDBOX`: `off` (no
    confinement), `best-effort` (default — confine where a mechanism exists,
    else run plain), or `enforce` (fail closed — refuse to start a sidecar on a
    platform with no mechanism). On Windows, `best-effort` runs sidecars
    unconfined; only `enforce` will refuse to start them at all.
  - This is a defence-in-depth layer for *plugins*, separate from the remote
    token. It does not constrain what an authenticated bus client can do.

---

## 6. Capability tokens (scoped credentials)

Beyond the single host token, the hub can mint **scoped capability tokens** so a
remote client need not be full operator. This is enforced at the same seam as
everything else — `internal/bus/rpc.go` `router.call()` gates every `call` frame
through `mayCall` (verb allowlist) then `authorize` — so a scoped token cannot
reach a method outside its tier.

- **Three fixed tiers** (`internal/authtoken/authtoken.go`):
  - `view` — read-only (snapshots, listings, `push.key`).
  - `triage` — view + acknowledge/answer/approve and `push.subscribe` — enough to
    clear "needs you" prompts from a phone, but **not** to spawn or mutate.
  - `operator` — everything (`*`); equivalent to the host token.
  - `agents.spawn`, `terminals.create`, `git.push`, and `config.save` are
    deliberately **absent** from `view`/`triage` and reserved to operator;
    `cmd/brain/capspec_guard_test.go::TestSpawnStaysDeliberatelyUnscoped` locks
    that in so a future edit can't quietly leak spawn into a lower tier.
- **Minting / revoking.** `workspacer token create --scope view|triage|operator`,
  `workspacer token list`, `workspacer token revoke` (`cmd/workspacer/tokencmd.go`);
  tokens persist in `tokens.json` under the hub state dir (`0o600`) and are
  reloaded live on file change.
- **Known limits (see §5 and the exposure section).** Scoped user tokens are
  tiered **by verb only** — no per-path/per-argument confinement (that finer model
  exists only for *plugin* capabilities), and there is **no TTL/expiry** (a token
  lives until revoked). The default remote-share QR/link still embeds the host
  (operator) token, so scoping is something you opt into by minting and handing
  out a scoped token instead of the default link.

---

## If you ever want to expose this beyond a tailnet

The current feature is **not** built for direct exposure to the public internet
or to an untrusted network. Treat the following as the work that would be
required first — it is future work, not a supported configuration today:

- **TLS / WSS.** Terminate TLS (real certificate) and serve `https://` / `wss://`
  so transport encryption no longer depends on an underlying VPN.
- **Header-based, short-lived tokens.** Move off the `?token=` URL form to
  `Authorization` headers, and issue short-lived / rotatable credentials instead
  of one long-lived shared secret embedded in a link.
- **Per-client identity and least privilege.** _Partially delivered_ — scoped
  capability tokens (§6) already replace "any token → trusted host" for clients
  you deliberately mint a scoped credential for. What remains: the default share
  link still hands out the operator token (so scoping is opt-in, not the default
  onboarding path); scoped user tokens are tiered by verb only, with no
  per-argument/path confinement and no expiry.
- **Rate limiting & abuse protection.** Add connection/request rate limiting and
  lockout on the auth path; none exists today.
- **Route authorization review.** Re-examine every currently-unguarded route
  (assets, `/plugins`, `/plugins/ui/…`, `/health`) under a hostile-network
  assumption rather than the current "the boundary is `/bus`" assumption.

Until those exist, keep remote sharing on a trusted tailnet only.
