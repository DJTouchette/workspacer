---
title: Native fleet wakes parse the shared contract grammar
date: 2026-10-05
confidence: medium
suggested_doc: chat-tool-rendering
related_paths:
  - apps/native/src/transcript.rs
  - apps/native/src/ui/fleet_card.rs
  - contracts/fleet-message-cases.json
promoted: false
---

# Native fleet wakes parse the shared contract grammar

## Observation
Fleet/supervisor wakes arrive as user_message rows. Native transcript::fleet now ports desktop ENTRY_RE (cwd|approval|question, stopped/killed, FAILED, crossed, NEEDS A DECISION, last reply/reports) and is tested against contracts/fleet-message-cases.json. Blocked wakes put approval/question where the cwd would be, so their cwd is not recoverable. ui/fleet_card.rs renders the card (named for the worker or 'N sessions', live status prefers current session state: Needs approval -> Resolved), with Open guarded to sessions whose parent is empty or the viewed conversation.
