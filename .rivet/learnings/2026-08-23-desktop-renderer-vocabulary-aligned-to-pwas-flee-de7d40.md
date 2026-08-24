---
title: Desktop renderer vocabulary aligned to PWA's fleet-manager language (2026-08-23)
date: 2026-08-23
confidence: high
suggested_doc: remote-mobile
related_paths:
  - apps/desktop/src/renderer/src/**
  - services/hub/cmd/hub/mobile.html
promoted: true
---

# Desktop renderer vocabulary aligned to PWA's fleet-manager language (2026-08-23)

## Observation
The mobile PWA (services/hub/cmd/hub/mobile.html) was migrated to fleet-manager vocabulary in commit 8d5db890 ("Mobile PWA: make /m read as the Fleet Manager, not a session list"). Key renames: state badges Working→'In flight', Idle→'Standing by', Finished→'Landed'; verb Spawn→Dispatch (CTA/tooltips/toasts); inbox tab 'Needs you'→'Waiting'. NOT renamed in the PWA (so left alone in the desktop too): approval/question language ('Needs approval', 'Waiting for input'), and 'agent' as the general noun (PWA still uses 'agent' pervasively — only a few narrow phrases say 'worker'). Applied the same renames to apps/desktop/src/renderer/src/ (SideBar, FleetDeck, AgentCard, AttentionCard, InboxDrawer, OverviewPane, SpawnAgentDialog, HomeSpace, AskPane, NotificationsSection, ClaudeProfilesSection, JobsSection, LibraryPane, Onboarding, CommandPalette, shortcuts.ts, pluginPermissions.ts, useAttentionFeed, AgentWatchPane) plus matching test-string updates.

## Impact
Deliberately NOT renamed, and why — future sweeps should keep these exceptions: (1) components/claude/WorkingTimer.tsx's default 'Working' verb — no PWA equivalent exists, and renaming it broke 8+ test assertions across 3 files while mismatching the component's own name; left as-is. (2) InboxDrawer's 'Review' tab and AttentionCard/SideBar's bigdiff 'Review'/'Review changes' labels — NOT renamed to 'Landed' despite PWA doing so, because 'Review' is already a distinct, load-bearing feature name elsewhere in this codebase (the git-diff ReviewPane / paneMenu 'review' pane type) and renaming would create ambiguity between two unrelated 'Review' concepts. (3) 'agent' noun kept throughout (not swapped to 'worker') — matches the PWA's own restraint; only the state/verb vocabulary moved.

## Disposition
Promoted into .rivet/context/domains/remote-mobile.md (hand-authored notes 2026-08-23, vocabulary alignment + named exceptions).
