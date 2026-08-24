# Changelog

All notable changes to Workspacer are recorded here. Versions are the desktop
app version (`apps/desktop/package.json`); each `vX.Y.Z` tag builds installers
for macOS, Windows, and Linux plus a standalone headless-server bundle. The
rolling `nightly` prerelease tracks `master` between tagged releases.

The format loosely follows [Keep a Changelog](https://keepachangelog.com/).

## [0.150.0] - 2026-08-23

### Added
- **The Fleet Manager.** A dedicated orchestrator agent — a real session at
  operator tier, not a script — that dispatches worker agents, watches them
  finish or block, and keeps a per-project brief. It runs on Claude or Codex
  (`agents.managerProvider`) and dispatches to all four supported harnesses.
  The manager never polls: it is woken on a worker finishing, blocking,
  hitting a threshold it armed, or reporting its own progress
  (`agents.reportProgress`), all through one wake channel.
- **The brief.** A living per-project markdown file the manager writes as
  work lands, with a shape that encodes decay — a `## Now` list that empties
  as work finishes, a durable `## Direction`, and a `## Recently` log that
  records not just what happened but *why*, including retracted decisions
  and do-not-re-dispatch markers. `/checkpoint` prunes and archives it rather
  than letting it grow forever; the new Board pane renders it as a kanban
  with drag-to-archive. `brief_append` is safe for a worker and the manager
  to write to at the same moment.
- **Fleet succession, while the app is running.** If a manager crashes or is
  closed, its workers are discoverable as orphans (`list_orphans`) and can be
  adopted by a successor (`adopt_workers`, `reparentChildren`); crashed
  managers leave a tombstone their successor can read.
  `agents.list` now reports `parentSessionId`, `label`, and `isSupervisor` so
  the fleet's shape is visible. This bookkeeping lives in memory — it does
  not survive an app restart.
- **Structured results.** A dispatch can ask a worker for a JSON-Schema-shaped
  answer; the worker returns it as a fenced `wks-result` block, which is
  validated and rendered as a real card — on desktop and on the `/m` phone
  client — instead of a raw JSON dump in the transcript.
- **Mobile fleet-wake detection.** `/m` now distinguishes a worker that's
  merely taking a while from one that's actually stuck: a working agent with
  no progress for several minutes raises a "Not moving" item, and one whose
  status line has gone silent raises "No signal" instead.
- **Tiered dispatch tokens.** Workers can be minted a capability token scoped
  to view, triage, or operator tools, derived from one allowlist shared by
  the MCP facade and the hub — so a read-only scout genuinely cannot mutate
  anything.
- **Worktree setup hooks + node_modules auto-link.** Project-defined hooks run
  deterministically when a worktree is created for a dispatched agent, and
  `node_modules` is auto-linked into it at whatever depth the project needs,
  so a fresh worktree doesn't need a full reinstall to run.

### Fixed
- **Security: a federated call could forge caller identity.** `router.call`
  and the cross-hub `federatedCall` path each maintained their own
  hand-written list of which fields to strip or stamp for caller identity
  before dispatch (profile/yolo-grant on `agents.spawn`, `callerSessionId` on
  `agents.reportProgress`). The two lists could drift, and one gap did: a
  view-tier token could forge a `callerSessionId` across a federated hop.
  Both paths now consult a single sanitizer table so they cannot diverge
  again.
- **Hardened the pending-message-slot invariant** across the Rust session
  state machine, the hook feed, and the store, closing several races where a
  send could be dropped or a parked approval clobbered.
- Numerous smaller fixes to worker-finished wake reliability, model/profile
  respawn duplicate cards, and sidebar/grandchild session display.

### Changed
- `CLAUDE.md` and the `.rivet/context` documentation are now tracked in the
  repository.
- The mobile end-to-end suite (`/m`, Playwright) was repaired — it had been
  silently unrun for two days — and is now wired into CI on every PR.

## [0.149.0] - 2026-08-14

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
- **Notifications say what the agent actually said.** They used to name the
  event and nothing else — "Approve a tool use" told you an agent was blocked
  but not on what. They now carry the tool and its argument (`Bash rm -rf
  build/`), the text of the question being asked, or the tail of the reply that
  just finished. It is per device and switchable, because that text lands on a
  lock screen: **Show message contents**, on by default. There is also an opt-in
  **Ping while still running**, at 10 and 30 minutes into a single run, for when
  you have walked away from something long.
- **"Send me a test notification", and settings you can actually reach.** The
  phone can now provoke a push on demand and reports how many devices it went
  to, so "I got nothing" stops being a scavenger hunt across trigger, delivery,
  subscription and permission — it deliberately ignores the per-kind switches,
  since a test something could silence cannot tell muted from broken. The More
  sheet also opens without an agent selected (it used to be a dead button on the
  screen the app lands on), and the notification banner now explains itself when
  push is unavailable — "Add to Home Screen" or "needs an HTTPS address" —
  instead of rendering nothing at all, which is what made this invisible.
- **More kinds of phone notification, and per-device control of them.** The
  phone used to buzz for one thing: an agent blocked on you. It now also
  notifies when a run *finishes* and when a session *ends* — and because every
  turn ends, "finished" only fires for runs longer than a threshold, so the
  twenty-second exchange you are watching stays silent while the thing you
  walked away from tells you it is done. Each kind is switchable per device
  (More → Notification settings), with the length threshold picked from Any /
  1m / 5m / 15m, so a phone and a tablet need not agree.
- **Spawn an agent from the command palette.** It already could, under a name
  nobody would search for.
- **Projects have a face.** A fleet of a dozen agent cards had exactly one thing
  saying which repo each belonged to: its cwd, in a tooltip. Every directory now
  carries a mark — initials from its name, a colour from its path — shown on the
  sidebar cards, the Fleet Deck, the spawn dialog, the overview, and the TUI's
  sidebar and roster. The point is that it works with **no configuration**: the
  fleet is legible the first time you look at it, and Settings → Projects is
  where you override something you didn't like rather than a form you must fill
  in before anything happens. Initials break on word boundaries, so `api-gateway`
  and `api-worker` don't both read `AP`. Override with an emoji, a favicon, a
  label or a colour; a project you have *named* names its agents too, instead of
  every one of them being called after the folder it happened to start in.
- **Per-project plugin settings.** A plugin marks a setting `"scope": "project"`
  and it is stored against the directory and edited on the Projects page beside
  that project's identity. Three plugins had already invented this privately —
  shiplight's repo, ci-watcher's repo, jira's directory-to-prefix map — each with
  its own storage and its own editor. A plugin reads the values through the
  `config.get` it can already declare, so this needed no new capability. Project
  scope names *which* thing; the plugin's own settings file still holds the key
  to it, and "project-scoped **and** secret" is refused when the plugin loads.
- **Forget a project.** A project entry outlived its directory: a repo you
  deleted kept its row, its icon and its plugin settings forever. The Projects
  page now marks the ones whose directory is gone and offers to forget them.
- **Claude's built-in skills are visible.** The Library pane showed zero skills
  in a repo with no `.claude/` of its own, and the Context pane showed eighteen
  names with nothing behind sixteen of them — both only ever looked in
  `<cwd>/.claude`. All the roots are read now (project, then user, then plugins),
  including `CLAUDE_CONFIG_DIR` for the first time, which is what Claude profiles
  set. Of eighteen skills a session reports, three are files; the other fifteen
  are compiled into the `claude` binary, so those now say **built-in** rather
  than leaving a blank row. A `Skill` tool call also gets a proper card — it was
  falling through to "the first string value in the input", so any invocation
  carrying arguments printed your argument text where the skill's name belongs.

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
- **The plugin docs now say `agent.snapshot` carries a conversation *window*.**
  It always described a "full per-agent snapshot"; nothing told an author the
  transcript on it is bounded, so the first plugin to read `data.conversation`
  would have got twelve turns with no way to know that was by design. The note
  points at `sessions.conversation` (`sinceSeq`) for anyone who needs real
  history, and at the wake-up-and-re-query pattern for anyone who does not.
- **Shiplight's pipeline widget shows a stopwatch, not an age.** A running
  pipeline gets a live `m:ss` clock and a finished one shows how long it took —
  "4m" looks identical whether a run is alive or wedged.
- **A pasted project icon is downloaded once, not hot-linked forever.** The URL
  used to go straight into an `<img src>`, so the mark vanished offline, the
  request repeated on every render, and the obvious way to get such a URL — a
  third-party favicon service — was told every project domain you work on. It is
  fetched once now and served from disk. `http(s)` only, 2 MB cap, 8s timeout,
  and it has to actually be an image; the file extension comes from the
  response's content type and never from the URL.
- **Your recent projects are a timestamp, not a capped list.** Opening a project
  used to shuffle a fixed-length array, so a project you had configured could
  fall off it because you opened eight others. Pins and ordering from an older
  config are read as they were, and never rewritten — downgrading loses nothing.

### Fixed
- **Web Push never worked on iPhone at all.** Every notification to
  `web.push.apple.com` came back `403` and always had: the VAPID `sub` claim was
  `mailto:workspacer@localhost`, and Apple requires a real domain or an `https:`
  URL. Chrome and Android accept it, so the one push service that refused it was
  the one the feature was built for — and the send path discarded its error
  without a log, so the refusal was invisible from the moment it shipped. Send
  failures are now logged with the push service and status code, and the phone's
  test button reports what was actually delivered rather than how many
  subscriptions it tried (it said "sent to 4 devices" while four of them were
  being refused). Existing subscriptions keep working; the subject is a claim
  inside the signed JWT, not part of the keypair.
- **The phone client could sit on "Offline" far longer than it needed to.** Its
  handshake watchdog gave every connection attempt a flat 4.5 seconds — but the
  case it exists for is a sleeping host or a cold tailnet link, which takes far
  longer than that. So it killed the very attempt it was written to survive,
  over and over, and only recovered once repeated tries had warmed the route.
  The budget now grows with the retry backoff (4.5s → 9s → 20s), and tapping
  the status pill while it reads "Offline" retries immediately instead of
  waiting out the backoff.
- **Your newest message stopped riding the top of the viewport.** Sending
  scrolled the transcript so your message sat at the top with the reply growing
  into the space below it; since 0.148.0 the message was landing at the bottom
  instead, with the blank space stranded below the fold. The spacer was being
  built correctly and never scrolled into: the scroll the pin itself issued was
  being read as *you* scrolling away, which unstuck the pin a few milliseconds
  after it armed.
- **Connecting a web or remote client took far longer than it should have.** The
  hub published every session's entire transcript on every flush of every
  session — for a 500-turn agent that is 2.73 MB per push, so a fleet of fifteen
  cost 41 MB before the window rendered anything and a single streaming agent
  pushed ~27 MB/s at every connected client. Snapshots now carry a bounded
  window of the newest turns (2.73 MB → 160 KB, 94% smaller; connect 41 MB →
  2.3 MB), anchored so a client holding full history splices it in rather than
  losing scrollback. This applied to the desktop too, not just browsers, any
  time remote sharing was on. Plugins that read `agent.snapshot` should treat
  the conversation as a window — see the note in the plugin docs.
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
- **Setting an icon on one project wiped every other project's identity.** Three
  config maps are replaced outright on save rather than merged, because a merge
  can only add keys — under one, an entry you just deleted would come straight
  back. The renderer trims every save down to what actually changed, so it has to
  leave those maps whole; both ends listed them and the lists drifted. The result
  was that saving one project shipped a one-entry map that the writer took as the
  whole truth, so every other project lost its icon, label, colour, pin and
  per-project plugin settings and fell back to derived initials. There is one
  list now, imported by both ends.
- **`push.test` reached the consent dialog as a raw method id.** The capability
  that provokes a test notification had no label, so the prompt asked you to
  approve `push.test` instead of "Send a test push notification" — the guard that
  catches exactly this had been failing, and was dismissed as pre-existing.

### Security
- **The library's MCP editor wrote API tokens in the clear.** It invites you to
  type a token into a server's `env` or `headers`, then wrote it as plain text
  into markdown under `.workspacer/library` — which the service's own header
  describes as per-repo and committable. No attacker was needed; `git add -A` was
  enough. Both fields are now redacted on the way out, and an echoed placeholder
  resolves back against what is stored so a round-trip through the masked editor
  keeps the real token. Redaction is the default and the spawn paths opt in by
  name: a missed redaction leaks a live credential, while a missed *un*-redaction
  produces a visibly broken token and a bug report.
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
