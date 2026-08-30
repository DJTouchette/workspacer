---
title: Remote Control dialog is tabbed and wide
date: 2026-08-17
promoted: true
---

# Remote Control dialog is tabbed and wide

## Observation
2026-08-17: RemoteShareDialog restructured — 660px wide, feature tabs Phone / Machines / Server (ModeTab reused as the strip). Phone tab = share toggle + QR-left/controls-right two-column EnabledState; Machines tab = LinkedMachinesSection (its section border + Link2 header removed — the tab strip is the header now); Server tab = RemoteClientSection promoted out of its <details> 'Advanced' collapse. Tab visibility: remote-client mode shows ONLY Server (strip hidden at 1 tab); Server tab requires window.electronAPI.setRemoteServer (absent in web mirror). Dialog title changed 'Phone access' -> 'Remote Control', matching what landing/docs.html already called it.

## Disposition
Not promoted — a UI-layout description of RemoteShareDialog.tsx's current tab structure, not an invariant/trap/API behavior. This dialog has already been restructured twice in a month; a context doc listing its current tabs would itself go stale on the next redesign. Left in the log.
