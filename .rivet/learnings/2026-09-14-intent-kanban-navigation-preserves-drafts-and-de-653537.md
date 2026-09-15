---
title: Intent Kanban navigation preserves drafts and delegates lifecycle actions
date: 2026-09-14
promoted: false
---

# Intent Kanban navigation preserves drafts and delegates lifecycle actions

## Observation
IntentBoard groups persisted draft/active/review/complete statuses. Needs me combines review status with live session attention. Card actions and cross-column drops open the existing overview/start or review workflow; they never issue status updates or accept evidence. IntentWorkspaces keeps selected intent and draft state while the detail panel is hidden, avoiding auto-selecting an intent when the board first loads. On narrow screens the inspector covers the board and Back to board returns to it. The browser shell test now exercises board navigation at 320, 768 and 1280 pixels.
