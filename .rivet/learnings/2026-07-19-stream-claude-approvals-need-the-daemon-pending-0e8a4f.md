---
title: Stream-Claude approvals need the daemon pending fold, not hooks
date: 2026-07-19
promoted: true
---

# Stream-Claude approvals need the daemon pending fold, not hooks

## Observation
Approval cards (needs-you dock / sidebar / inbox / fleet) come from snapshot.pendingApproval, surfaced by useAttentionFeed. That field is set two ways: hookEventRouter's PermissionRequest case (PTY Claude), or claudeSessionStore.applyManagedMode folding the daemon's Pending::Approval slot into pendingApproval. The fold was gated on provider !== 'claude', with the comment 'Claude PTY and stream keep the hook path.' That's WRONG for stream: headless stream-json Claude (--permission-prompt-tool stdio) routes approvals through the control protocol (can_use_tool), handled in claude_stream.rs surface_approval -> set_managed_mode(Approval, Pending::Approval) — there is NO PermissionRequest hook in stream mode. So stream-Claude approvals lived only in the daemon pending slot, which the desktop threw away -> ambient flipped to waiting_approval but pendingApproval stayed null -> nothing surfaced anywhere. Fixed 2026-07-20 (commit 64cdccb): fold when provider !== 'claude' OR transport === 'stream'; PTY still defers to the hook path so they never race. ClaudePane has NO inline approval card — approvals only ever surface via the attention system. Codex/OpenCode were always correct (non-claude branch); if codex approvals 'don't show' it's the yolo auto-approve, not a code gap.

## Disposition
Folded into .rivet/context/domains/mission-control-attention.md (approvals-are-transport-shaped notes).
