---
title: Resume race: register_spawn left Stopped rows stopped
date: 2026-07-19
promoted: true
---

# Resume race: register_spawn left Stopped rows stopped

## Observation
claudemon's register_spawn reuses an existing SessionState row on resume (states.entry().or_insert) but did not reset its mode — a resumed session reported mode=stopped until Claude's SessionStart hook landed seconds later. The desktop's verifyAttachTarget (added in the 2026-07-18 lifecycle hardening) probes GET /sessions/:id right after attaching and treats mode=stopped as dead, so every RECENT-sidebar resume / stopped-agent respawn lost the race: viewer torn down, '[Claude session exited]' banner, pane deaf even though claude was starting fine. Fixed 2026-07-19: register_spawn flips Stopped→Unknown (clears pending, bumps updated_at) like register_managed already did with Input; verifyAttachTarget now only tears the viewer down on 404 (row truly gone) — for mode=stopped it fires terminal:exit but keeps the SSE viewer attached, because a respawn revives the SAME session id and the still-open stream (plus SSE backoff retry) is what brings the pane back without a remount. Also: exitNotified flag on SessionStream dedupes the double banner from React remount racing the first verify fetch.

## Disposition
Folded into .rivet/context/domains/session-lifecycle.md (resume/restart hardening notes).
