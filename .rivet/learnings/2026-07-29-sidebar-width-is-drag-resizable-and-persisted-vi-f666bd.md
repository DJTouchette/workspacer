---
title: Sidebar width is drag-resizable and persisted via config.ui.sidebarWidth
date: 2026-07-29
promoted: true
---

# Sidebar width is drag-resizable and persisted via config.ui.sidebarWidth

## Observation
SIDEBAR_WIDTH is gone from SideBar.tsx — the expanded width is now a prop, and the constants live in renderer/src/lib/sidebarWidth.ts (SIDEBAR_DEFAULT_WIDTH 296, SIDEBAR_RAIL_WIDTH 74, min 220 / max 560, plus a 45%-of-viewport ceiling). Non-obvious pieces: (1) resolveSidebarWidth() re-clamps on every window resize but deliberately does NOT commit, so a width dragged out on a big monitor survives a session on a laptop and comes back; (2) the drag writes React state per frame (rAF-coalesced) and only calls saveConfig on pointerup / after a 400ms keyboard-repeat window — a config write per mousemove would hammer config.yaml through the mtime-gated writer; (3) SidebarResizeHandle uses pointer capture, not window listeners, because a browser/plugin webview pane swallows pointer events and the drag would stick; (4) the mobile overlay passes width=undefined so a desktop drag can't size a phone drawer. Verified live in Chromium against the sidebar harness (which now mounts the handle): 1:1 tracking, clamp at 220, double-click reset to 296.

## Disposition
Folded into .rivet/context/domains/config.md (write-throttling pattern for high-frequency UI config).
