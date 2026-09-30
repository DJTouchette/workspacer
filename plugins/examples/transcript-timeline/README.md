# Timeline — transcript replay plugin

A webview-only plugin that turns any agent session's transcript into a
scrubbable timeline: start at 0 and drag right to watch the conversation,
tool calls, and file edits replay in order.

- **Session picker** — live agents (from `agents.list`) plus historical
  sessions recorded on disk (from `claude.sessionsForDir`), grouped in one
  dropdown. Opening the pane inside an agent workspace preselects that agent.
- **Scrubber** — the slider position N renders the first N timeline events.
  Colored ticks mark prompts (blue), file edits (green), commands (amber),
  and errors (red). `←`/`→` step one event; `space` replays automatically.
- **File changes panel** — folds `Write`/`Edit`/`MultiEdit` calls up to the
  scrub position into per-file change stacks. Files first created by a full
  `Write` are reconstructed exactly as they stood at that moment (latest
  change highlighted); edit-only files show their diff stack, since the
  transcript never carried the original base content.
- **Live tail** — running sessions keep growing at the right edge via
  incremental `sessions.conversation { sinceSeq }` polling; "follow live"
  keeps the scrubber pinned to now.
- **Worktree replay** — the `⎇ Worktree` toggle materializes the replay
  into a real, disposable git worktree that physically follows the
  scrubber. `replay.open` resolves the repo's last commit before the
  session started and checks it out detached under the OS temp dir;
  every scrub re-seeks it (`replay.seek` resets to that base and
  re-applies the session's Write/Edit ops up to the cursor); toggling
  off (or closing the pane) removes it via `replay.close`. The agent's
  real checkout is never touched, and ops that no longer apply cleanly
  (path outside the repo, file diverged) are skipped and reported in
  the replay bar rather than half-applied. Best-effort by nature:
  changes made outside tracked tools (shell commands, manual edits)
  aren't in the transcript, so they don't materialize.

Data comes from two capabilities, normalized into one item shape:
`sessions.conversation` (claudemon's structured log, used for sessions the
daemon tracks) and `sessions.transcript { sessionId, cwd }` (raw parsed
JSONL — the `cwd` lets claudemon resolve historical sessions from
`~/.claude/projects` after the daemon has forgotten them).

No sidecar and no build step: the hub serves `ui/` statically and the page
connects straight to the bus with its pane token.
