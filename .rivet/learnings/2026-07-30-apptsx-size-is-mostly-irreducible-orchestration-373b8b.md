---
title: App.tsx size is mostly irreducible orchestration; the extractable parts were pure policies
date: 2026-07-30
promoted: true
---

# App.tsx size is mostly irreducible orchestration; the extractable parts were pure policies

## Observation
Audited apps/desktop/src/renderer/src/App.tsx (2437 prod lines, 88 hook calls) for decomposition on 2026-07-30. Its only tested export was migrateSessionData; the App() body had zero direct tests. The genuinely extractable clusters turned out to be small pure policies, not big feature slabs: browser-pane hibernation (now lib/hibernation.ts + hooks/useBrowserHibernation.ts), session-snapshot promote/evict/prune (now hooks/useSessionSnapshots.ts, with shouldEvictSession + omitSession added to lib/promoteSessionSnapshots.ts), and text scaling (now lib/textScale.ts). That removed ~120 lines. The remaining ~2300 is JSX (~400) plus orchestration that wires 20+ hooks together with 64 useCallbacks — irreducible without inventing indirection. Two behaviours worth knowing, both previously unpinned: (1) hibernation only ever stamps panes in the ACTIVE tab, so a browser pane in a tab never focused this run has no sighting and is NEVER hibernated (the lastSeen > 0 guard) — restored layouts keep those webviews until opened once; (2) the old setTextScale had no NaN guard, so a non-finite value flowed through Math.min/max to NaN, compared !== to the current scale, and would have written uiFontScale: NaN into config.yaml; clampTextScale now returns the default instead.

## Disposition
Folded into .rivet/context/paradigms/hotspots.md (App.tsx guideline).
