---
title: Chat column width is one CSS token, not six literals
date: 2026-07-29
promoted: true
---

# Chat column width is one CSS token, not six literals

## Observation
The centered chat measure was maxWidth: 1040 hardcoded in six places (ClaudePane transcript + GUI status row, Composer, TasksCard, NeedsYouDock x2) plus 900 in AgentWatchPane — they had to agree or the composer/dock/status row stop sitting flush under the transcript. Now all seven read --wks-chat-width from App.css :root (900px, narrowed from 1040 to match competitor reading measure). It is NOT a theme token, so applyTheme() never touches it; changing the chat width is a one-line edit in App.css :root. Documented in apps/desktop/DESIGN_LANGUAGE.md section 4.

## Disposition
Folded into .rivet/context/domains/chat-tool-rendering.md (--wks-chat-width note).
