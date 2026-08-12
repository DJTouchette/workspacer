# Changelog

All notable changes to Workspacer are recorded here. Versions are the desktop
app version (`apps/desktop/package.json`); each `vX.Y.Z` tag builds installers
for macOS, Windows, and Linux plus a standalone headless-server bundle. The
rolling `nightly` prerelease tracks `master` between tagged releases.

The format loosely follows [Keep a Changelog](https://keepachangelog.com/).

## [Unreleased]

### Added
- **Widgets.** A project's widget board lives in the inspector rail's Project
  tab: small (1x1), medium (2x1) and large (2x2) tiles pinned to a *directory*
  rather than to a session, so every agent working in that repo sees the same
  board. Built-in git and usage tiles ship with it, and plugins can contribute
  their own — a widget is declared separately from a pane, because a pane may own
  a PTY or a 1.2MB editor bundle and none of that reads at 150px.
- **Release notes in the app.** Settings → Updates lists every release, newest
  first, with the running version marked. A one-time notice appears the first
  time an install runs a new version. Both read the same `CHANGELOG.md` the
  GitHub release page is cut from, so they cannot disagree.
- **See what's new before you restart.** The "Update ready" prompt has a third
  button that opens the release notes for the build it is offering. It then asks
  again rather than treating "I read them" as an answer. The notes open on the
  release page because a running build cannot contain them — its copy of the
  changelog was baked before that release existed.
- **Spawn an agent from the command palette.** It already could, under a name
  nobody would search for.

### Changed
- **A widget tile has no title bar.** The host used to draw the widget's name
  above it, which spent a fifth of a 148px square restating what the tile already
  said and pushed the content into a corner. Tiles now own their whole square and
  small ones centre; the name moved to the picker and to Edit mode, which is when
  you are identifying a tile rather than reading it.
- **The editor's file tree reads like the rest of the app.** It picked up the
  review pane's tree language: inset rounded rows, a tinted accent selection
  instead of a solid fill, and single-child folder chains collapsed into one row
  (`apps/desktop/src` rather than three). The sidebar is on the app's sans now —
  it is chrome, not code — while the file bar and status bar stay mono.
- **Shiplight's pipeline widget shows a stopwatch, not an age.** A running
  pipeline gets a live `m:ss` clock and a finished one shows how long it took —
  "4m" looks identical whether a run is alive or wedged.

### Fixed
- **The editor's file tree did not follow the file you opened.** Opening a file
  from outside the tree — right-click → Open in editor, a search hit, or a file
  you just created — loaded it into the editor while the sidebar stayed wherever
  it was, with nothing selected. The tree now expands to the file, highlights it
  and scrolls it into view, and keeps that highlight across a Refresh or an
  on-disk change. The plugin also read paths as POSIX-only, so on Windows every
  file was titled with its whole path instead of its name.
- **Settings that would not stick.** With remote sharing on, the hub handed the
  brain its own wildcard bind address as a *dial* URL, which the DNS-rebinding
  guard correctly refused — so the brain reconnected into a 403 forever with its
  output going to `/dev/null`, and every capability it is the sole provider for
  (config, library, saved layouts, sessions, profiles) answered "no provider" on
  the bus. The app runs on that bus by default, so every settings write failed
  silently. It reached people as a widget board that would not persist.
- **HTTPS via Tailscale returned 403 on every route.** The Remote Share dialog's
  one-tap toggle fronts a loopback hub with `tailscale serve`, which forwards a
  public `Host` to a loopback socket — indistinguishable from the rebinding shape
  the hub refuses. It was the only path to the secure context the phone client
  and Web Push need.
- **Writes that reported success without reaching disk.** The desktop config
  writer adopted and then served a value it had failed to write; the hub's layout
  document reported a version bump for a document that never landed; the
  quit-save handshake said "saved" on failure; the web Settings pane printed a
  green "Saved" for a pricing write that was a no-op stub. The TUI said "Renamed"
  into a directory it never created, so on a TUI-only machine every rename, pin
  and note had been failing silently since the feature shipped.
- **The live-agent view could go permanently empty under `workspacer serve`.** A
  non-2xx from the daemon was read as a clean end-of-stream and both callers
  discarded even that, so a permanent failure produced no log line anywhere.
- **A same-millisecond write by the other process was invisible to both config
  writers**, and was silently reverted through the lock with both sides reporting
  success.

### Security
- **Path containment on Windows.** Two escapes, neither reachable on Linux and
  both found the first time the guards were executed on Windows at all. A path
  *through a regular file* was allowed, because Windows reports that as
  not-exist where POSIX says ENOTDIR. And a trailing space made `..` a literal
  child name to the guard while Win32 read it as a parent traversal — enough to
  list the config directory, which holds the token that promotes a bus connection
  to trusted. All three copies of the containment rule carried both.
- **The git per-user-config gate was off on Windows**, so a dotfiles-symlinked
  `~/.gitconfig` was an ordinary writable file — and `filter.<drv>.clean` is
  command execution one write away.
- **Seven rounds of adversarial hardening** across the capability plane: plugin
  grants, boot-document restore, the event and HTTP planes, provider adapters,
  session lifecycle, and the twins that implement each rule more than once. The
  method and the findings are written up in `docs/adversarial-hardening.md`.

## [0.148.0] - 2026-07-29

### Added
- **Resizable sidebar.** Drag the panel's right edge to set any width from 220px
  to 560px (double-click to reset; arrow keys nudge once the handle has focus).
  The width persists in `config.ui.sidebarWidth` and is re-clamped to at most 45%
  of the window on load, so a width set on an ultrawide can't swallow a laptop
  screen. A drag writes config once on release, not once per frame.
- **The chat says how long it has been working.** A live elapsed timer on the
  in-progress turn, driven by the turn actually being open (an approval-parked
  turn is not over) rather than by the streaming flag.
- **Copy button on agent messages.** Hover an assistant message for a quiet
  action row that copies its raw markdown; the row reserves its height whether
  revealed or not, so it can't reflow a streaming transcript.
- **Sent messages admit they aren't acknowledged yet.** Until the daemon echoes
  the turn back, the bubble is dimmed and marked "Sending…" or "Queued".

### Changed
- **The sidebar's bottom belongs to the feed.** The full-width "Spawn agent"
  pill and the hub status row are gone. Spawn is now an icon in the header
  cluster beside the notification bell, and the mobile-client (Remote control)
  phone icon moved there with it — the same three actions, in the same order, in
  the collapsed rail. The History row pins to the real bottom of the panel.
- Your newest message rides the top of the viewport while the reply streams in
  below it, instead of both being crushed against the composer.
- Stop moved into the composer beside send rather than replacing it, so you can
  still queue a message mid-turn. Esc still cancels.
- The chat column is narrower (900px) and now a token, `--wks-chat-width`.

### Removed
- The hub connection dot. Reconnect was always automatic (`useHubReconnect`,
  and plugins re-subscribe) — the indicator was something to watch, not to act
  on. The "? Help" button in that row went with it; help is still F1, the
  command palette, and onboarding.

## [0.144.0] - 2026-07-22

### Added
- **Real plugin update detection.** Plugin manifests now carry a `version`
  field, and the Plugins Manager checks each installed plugin's install source
  for a newer published version instead of showing a permanent Reinstall button.
  A new guarded hub route `POST /plugins/updates` re-fetches every sourced
  plugin's manifest and compares versions; the pane shows a version chip, a
  **Check for updates** action, an accent **Update → vX** button only when the
  source is genuinely newer, and a muted **Reinstall** otherwise.
- The **Browse catalog** dialog now shows each plugin's published version.

### Changed
- Every plugin in the public catalog (and the bundled example plugins) is
  baselined at `1.0.0`; `build_index.py` now carries `version` into `index.json`.
- Documented the `version` field and the version-driven update flow in the
  build-a-plugin landing docs.

### Fixed
- The Plugins Manager no longer implies an update is available for every plugin
  installed from a source. A plugin with no declared `version` reports no update
  (it can still be reinstalled on demand), so the badge only appears for a real
  upgrade.

## [0.143.0] - 2026-07-21

### Added
- Review pane **history mode**: browse recent commits and their diffs.

### Changed
- Plugin panes split in place, scoped to the active agent; opening a global pane
  from an agent workspace highlights the right tab.
- Sidebar EARLIER and RECENT sections dock to the bottom of the feed.

### Fixed
- Plugin failures stay contained to their pane (focus recovery, timeouts, a
  Ctrl+P fallback), and a plugin update can no longer gut a live install on
  Windows file locks.
- The command palette can never trap the keyboard; webview key forwarding
  derives from the live keybinding config.
- Conversation no longer flickers when switching agents in stream mode.

## [0.142.0] - 2026-07-20

### Added
- **Keep-warm** learns Codex: per-provider heartbeats warm ChatGPT windows too.

### Fixed
- Agents reconnect after a reboot (reconcile + auto-resume at boot); terminated
  agents reach the sidebar RECENT list promptly.

## [0.141.0] - 2026-07-20

### Added
- **Keep-warm**: auto-start the 5h rate-limit window on a schedule; keep-warm
  pings become logged, listable claudemon heartbeats.
- PostHog analytics on the landing pages.

### Changed
- Styling consistency sweep: icons over symbols, single-source design tokens.
- Stop interrupting agents on usage warnings; keep the accurate gauges.

## [0.140.0] - 2026-07-19

### Added
- Approval cards for stream-transport Claude sessions, resolved structurally
  from the inbox.

### Changed
- The needs-you dock is minimizable and denser.

## [0.139.0] - 2026-07-19

### Changed
- Expanded sidebar reworked into live activity-feed cards (action log, provider
  hues, inline Approve/Reply); RECENT lists resumable sessions with auto titles.
- Single implicit session — named sessions removed; boot restores the most
  recent. Stopped daemon rows revive on resume; attach viewers stay alive.

## [0.138.0] - 2026-07-18

### Added
- Plugin authoring loop: `workspacer plugin dev` hot-reload, sidecar log
  streaming, and a `window.workspacer` SDK auto-injected into plugin webviews.
- `make-workspacer-plugin` skill + a build-a-plugin landing page.

### Changed
- New configs default to the everforest theme; a single `DEFAULT_THEME` constant
  replaces scattered `dark` fallbacks.

[0.144.0]: https://github.com/DJTouchette/workspacer/releases/tag/v0.144.0
[0.143.0]: https://github.com/DJTouchette/workspacer/releases/tag/v0.143.0
[0.142.0]: https://github.com/DJTouchette/workspacer/releases/tag/v0.142.0
[0.141.0]: https://github.com/DJTouchette/workspacer/releases/tag/v0.141.0
[0.140.0]: https://github.com/DJTouchette/workspacer/releases/tag/v0.140.0
[0.139.0]: https://github.com/DJTouchette/workspacer/releases/tag/v0.139.0
[0.138.0]: https://github.com/DJTouchette/workspacer/releases/tag/v0.138.0
