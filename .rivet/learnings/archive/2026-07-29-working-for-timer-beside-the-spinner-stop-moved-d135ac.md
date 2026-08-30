---
title: Working-for timer beside the spinner; Stop moved into the composer
date: 2026-07-29
promoted: true
---

# Working-for timer beside the spinner; Stop moved into the composer

## Observation
ClaudePane's streaming row is spinner + WorkingTimer ('Working for 1m35s'); the old inline cancel button is gone (.wks-stop-btn deleted) and Stop now lives in the composer beside send (.wks-composer-stop, Composer working/onStop props) — beside, not replacing, because a message typed mid-turn is queued for the next turn. Non-obvious: (1) the reset effect must check isStreaming BEFORE ambientIdle, since the optimistic post-send bridge is streaming while ambientState is still idle — the idle-first version left the label blank for the whole settle window and a test caught it; (2) the start comes from the newest user turn's timestamp read through a ref, so the count survives remount/attach mid-turn and a queued follow-up can't restart it; (3) the timer owns its own interval so a per-second tick doesn't repaint the transcript; (4) aria-label went Cancel -> Stop, which three ClaudePane tests assert on. fmtDuration gained an hours branch (1h02m), shared with subagent/workflow rows.

## Disposition
Not folded: already reflected in .rivet/context/domains/chat-tool-rendering.md (hand-authored notes 2026-07-29, working timer).
