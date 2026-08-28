---
title: Fleet wake extras (structured result, full reply) never reached the GUI card — only bullets were parsed
date: 2026-08-23
confidence: high
suggested_doc: chat-tool-rendering
promoted: false
---

# Fleet wake extras (structured result, full reply) never reached the GUI card — only bullets were parsed

## Observation
buildFleetMessage emits post-bullet BLOCKS after a blank line (FAILED note, STOPPED note, `Structured result — <label> (session:<id>):` + pretty JSON, `Structured result MISSING — …`, `Full final message — …`), but parseFleetMessage's bullet loop `break`s at the first non-bullet line, so every one of those blocks was dropped. Consequence: a dispatch with a `resultSchema` produced a validated object the manager AGENT could read in the wake text, while the GUI card showed no trace of it at all — not a raw JSON dump, nothing. `resultError` was equally invisible. Fixed 2026-08-23 (01ad83f7): attachResultBlocks() folds `result`/`resultError` back onto their entries by sessionId and STOPS at the first `Full final message — ` block, because a full reply is arbitrary worker prose with blank lines whose paragraphs would otherwise be scanned as blocks (and could forge a result). `fullReply` deliberately still does not round-trip.</observation>
<parameter name="impact">Anything added to a wake as an extras block is invisible to FleetMessageCard until the parser is taught to fold it back. The RESULT_MAX cap also means a valid result can arrive as INVALID JSON (truncated mid-object with a `[truncated: …]` marker), so any renderer of `entry.result` must tolerate unparseable input.</impact>
<parameter name="recommendation">When adding a new extras block to buildFleetMessage, add the matching fold in attachResultBlocks and a round-trip test in fleetMessages.test.ts, or the GUI silently loses it. Render structured results through components/claude/StructuredResultCard (+ structuredResultFields), which classifies by value shape because the schema is authored per dispatch.</recommendation>
<parameter name="related_paths">["apps/desktop/src/main/shared/fleetMessages.ts", "apps/desktop/src/main/shared/structuredResult.ts", "apps/desktop/src/renderer/src/components/claude/StructuredResultCard.tsx", "apps/desktop/src/renderer/src/components/claude/structuredResultFields.ts"]
