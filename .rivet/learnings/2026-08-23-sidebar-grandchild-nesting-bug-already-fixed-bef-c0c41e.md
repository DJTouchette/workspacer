---
title: Sidebar grandchild-nesting bug already fixed before this task started
date: 2026-08-23
confidence: high
suggested_doc: mission-control-attention
related_paths:
  - apps/desktop/src/renderer/src/components/SideBar.tsx
  - apps/desktop/src/renderer/tests/components/sidebarGrandchildNesting.test.tsx
promoted: false
---

# Sidebar grandchild-nesting bug already fixed before this task started

## Observation
SideBar.tsx's fleet-nesting logic (rootOf/childrenByParent, lines ~826-848) already walks the full parentId chain to the top-most resolvable ancestor and flattens all descendants (children AND grandchildren+) into that root's bucket. This was fixed in commit a394bd4d "fix(sidebar): stop dropping fleet grandchildren from nesting" (merged via 57dde47b), which is already an ancestor of HEAD on this branch. A companion test file tests/components/sidebarGrandchildNesting.test.tsx already covers the grandchild case and passes.

## Impact
A task brief cited "SideBar.tsx:1342-1349 only expands children of top-level cards" as a live bug, but that observation was stale by the time this task ran — the fix landed the same day. Saves a future worker from re-fixing something already done.

## Recommendation
Before acting on a bug report against SideBar.tsx's nesting logic, check git log -- SideBar.tsx first; this area churns fast.
