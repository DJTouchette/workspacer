---
title: Chat transcript pins your message to the top via a derived tail spacer
date: 2026-07-29
promoted: true
---

# Chat transcript pins your message to the top via a derived tail spacer

## Observation
ClaudePane's new tail spacer (src/lib/chatScroll.ts) is DERIVED, not animated: height = anchorTop - 12 + viewportHeight - contentHeight(excluding the spacer). Because the spacer shrinks by exactly what the streaming reply grows, total scrollHeight holds still and the existing sticky-bottom ResizeObserver keeps the pinned user message at the top with no extra scroll math. Two non-obvious consequences: (1) every at-bottom test must go through distanceFromContentEnd() — raw scrollHeight-scrollTop-clientHeight now reads the dead space as 'user scrolled away' and would hide autoscroll + show the scroll-to-bottom button right after a send; (2) the pin is armed by handleSend (pinArmedRef), never on mount, so a restored transcript still opens at its natural bottom instead of with a blank tail. Verified against real Chromium layout (jsdom has no layout, so the ClaudePane tests can only assert wiring: [data-tail-pad] absent before a send, present after, [data-pin-anchor] on the newest user turn).

## Disposition
Folded into .rivet/context/domains/chat-tool-rendering.md (tail-spacer pin note, incl. the 2026-07-30 raw-vs-discounted distance correction).
