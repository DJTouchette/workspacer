---
title: wks-result's fenced-block extraction is fleet-wake-only, not a general chat mechanism
date: 2026-09-05
confidence: high
suggested_doc: chat-tool-rendering
related_paths:
  - apps/desktop/src/main/shared/structuredResult.ts
  - apps/desktop/src/renderer/src/components/markdown.tsx
  - apps/desktop/src/renderer/src/components/claude/StructuredResultCard.tsx
promoted: false
---

# wks-result's fenced-block extraction is fleet-wake-only, not a general chat mechanism

## Observation
No provider adapter (services/claudemon/src/providers/*.rs) has an HTML/card content kind on the wire — AgentUpdate only has AssistantText/ToolUse/ToolResult/Error/mode variants, and Error itself is folded into AssistantText prose because "the renderer only knows fixed item kinds" (claudemon-providers doc). So any new model-facing content kind must ride as a fenced code block inside ordinary assistant text. The exact reusable algorithm already exists in apps/desktop/src/main/shared/structuredResult.ts (RESULT_FENCE='wks-result', FENCE_RE regex, last-tagged-block-wins, RESULT_SCHEMA_MAX/RESULT_MAX caps, never-throws parse) — but that module runs in the MAIN process over HOST-CONSTRUCTED fleet-wake text (buildFleetMessage in fleetMessages.ts), consumed only by StructuredResultCard inside FleetMessageCard. It is a Fleet-Manager mechanism, not on the path a model-emitted fenced block in an arbitrary session's own reply actually takes. That path is the RENDERER's markdown.tsx parseMarkdownBlocks (called from ConversationMessage.tsx), whose fence loop (markdown.tsx:398-430) captures a `lang`/`info` string but never branches on its value today — CodeBlock treats it as a pure syntax-highlight hint. Also verified: zero uses of dangerouslySetInnerHTML anywhere in apps/desktop, and no markdown library (marked/remark/rehype/dompurify/react-markdown) in package.json — the whole markdown renderer is hand-rolled React with no existing raw-HTML-injection precedent.

## Impact
Anyone building a new model-facing structured-content feature (e.g. HTML response cards) will be tempted to extend/reuse structuredResult.ts directly since it looks like the exact right tool — but it only fires for fleet-dispatch wake text with a resultSchema, not for ordinary chat turns in any session. Building on it instead of markdown.tsx's fence loop would silently only work inside Fleet Manager conversations.

## Recommendation
For any new fenced-block content contract meant for ordinary chat (not fleet wakes), add the branch inside markdown.tsx's parseMarkdownBlocks fence-handling loop (alongside the existing CodeBlock/drawnTableFromBlock special-casing), reusing structuredResult.ts's ALGORITHM (tag-match, last-block-wins, size caps, never-throws) under a new tag constant — not its call site.
