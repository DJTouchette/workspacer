---
title: resolveAttention drifted from ClaudePane on transport
date: 2026-07-19
promoted: true
---

# resolveAttention drifted from ClaudePane on transport

## Observation
There are TWO resolve paths for approvals/questions and they must stay in sync: (1) ClaudePane's handleApprovalRespond/handleAnswer (the inline card in the agent pane) and (2) lib/resolveAttention.ts resolveApproval/resolveAnswer (the SHARED path for Triage Inbox, needs-you dock, FleetDeck, SideBar). ClaudePane guards both on '!hasTerminal || !isClaude' — correctly treating stream-transport Claude as no-PTY. resolveAttention only knew about provider, never transport, so for stream Claude resolveAnswer took the keystroke branch on its PRIMARY path (writing picker digits into a nonexistent PTY => answering an AskUserQuestion from the inbox silently did nothing); resolveApproval had the milder version (keystrokes only in the /approve failure catch). Fixed 2026-07-20 (bdf8ceb) by threading transport from the snapshot and treating 'stream' as no-PTY. Daemon side was already right: post_approve tries submit_managed_decision first and post_answer tries submit_managed_answer first, both returning managed:true for stream/codex before any PTY wrapper lookup. resolveAttention.ts's own header names ClaudePane as source of truth — check it whenever ClaudePane's approval/answer guards change.

## Disposition
Folded into .rivet/context/domains/mission-control-attention.md (2026-07-19/20 hand-authored notes).
