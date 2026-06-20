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
`services/hub/internal/bus/bus.go:118`

`/health` returns subscriber/method counts with no auth even when a token is
configured. Low sensitivity; consider reducing to `{"status":"ok"}` or gating
the detailed fields.

### 5. claudemon API uses permissive CORS on mutation endpoints — High
`services/claudemon/src/daemon/api.rs:94`

`CorsLayer::permissive()` lets any web origin call `POST /sessions/spawn`,
`POST /git/commit`, `POST /git/push`, and signal/input-injection endpoints.
Any page visited while the daemon runs can spawn processes or commit to git as
the daemon user.

Recommended: restrict `CorsLayer` to known origins (Electron scheme +
`http://localhost:*`). Decision needed: the allowed origin set.

### 6. claudemon git commit does not canonicalize `cwd` — Medium
`services/claudemon/src/daemon/git.rs` (`commit`)

`cwd` is passed to git as `current_dir` after only `rev-parse
--is-inside-work-tree`. A symlinked `cwd` can point outside the intended repo,
allowing commits to any git repo the daemon user can write.

Recommended: `canonicalize` `cwd` and verify it is under an expected prefix
(e.g. the user's home / configured project roots).

### 7. Electron path traversal in session load/delete — High
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
`apps/desktop/src/main/services/hubCapabilities.ts` (`fs.read`, `fs.write`)

These accept an arbitrary `path` from any bus client and read/write it. Under
remote sharing a client can read `/etc/passwd` or overwrite `~/.bashrc`, SSH
keys, etc. The existing binary/size checks are not a security boundary.

Recommended: restrict to an allowlist of base directories (home, configured
project roots); reject anything that escapes via `path.resolve`. Decision
needed: the allowed roots.

### 9. Electron `claude:signal` forwards an unvalidated signal string — Medium
`apps/desktop/src/main/ipc.ts` (`claude:signal`) →
`claudemonSessionClient.ts`

The renderer-supplied `signal` string is forwarded verbatim to the daemon's
REST API with no allowlist.

Recommended: validate against `['SIGTERM','SIGINT','SIGKILL','SIGSTOP','SIGCONT']`
before forwarding.

### 10. Electron enables `webviewTag` with no attach guard — Medium
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
