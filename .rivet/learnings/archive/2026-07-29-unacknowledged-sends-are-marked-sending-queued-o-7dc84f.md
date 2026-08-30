---
title: Unacknowledged sends are marked Sending/Queued on the bubble
date: 2026-07-29
promoted: true
---

# Unacknowledged sends are marked Sending/Queued on the bubble

## Observation
ConversationMessage gained pending:'sending'|'queued' — dimmed + dashed bubble with a clock marker until the daemon echoes the turn back (the existing consumedUserCountRef dequeue IS the acknowledgement; nothing new was added to the lifecycle, it's just rendered now). pending is derived positionally in renderedConversation (pendingFrom = conversation.length - optimisticMessages.length; the dequeue is FIFO). queued vs sending is captured at send time from !ambientIdle || pendingCount>0 — note ambientIdle counts 'background' as idle but not 'waiting_approval', so a send during an approval pause is correctly Queued. Trap: the optimistic turn object must NOT be built inside the setOptimisticMessages updater, because two failure paths remove it by object identity (prev.filter(t => t !== optimisticTurn)) — hence pendingCountRef instead of the updater's prev.length.

## Disposition
Not folded: already reflected in .rivet/context/domains/chat-tool-rendering.md (hand-authored notes 2026-07-29, unacknowledged sends).
