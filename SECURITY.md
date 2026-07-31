# Security Notes

This document tracks known security-sensitive findings in workspacer that
require a **policy / design decision** before they can be safely changed.
They were surfaced during a code audit and are intentionally **left as-is**
because fixing them changes externally-observable behavior (remote sharing,
plugin model, CORS, webview embedding) and should be decided deliberately
rather than patched blindly.

Threat model context: workspacer runs a local control plane (the Go `hub` bus
and the Rust `claudemon` daemon) on loopback. Several of these issues only
become exploitable when the user opts into remote sharing
(`WORKSPACER_REMOTE_SHARE=1` / a shared `HUB_TOKEN`, e.g. over Tailscale) or
visits a malicious web page while the daemons are running (DNS-rebinding /
cross-origin requests against loopback services).

Severity is the reviewer's estimate assuming the remote-sharing path is in use.

---

## Open items (decision required)

### 1. Hub bus WebSocket disables origin checking — High
**FIXED 2026-07-05** (`5d223ca`): explicit origin policy — no-Origin native
clients, same-origin-as-Host (incl. Tailscale), loopback any port; cross-site
browser origins get 403 before any auth work.
`services/hub/internal/bus/bus.go:132`

`websocket.Accept` is called with `InsecureSkipVerify: true`, skipping the
same-origin check. With no token (the localhost default), **any web page the
user visits** can open `ws://127.0.0.1:7895/bus` from the victim's browser and
drive the bus (publish events, call capabilities, register providers).

Recommended: replace `InsecureSkipVerify` with an explicit `OriginPatterns`
allowlist (Electron app origin + expected hosts); keep the token as
defense-in-depth. Decision needed: which origins are legitimate for remote/web
clients.

### 2. Hub `register` allows capability hijack — High
**FIXED 2026-07-05** (`5d223ca`): first-registration-wins — a method owned by
a different live connection cannot be re-registered (trusted conns included);
ownership frees on disconnect so reconnect flows are unaffected.
`services/hub/internal/bus/rpc.go:107`

`register` unconditionally sets `rt.providers[method] = conn.id`, overwriting
any existing provider. The `SetAuthorize` seam gates `call` but **not**
`register`, so any client that passes the bus token can register
`claude.approve`, `agents.spawn`, etc. and intercept every subsequent caller's
params (session ids, prompts, approvals).

Recommended: refuse re-registration of a method already owned by a different
live conn, and/or consult the authorize callback on `register` too. Decision
needed: the intended capability-ownership / delegation model.

### 3. Plugin install runs arbitrary commands + unbounded extraction — High
**PARTIALLY FIXED 2026-07-05** (`aa87c8d`): extraction bounded (512 MiB total /
128 MiB per file / 10k entries, enforced streaming). The build command still
runs unconfined — the interactive-consent / sandbox decision remains open.
`services/hub/internal/plugin/install.go` (`runInstall`, tar extraction);
route `cmd/hub/main.go` `/plugins/install`

`runInstall` executes `argv[0]` from the *downloaded* `plugin.json` with the
plugin dir as cwd — full arbitrary command execution. The route is
token-guarded, but a shared `HUB_TOKEN` turns any token holder into RCE on the
host. Separately, tar extraction uses `io.Copy` with no size ceiling, so a
decompression bomb can fill the disk.

Recommended (decision needed): treat install as an explicitly-consented trusted
operation (interactive confirmation rather than any authenticated POST), and
cap extraction with `io.CopyN` + a total-bytes ceiling. The zip-slip path guard
(`filepath.Rel`) is already correct.

### 4. Hub `/health` leaks internal counts unauthenticated — Low
**FIXED 2026-07-05**: `/health` now gates the detail on auth — when a token is
configured and the caller isn't authorized, it returns `{"status":"ok"}` only;
with no token (the loopback default) or an authorized caller it still returns the
subscriber/method counts (handy for local ops/tests). So the counts never reach an
unauthenticated remote probe once the bus is token-guarded.
`services/hub/internal/bus/bus.go:118`

`/health` returns subscriber/method counts with no auth even when a token is
configured. Low sensitivity; consider reducing to `{"status":"ok"}` or gating
the detailed fields.

### 5. claudemon API uses permissive CORS on mutation endpoints — High
**FIXED 2026-07-05**: CORS restricted to loopback origins only (no legitimate
browser context calls claudemon directly), plus a Host-header guard against
DNS rebinding; session ids are validated at the API boundary and transcript
reads are lexically confined to the projects root.
`services/claudemon/src/daemon/api.rs:94`

`CorsLayer::permissive()` lets any web origin call `POST /sessions/spawn`,
`POST /git/commit`, `POST /git/push`, and signal/input-injection endpoints.
Any page visited while the daemon runs can spawn processes or commit to git as
the daemon user.

Recommended: restrict `CorsLayer` to known origins (Electron scheme +
`http://localhost:*`). Decision needed: the allowed origin set.

### 6. claudemon git commit does not canonicalize `cwd` — Medium
**FIXED 2026-07-05** (deviation from the recommendation, noted): the git surface
this finding describes no longer lives in claudemon — commit `3732018` moved it to
the host (`apps/desktop/src/main/services/gitService.ts`) and the daemon no longer
touches git. Its remaining remote-reachable entry point is the `git.*` hub
capabilities, so the containment was applied there
(`apps/desktop/src/main/services/hubCapabilities.ts`): every `git.*` cap now
canonicalize-then-contains its caller-supplied `cwd` to the same workspace roots as
fs.* (#8) — the live agent cwds the review pane legitimately operates on, plus the
config dir. Canonicalization resolves symlinks before the check, so a symlinked
`cwd` can't escape the roots (the finding's original concern). Scope deviation from
the doc's "under the user's home": home-only would be both too broad for a remote
caller and would need to gate the trusted local desktop IPC path (the user
reviewing their own repos, which may live outside home) — the workspace-roots set
is tighter for the bus and leaves legitimate local review untouched.
`services/claudemon/src/daemon/git.rs` (`commit`) — moved to
`apps/desktop/src/main/services/gitService.ts`; guarded at the `git.*` hub caps.

`cwd` is passed to git as `current_dir` after only `rev-parse
--is-inside-work-tree`. A symlinked `cwd` can point outside the intended repo,
allowing commits to any git repo the daemon user can write.

Recommended: `canonicalize` `cwd` and verify it is under an expected prefix
(e.g. the user's home / configured project roots).

### 7. Electron path traversal in session load/delete — High
**FIXED 2026-07-05**: `loadSession` / `deleteSession` now resolve the
caller-supplied `filename` against the sessions dir and require the result to sit
at or under it (`resolveWithinSessionsDir` — `path.resolve` collapses `..`, then a
prefix check), rejecting a traversal (`../../.ssh/id_rsa`) or an absolute path
before any `fs` call. The check runs *outside* the existing try/catch so an escape
attempt rejects loudly instead of being swallowed into a null "not found". This
covers the `sessions.load` / `sessions.delete` hub capabilities too, since they
delegate to these methods.
`apps/desktop/src/main/services/sessionService.ts` (`loadSession`,
`deleteSession`); also the `sessions.load` hub capability in
`apps/desktop/src/main/services/hubCapabilities.ts`

`path.join(getSessionsDir(), filename)` uses a renderer/bus-supplied `filename`
with no containment check; `filename = "../../.ssh/id_rsa"` reads/deletes
outside the sessions dir. Reachable from the hub bus (and thus a remote client)
via the `sessions.load` capability.

Recommended: after `path.join`, assert
`path.resolve(p).startsWith(getSessionsDir() + path.sep)` and reject otherwise.
(Low-risk to apply; left here only because it is part of the same path/allowlist
decision as #8.)

### 8. Electron `fs.read` / `fs.write` hub capabilities have no path allowlist — High
**FIXED 2026-07-05** (`5d223ca`): fs.read/write/listEntries/watch/search are
canonicalize-then-contain confined to live agent cwds + the config dir
(fs.listDir to the home tree for the folder picker).
`apps/desktop/src/main/services/hubCapabilities.ts` (`fs.read`, `fs.write`)

**REGRESSED, then fixed again 2026-07-29.** The 2026-07-05 fix was confined to
the desktop provider, which registers these methods through `cat(...)` — a no-op
once the capability catalog was delegated to the Go brain, and that delegation is
the DEFAULT (`brainDelegation.ts`, off only with `WORKSPACER_NO_BRAIN=1`). So the
provider actually answering `fs.read`/`fs.write`/`fs.listEntries`/`fs.listDir`
was `services/hub/cmd/brain`, whose handlers did `expandTilde` and then read or
wrote the path with no containment at all. Any bus client — a remote-share client
on the tailnet, a plugin, or an agent through the MCP facade — could read
`~/.ssh/id_rsa` and write `~/.ssh/authorized_keys`, exactly the issue this entry
records as fixed. The app-side test could not catch it: it mocks delegation OFF,
so it only ever exercised the kill-switch path.

The containment is now ported into the brain (`cmd/brain/fsguard.go`, same
canonicalize-then-contain rule and the same roots) and covered by
`cmd/brain/fsguard_test.go`, which asserts the deny for absolute, traversal,
symlink-out-of-cwd and `~`-relative escapes, and that a denied write leaves the
target byte-identical. `cmd/brain/delegation_guard_test.go` now cross-checks that
the two providers partition the surface, so a capability cannot again be guarded
behind a door the default configuration never opens.

Lesson for any future path-bearing capability: the guard has to live with the
provider that answers, and "which provider answers" depends on delegation.

**ROOTS NARROWED 2026-07-30** (unreleased). With the rule finally enforced in
both providers, the audit found the chosen *root* was the remaining problem. The
roots were "live agent cwds + the config dir", and the config dir is where the
credentials live: `remote-token`, `tokens.json`, `remote-server.json`,
`vapid.json`, every plugin's `.bus-token`, every plugin's `.settings.json` (which
holds plugin secrets in plaintext), `workspacer.db` and the Electron cookie jar.
Any granted fs scope that resolved in there could read the token that promotes a
plugin connection to a *trusted* bus connection — a containment that hands out
the credential for bypassing containment.

The roots are now:

- **workspaceRoots** — used by `fs.read`/`fs.write`/`fs.listEntries`/`fs.watch`/
  `fs.search`, the `git.*` caps (#6) and `library.*`: the live agent cwds, plus
  exactly three config-dir subtrees, `<configDir>/library`,
  `<configDir>/layouts`, `<configDir>/sessions`. The config dir itself is no
  longer a root.
- **browseRoots** — `fs.listDir`, the folder picker: workspaceRoots + `$HOME`,
  unchanged.
- Independently of the roots, and after canonicalization, a path is denied for
  read **and** write if its basename is `.bus-token` or `.settings.json`
  anywhere at all, or if it equals `<configDir>/remote-token`,
  `<configDir>/tokens.json`, `<configDir>/remote-server.json` or
  `<configDir>/vapid.json`. The deny-list holds independently of the root
  narrowing; either one alone would have stopped the credential reads.

Both refusals raise the same message and neither echoes the path, so a caller
cannot use the wording to probe for what exists. The desktop provider is stricter
on one axis — it refuses the whole config dir outside those three subtrees, which
additionally covers `workspacer.db` and the cookie jar. Stricter, never looser:
a deliberate divergence, not drift.

Consequence: a plugin installed at `<configDir>/plugins/<id>` can no longer
`fs.read`/`fs.write` its own install dir *through the bus*. Every manifest in the
public catalog scopes `fs.*` to `${agentCwd}`, and sidecars read their own
directory with local Node `fs`, so nothing shipped regresses.
`services/hub/cmd/brain/fsguard.go` (`configStoreRoots`, `workspaceRoots`);
`apps/desktop/src/main/services/hubCapabilities.ts`

These accept an arbitrary `path` from any bus client and read/write it. Under
remote sharing a client can read `/etc/passwd` or overwrite `~/.bashrc`, SSH
keys, etc. The existing binary/size checks are not a security boundary.

Recommended: restrict to an allowlist of base directories (home, configured
project roots); reject anything that escapes via `path.resolve`. Decision
needed: the allowed roots.

### 9. Electron `claude:signal` forwards an unvalidated signal string — Medium
**FIXED 2026-07-05**: the signal name is now validated against an allowlist
(`['SIGTERM','SIGINT','SIGKILL','SIGSTOP','SIGCONT']`) in
`claudemonSessionClient.signal()` — the single chokepoint both the `claude:signal`
IPC handler and the `claude.signal` hub capability funnel through, so the renderer
and the remote/MCP bus paths are gated together. An unrecognized signal rejects
before any daemon call. Note claudemon is stricter still: its `Signal` serde enum
only accepts SIGINT/SIGTERM/SIGKILL, so SIGSTOP/SIGCONT already fail closed there
(deserialize error → 4xx); the allowlist keeps the doc's recommended superset so it
stays correct if the daemon later grows job-control signals.
`apps/desktop/src/main/ipc.ts` (`claude:signal`) →
`claudemonSessionClient.ts`

The renderer-supplied `signal` string is forwarded verbatim to the daemon's
REST API with no allowlist.

Recommended: validate against `['SIGTERM','SIGINT','SIGKILL','SIGSTOP','SIGCONT']`
before forwarding.

### 10. Electron enables `webviewTag` with no attach guard — Medium
**FIXED 2026-07-05**: the main window now installs a `will-attach-webview` handler
(`apps/desktop/src/main/index.ts`, logic in `main/lib/webviewGuard.ts`) that
force-applies safe prefs on every attach — strips any `preload`, sets
`nodeIntegration=false` (top frame and sub-frames), `contextIsolation=true` —
regardless of what the `<webview>` tag requested, so an injected privileged webview
can't reach the main process. It also confines the `src` to http/https/about
(BrowserPane does arbitrary http(s) browsing; plugin panes load the hub UI origin
or a 127.0.0.1 sidecar — no legitimate webview uses `file://` or other local
schemes), and a `did-attach-webview` handler re-runs that scheme check on every
`will-navigate` / `will-redirect`, so a `file://` URL typed into the browser bar is
blocked too, not just the initial src. Investigated: no webview sets a preload or
uses `file://`, so forcing these prefs is behavior-preserving for the real panes.
`apps/desktop/src/main/index.ts:192`

**CORRECTION 2026-07-30** (unreleased). The last clause of that annotation — "a
`file://` URL typed into the browser bar is blocked too" — was false as shipped,
and had been since 2026-07-05. BrowserPane navigates by calling
`webview.loadURL()`, and `will-navigate` does not fire for a navigation the
embedder initiates that way; it fires only for navigations the guest page starts.
So the only scheme check that actually ran on a browser pane was the attach-time
`src` check, and typing `file:///home/<user>/.ssh/id_rsa` into the pane's address
bar loaded it.

It is now enforced on `did-start-navigation`, which does fire for `loadURL`. That
event is not cancelable, so the handler calls `stop()` and bounces the guest to
`about:blank` rather than preventing the event; `will-navigate` stays hooked as
the cancelable path for guest-initiated navigations. The documented behaviour is
finally true. Consequence: anyone who was using a browser pane to read local
files loses that.
`apps/desktop/src/main/lib/webviewGuard.ts` (`installWebviewNavigationGuard`)

`webviewTag: true` with no `will-attach-webview` handler lets renderer content
create a `<webview>` with arbitrary `src` / `nodeintegration` / `preload`.

Recommended: add a `will-attach-webview` handler that forces
`nodeIntegration=false`, `contextIsolation=true`, and an allowed-origin list.
Decision needed: which webview sources are legitimate.

### 11. claudemon ingress: `/wrapper/:id`, the hook port, and `bin` on the models route — High
**FIXED 2026-07-30** (unreleased).
`services/claudemon/src/daemon/wrapper_ws.rs` (origin policy, Register/path
agreement, single-wrapper rule); `services/claudemon/src/daemon/hook.rs` (Host
guard); `services/claudemon/src/daemon/spawn.rs:473` (models route)

Three unauthenticated ingress paths on the daemon's two loopback ports, all
reachable from a page the user merely visits:

- `GET /wrapper/:id` upgraded a WebSocket with no origin policy, so any page
  could dial `ws://127.0.0.1:7891/wrapper/<id>` and drive a session's PTY. It now
  applies the same policy as the hub bus (#1): no-Origin native clients and
  same-origin/loopback pass, cross-site browser origins get 403 before the
  upgrade. Two further rules, both about a wrapper claiming a session it has no
  business with: a `Register` frame whose `session_id` disagrees with the path
  that was dialed closes the socket, and a second wrapper on a session that
  already has a live one is refused instead of silently taking over.
- The hook ingress port (7890) accepted any `Host` header, so a DNS-rebinding
  page could POST forged hook events — the events that drive session state,
  approvals and usage accounting. It now 403s a `Host` that is neither loopback
  nor the configured bind host, exactly as the API port already did (#5).
- `GET /providers/:provider/models` executed a `bin` taken from the query string
  in order to enumerate models. A GET needs no preflight, so that was drive-by
  command execution as the daemon user. The parameter is still accepted for
  compatibility but is never executed: the daemon resolves the launcher itself
  from `PATH`, with a `WKS_<PROVIDER>_BIN` environment override. Note the Host
  guard from #5 already covered this route and would not have helped — a page
  dialing 127.0.0.1 sends a loopback `Host` that the guard correctly permits.
  Ignoring `bin` is the fix.

Breakage, accepted deliberately: a third-party wrapper client breaks if it runs
in a browser from a non-loopback origin, registers a session id other than the
one it dialed, or attaches as a second wrapper to a live session. The bundled PTY
wrapper does none of those. Separately, `agents.binaries.<provider>` from
`config.yaml` no longer reaches the model picker — claudemon has no YAML reader —
so a custom launcher path is honoured only when `WKS_<PROVIDER>_BIN` is exported
into the daemon's environment, and nothing exports it yet. A user who pointed
that setting at a binary outside `PATH` will see the picker use the PATH-resolved
one. Known gap, not yet wired.

### 12. `updates.*` is host-trusted — Medium
**DECIDED 2026-07-30** (unreleased).
`services/hub/cmd/brain/config.go:168` (`hostTrustedSections`);
`apps/desktop/src/main/services/updateService.ts` (`sanitizeUpdateChannel`)

`updates.channel` is string-concatenated into the electron-updater feed URL — it
becomes `<channel>.yml` on the provider's base URL — and it was writable by
anything that could call `config.save`: a remote-share client, a plugin, or an
agent through the MCP facade. Retargeting the feed points the auto-updater at an
attacker-chosen repository, which is code execution on the host at the next
update check.

A `config.save` arriving over the bus now drops the whole `updates` block and
logs that it did; only the desktop's in-process `configService` write can change
it. Independently, a channel that is not a bare channel name is clamped to
`latest` with a warning before it reaches `setFeedURL`, so a malformed value
already on disk cannot retarget the feed either.

Consequence, and it is real: changing the update channel from a web or remote
client's Settings no longer persists — it silently has no effect. Update
configuration is a desktop-only control now. That is the intended trade, and
`updates` is the entire host-trusted list today; anything added to it takes the
same trade.

### 13. Layout documents carried the bus token in plugin pane URLs — High
**FIXED 2026-07-30** (unreleased).
`services/hub/internal/layout/layout.go:45` (redacted on write and on load);
`apps/desktop/src/renderer/src/hooks/useLayoutSync.ts:83`;
`apps/desktop/src/renderer/src/components/panes/PluginPane.tsx`

A plugin pane's `url` carried `busToken=<host token>` as a query parameter, and
the layout is a *shared* document: it is published to the hub and mirrored to
every web/remote client. So the pane URL handed the host bus token — which opens
a trusted connection able to call any capability — to any client that could read
the layout.

`pane.url` is now published with `busToken=` blank. The hub blanks it both when
writing the document and when loading one that is already poisoned on disk, and
the renderer strips it before publishing. A plugin pane mints its own token
locally when it mounts; previously only agent-scoped panes did.

Consequence: a plugin pane mirrored to a client that cannot mint a token shows
its own bus-disconnected state instead of silently inheriting the host's
credential. That visible failure *is* the leak being closed. Two residues, both
known and neither handing the token out: a layout document already on disk keeps
its persisted token bytes until the next write rewrites the file (`load` redacts
what is served), and local session snapshots written by the desktop may still
contain pane URLs with the static token — only the hub layout document is
redacted so far.

### 14. Plugin fs path scopes could climb out of their binding — Medium
**FIXED 2026-07-30** (unreleased).
`services/hub/internal/plugin/manifest.go:268` (`validateScope`);
`services/hub/internal/plugin/manager.go:87` (`expandScope`)

A plugin declares its own fs path scopes in its manifest (`"paths":
["${agentCwd}"]`). A scope of `${pluginDir}/../..` names the config directory, so
a plugin could widen its own grant by writing a string — into the directory whose
credential files #8 now denies.

`Manifest.Validate` refuses any scope containing a `..` segment, so such a
`plugin.json` fails at install time and at boot-time load (both go through
`Load`) with an error naming the offending scope. `expandScope` re-checks
independently, and additionally requires that a `${token}`-derived root still
resolve inside the directory the token names.

Two limits, stated rather than implied. The containment is lexical (`Clean` plus
a prefix check), not symlink-resolving: a plugin dir containing a symlink out
would still canonicalize outward, but resolving and enforcing that belongs to the
bus (`internal/bus/policy.go`), and calling `EvalSymlinks` here would reject
scopes naming directories that do not exist yet at load time. And an absolute
scope such as `/` is still accepted — that is the standing trusted-install stance
(#3), not this finding.

Only `..` is refused; absolute paths and `${token}/sub` subpaths behave exactly
as before, so no manifest in the public catalog is affected.

### 15. Plugin manifest `provides` is not validated — High
**DEFERRED 2026-07-30 — still open, decision required.**
`services/hub/internal/plugin/manifest.go:65,184,238`;
`services/hub/internal/bus/bus.go:577`; `services/hub/internal/bus/rpc.go:128`

A plugin's own manifest is the sole authority on which capability methods it may
answer. `Manifest.Validate` checks the id, apiVersion, pane types and settings,
but never looks at `Provides`; the value is copied onto the connection's grant
verbatim and matched with a glob, so `provides: ["*"]` matches every method and a
core name such as `claude.approve` or `agents.spawn` matches itself. A plugin
holding the provider slot for a method receives every subsequent caller's params
— session ids, prompts, file contents, approval decisions — and returns whatever
result it likes.

Two properties narrow this; neither closes it. First-registration-wins (#2) means
the plugin has to claim a method no live connection owns, which is a race it can
win at hub boot or during a provider restart. And in-process handlers take
precedence over WebSocket providers, so capabilities the hub answers itself are
safe — but the brain and the desktop are both WebSocket providers, and that is
where `fs.*`, `agents.spawn` and the `claude.*` surface live.

Not fixed in this pass because rejecting `*` or a foreign namespace rejects
manifests that validate today: it needs a catalog audit and a manifest schema
version to migrate them behind. Decision needed: whether `provides` is restricted
to the plugin's own id namespace (`<pluginID>.*`), with any core method requiring
an explicit host-side grant rather than a self-declaration, and what the
migration path is for manifests that already declare otherwise. Until then,
installing a plugin means trusting it with the bus — consistent with #3, but not
what the manifest's shape implies to someone reading it.

### 16. MCP facade is an unauthenticated local capability gateway — High
**KNOWN GAP, unchanged 2026-07-30 — decision required.**
`apps/desktop/src/main/services/mcpFacadeDaemon.ts` (`getMcpFacadeToken`);
`services/hub/cmd/mcp/main.go:44,95` (`-mcp-token`, `requireBearer`)

The facade at `127.0.0.1:7897/mcp` fronts the whole bus: spawn agents, read and
write host files, rewrite config. `cmd/mcp` already implements a bearer check on
`/mcp` and `/sse`, but the desktop does not set `WKS_MCP_TOKEN`, so the check is
off and any local process — or any page that can be talked into a request to
loopback — reaches it with no credential. This is the reachability path that made
#12 a code-execution issue rather than a nuisance.

Arming it is a coordinated change, not a flag flip, which is why this pass did
not take it: neither client can send the header today. `mcpConfig.ts` writes the
supervisor's `{ type:'http', url }` entry with no `headers` (and only when the
file is absent, so existing installs keep the old entry), and `managedSpawn`
hands claudemon the facade as a bare URL string with nowhere to attach one.
Setting the variable alone would trade a local-reachability risk for a certain
loss of the supervisor and every MCP-facade worker.

Decision needed: land header support in both clients together with the flip, and
decide what happens to an already-written supervisor MCP entry on upgrade.

---

## Resolved in the audit pass

- **Workflow script RCE via `new Function()`**
  (`apps/desktop/src/main/services/workflowWatcher.ts`): the meta-block
  evaluator was changed from `new Function(...)` to a sandboxed
  `vm.runInNewContext` with a frozen, global-less context and a timeout, so a
  crafted workflow `.js` can no longer reach `process` / `require` /
  `child_process`. Behavior for legitimate pure-literal meta blocks is
  unchanged.
