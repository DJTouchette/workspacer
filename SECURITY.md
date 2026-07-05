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

`webviewTag: true` with no `will-attach-webview` handler lets renderer content
create a `<webview>` with arbitrary `src` / `nodeintegration` / `preload`.

Recommended: add a `will-attach-webview` handler that forces
`nodeIntegration=false`, `contextIsolation=true`, and an allowed-origin list.
Decision needed: which webview sources are legitimate.

---

## Resolved in the audit pass

- **Workflow script RCE via `new Function()`**
  (`apps/desktop/src/main/services/workflowWatcher.ts`): the meta-block
  evaluator was changed from `new Function(...)` to a sandboxed
  `vm.runInNewContext` with a frozen, global-less context and a timeout, so a
  crafted workflow `.js` can no longer reach `process` / `require` /
  `child_process`. Behavior for legitimate pure-literal meta blocks is
  unchanged.
</content>
</invoke>
