# Changelog

All notable changes to Workspacer are recorded here. Versions are the desktop
app version (`apps/desktop/package.json`); each `vX.Y.Z` tag builds installers
for macOS, Windows, and Linux plus a standalone headless-server bundle. The
rolling `nightly` prerelease tracks `master` between tagged releases.

The format loosely follows [Keep a Changelog](https://keepachangelog.com/).

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
