# Native app threat model

Scope: `apps/native` (binary `wks-native`) and the parts of `services/hub-rs`
it embeds when started with `--local`. This describes what the code does today,
with the file each statement comes from. It is not the result of an adversarial
pass: see [What has not been reviewed](#what-has-not-been-reviewed).

For how to report a problem, see [`SECURITY.md`](../SECURITY.md). For the
earlier pass on the Electron app and the Go control plane, see
[`adversarial-hardening.md`](adversarial-hardening.md). For phone sharing, see
[`remote-sharing-security.md`](remote-sharing-security.md).

## What is trusted

- **The user's account on the machine.** Anything that can read or write the
  app's data folder as that user can read its credentials and change its
  state. Nothing below defends against that.
- **Agent CLIs the user installed** (Claude Code, Codex). They run with the
  user's access. Their *output* is not trusted.
- **Installed plugins.** Trusted local code, as in `SECURITY.md`.
- **GitHub's API and release downloads over TLS**, for update checks and
  downloads.

Not trusted: model output, anything arriving on the bus from a session or an
agent's MCP call, web pages the user visits while the app runs, and other
machines.

## Trust boundaries

### Listeners of the embedded backend

`--local` starts the hub, the MCP facade and the claudemon engine inside the
app's process (`apps/native/src/host/rust_local.rs`,
`services/hub-rs/src/backend.rs`). It opens loopback TCP listeners:

- the **bus** (`/bus` WebSocket, plus the web routes), on `127.0.0.1`. A first
  launch of the default install tries port 7895, then any free port; the port it
  got is saved in `config/hub-port` and reused. A stand-in port taken because
  7895 was busy is not saved (`keeps_port` in `rust_local.rs`).
- the **MCP facade**, on `127.0.0.1`, preferring 7897, saved in
  `config/mcp-port` under the same rule.
- the engine's API and hook listeners, on allocated loopback ports.

`configure_local_integrations` (`services/hub-rs/src/backend.rs`) defaults both
hub listeners to `127.0.0.1`. The native app has no flag that binds them to
another interface. Reaching the bus from another device goes through Tailscale
Serve, set up from Settings → Remote, which proxies to the loopback port.

### Bus authentication

`services/hub-rs/src/server.rs`:

- Every `/bus` upgrade must pass the Host and Origin checks in
  `services/hub-rs/src/server/policy.rs`. On a loopback socket the Host must be
  a loopback name, the socket's own address, or a name the owner explicitly
  trusted (a Tailscale Serve name, stored in `config/hub-trusted-hosts`). A
  browser Origin must be loopback, or match the Host and be a trusted name.
  This is the defence against a visited web page and DNS rebinding.
- The connection must present a credential, as `Authorization: Bearer <token>`
  or the `token` query parameter: the host token, or a scoped token from
  `config/tokens.json` (`view`, `triage`, `operator`). An empty credential is
  refused.
- The host token is `config/remote-token` in the data folder, created on first
  start with mode `0600` on Unix (`backend.rs`). On Windows no ACL is set
  beyond what the profile folder gives.

### The native client's own credential

When the app attaches to an existing hub instead of owning one
(`apps/native/src/main.rs`, `apps/native/src/bus.rs`):

- The hub URL must be `ws://` or `wss://` and may not carry credentials, a
  query or a fragment. The token travels only in the `Authorization` header.
- Token resolution order: `--token-file`, then `HUB_TOKEN`, then
  `<config dir>/workspacer/remote-token`. The last one is read **only when the
  hub URL's host is `127.0.0.1`, `localhost` or `[::1]`**, so the local
  identity is never sent to a remote endpoint by default.
- `ws://` to a non-loopback host is accepted and sends the token in clear
  text. The code does not refuse it.

### MCP port

`services/hub-rs/src/mcp.rs` (`authenticate`): the same Host and Origin checks,
then a bearer token from the `Authorization` header or the `t` query parameter,
resolved to an identity. Sessions the app launches are given the facade URL and
a bearer in their MCP configuration
(`services/hub-rs/src/services/session_facade.rs`). Request bodies are capped
at 64 MiB. A prompt-injected agent holds a valid MCP credential by design; what
it can do with it is bounded by the tool catalog and its scope, not by this
gate.

## Hub to UI requests

Agents and hub services can ask the window to do things. All of it goes through
`apps/native/src/ui_requests.rs` and `apps/native/src/ui/bus_commands.rs`.

- Only eight topics are parsed (`TOPICS`). Anything else is ignored.
- A request must be a JSON object of at most 64 KiB. Each field is typed and
  length-capped (`command` 64 KiB, `cwd`/`url` 16 KiB, notification `body`
  4 KiB, `action`/`paneType`/`type` 128 bytes, everything else 512). At most
  32 requests wait at once (`MAX_PENDING`, enforced in
  `apps/native/src/controller.rs`).
- Only the documented keys of a request are kept; extra keys are dropped
  before the request reaches the window.
- **Navigation** (`command.focus_agent`, `command.open_pane`,
  `command.run_action`, `command.open_spawn_dialog`) can switch the visible
  view, select a session that exists, or open the New Agent screen. It cannot
  start a session, approve anything or change settings. Unknown pane types and
  actions show a notice and do nothing.
- **Web pages** (`open_pane` with `paneType: browser`, the `open_browser`
  tool): only a URL beginning `http://` or `https://`, with a host and no
  whitespace, is offered (`web_page`). The window shows the URL and opens it in
  the system browser only when the person clicks "Open in browser". A request
  never opens a file, a script or another application's URL scheme.
- **Terminal commands** (`facade.openTerminal`): receiving one runs nothing.
  The window shows a one-line preview and a "Run in terminal" button. If
  accepted, the text is typed into the shell of the session the request names
  (else the selected one) with control
  characters other than newline and tab removed (`TerminalRequest::text`), so a
  request cannot carry an escape sequence behind the text that was shown. A
  multi-line command for another folder, or one for a non-POSIX path, is typed
  but not run.
- **Notifications** (`notify.post`, from an agent's `notify` tool or from hub
  services such as a proposed or failed job): the hub forwards only the
  documented string and boolean fields
  (`notification_event` in `services/hub-rs/src/services/mod.rs`). The window
  shows the text as a notice, and as an OS notification when the window is not
  active and the request is not `silent`/`inAppOnly`. A click can only select
  the named session. The `url`, `paneType` and `paneSection` fields are not
  acted on.

Residual risk: the offer is only as good as the person reading it. The preview
shows the first line and a count of the rest, and a notification's text is
whatever the agent wrote. An agent can phrase either to mislead.

## Links and files in chat

`apps/native/src/links.rs` classifies every link from Markdown, HTML cards,
image thumbnails and tool file targets in one place.

- Only `http`/`https` URLs reach the system browser. `file:` URLs are treated
  as file links (below). Every other scheme is refused with a message.
- A file link names a path on the machine that runs the session. It is read
  through the connected hub, not from the client's disk, with size limits (text
  previews 1 MiB kept, images 2 MiB, rendered Markdown 256 KiB).
- Commit and `#123` references open the in-app review view.

The file viewer can also write a file back through the hub (up to 4 MiB). That
is an operator capability of the connected hub, not something a link triggers.

## Terminal

The terminal pane is the selected session's own shell, on the machine that
runs the session. Output is untrusted bytes rendered by the app's terminal
emulator (`apps/native/src/terminal.rs`, pending output capped at 4 MiB per
session). The emulator's handling of escape sequences (title changes,
clipboard requests, hyperlinks) has not been reviewed adversarially.

## Updates

`apps/native/src/updates.rs`, `apps/native/src/update_helper.ps1`.

Check, all platforms:

- One HTTPS request to GitHub's releases API for this repository
  (`/releases/latest` for a stable build, the `nightly` tag for a build whose
  version contains `-nightly`). The response is capped at 4 MiB.
- On macOS and Linux the Download button opens this platform's package URL in
  the system browser, and only if that URL begins with this repository's
  `releases/download/` prefix. Otherwise it opens the release page URL from the
  API response. Nothing is installed by the app.

Download and install, Windows only:

- The asset name must match `Workspacer-Setup-<version>-x64.exe` (or the
  pre-0.170 preview name) and contain no path separator. The URL must begin
  with `https://github.com/DJTouchette/workspacer/releases/download/`.
- The published size must be between 1 byte and 256 MiB. The download stops if
  it exceeds that size and fails if it ends short.
- When the API response carries GitHub's per-asset `sha256:` digest, the
  download's SHA-256 must match it. When it does not, there is no checksum
  check. The `.sha256` file published beside the installer is for people; the
  app does not read it.
- The helper is a constant PowerShell script run by the system's
  `powershell.exe` (resolved from `%SystemRoot%`, never `PATH`), with paths
  and arguments passed in a JSON plan file, not interpolated into the script.
- The helper waits for the app to exit, runs the installer silently in the
  existing install folder, then checks that `build-stamp.json` reports the
  expected version. The app is relaunched whether or not the install worked.

Known limits:

- **The app does not verify an Authenticode signature.** Release installers are
  signed only on `v*` tags when the `AZURE_CODESIGN_PROFILE` repository
  variable is set (`.github/workflows/release.yml`); nightlies and unsigned tag
  builds are not. Integrity rests on TLS to GitHub plus the digest above.
- **No rollback.** A failed install is reported with a log path; the previous
  version is not restored.
- **No downgrade protection beyond the version comparison** in `check_result`:
  an update is offered only when the release's version is newer.
- The download goes to `%TEMP%\workspacer-native-update-<milliseconds>`, a
  predictable name created with `create_dir_all`. On a normal Windows profile
  `%TEMP%` is per-user.
- macOS builds are ad-hoc signed and not notarized. Users are told to clear the
  quarantine flag, which removes Gatekeeper's check for that copy.

## Electron import

`apps/native/src/host/electron_import.rs`.

- Runs at startup, before the backend opens the store: automatically for a
  default install with no data, with `--import-electron`, or when Settings →
  About left the marker file `import-electron.requested` in the data folder.
  The marker is deleted before the import starts, so a failing import is not
  retried on every launch.
- Sources are only read. SQLite files are copied with `VACUUM INTO` from a
  read-only connection. Symlinks in the source tree are skipped. Chromium
  caches, logs, lock and journal files, and the saved hub/MCP ports are not
  copied.
- Everything else in the Electron config directory is copied, **including its
  hub credentials** (`remote-token`, `tokens.json`) when present. After an
  import the native hub accepts the same pairing links and host token the
  Electron app did.
- Files are staged in a sibling directory and moved into place only when
  staging succeeded. An import over existing data first moves `config`, `hub`
  and `state.db*` into a `…before-electron-import-<time>` directory beside the
  store. If placing the import fails, those are moved back. The backup
  directory is never deleted by the app.
- Usage history is merged from a snapshot of Electron's `workspacer.db` into
  `config/headless-analytics.sqlite`. Table and column names come from the two
  schemas and are restricted to plain identifiers before they are put into SQL.

Anything able to write the marker file can make the next start replace the
store (after the backup). That requires write access to the data folder, which
is already full control.

## Single-instance guard

`apps/native/src/instance.rs`. One running app per data folder.

- The owner listens on `127.0.0.1` on an OS-assigned port and writes the port,
  a random token and its PID to `instance.json` in the data folder.
- A second launch reads that file, connects, and sends `<token> activate`. The
  only effect is that the first window is raised. A request line is read
  through a 256-byte limit with a 750 ms timeout; a wrong token gets no reply.
- A stale or malformed record is replaced. If the listener cannot be created
  the app starts without the guard.

Limits: the token comes from `std::collections::hash_map::RandomState`, which
is seeded by the OS but is not a documented cryptographic generator.
`instance.json` is written with default file permissions. A local process that
can read it can raise the window and nothing else. Connections are answered
one at a time, so a local process can delay a raise request.

## Data folder migration

`apps/native/src/data_dir.rs`. The folder was renamed from
`Workspacer Native Rust Preview` to `Workspacer Native`.

- On start, a preview folder is renamed to the new name when the new name does
  not exist, or exists as an empty directory (removed with `remove_dir`, which
  refuses anything non-empty).
- If the new folder already holds `state.db` or `config/config.yaml`, or the
  rename fails, nothing is moved. The two folders are never merged and neither
  is deleted.
- The Windows installer does not touch either data folder
  (`apps/native/packaging/windows/installer.nsi`).

## Launch environment

On Linux, a `--local` launch with no terminal on stdin runs the user's login
shell (`$SHELL -ilc`, default `/bin/sh`) to read its `PATH`, and uses that
`PATH` for the process (`launcher_path` in `apps/native/src/main.rs`). macOS
does the same when opened from Finder. This executes the user's shell profile
and trusts `$SHELL`. `WKS_NATIVE_KEEP_PATH` disables it on Linux.

## What has not been reviewed

There has been **no adversarial pass on the native app** comparable to
[`adversarial-hardening.md`](adversarial-hardening.md). That document's
findings were made against the Electron app and the Go/TypeScript control
plane, and its guards do not carry over by default. Specifically not reviewed:

- Rendering of untrusted transcript content in GPUI: Markdown, HTML cards,
  images (decoding of attacker-supplied image bytes), diffs, very large or
  malformed input.
- The terminal emulator's escape-sequence handling.
- The Rust hub's capability handlers as a whole (file read/write containment,
  git operations, plugin routes, the job runner) against a prompt-injected
  agent holding an MCP credential.
- Request authentication on the embedded engine's own API and hook listeners.
  This document did not re-read it.
- Scope enforcement for `view` / `triage` tokens across every bus method.
- The `/app` web client served by the native hub when a `web` folder or
  `WORKSPACER_WEBAPP_DIR` is present. The directory is trusted as given.
- The Windows installer and update helper under a hostile local user, and
  install-folder permissions.
- Tailscale sharing set up from Settings → Remote.
- Keep-warm and worktree artifact cleanup in the Rust hub
  (`services/hub-rs/src/services/keep_warm.rs`,
  `services/hub-rs/src/services/worktrees/artifact_cleanup.rs`), which are new
  ports. Cleanup deletes directories.

## Known gaps

Collected from the sections above:

1. No Authenticode verification of a downloaded installer; signing itself is
   conditional.
2. No checksum check when GitHub supplies no asset digest.
3. No update rollback.
4. macOS builds are not notarized.
5. `ws://` to a non-loopback hub sends the token unencrypted.
6. `instance.json` and, on Windows, `remote-token` rely on the profile
   folder's permissions.
7. A terminal offer shows only the first line of a multi-line command.
8. An Electron import carries the Electron hub's tokens into the native store.
