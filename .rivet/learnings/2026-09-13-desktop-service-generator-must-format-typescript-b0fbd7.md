---
title: Desktop service generator must format TypeScript before writing
date: 2026-09-13
confidence: high
related_paths:
  - apps/desktop/scripts/gen-desktop-services.mjs
  - apps/desktop/src/main/shared/desktopServices.generated.ts
promoted: false
---

# Desktop service generator must format TypeScript before writing

## Observation
gen-desktop-services.mjs emitted raw JSON as a TS const, which disagreed with desktop Prettier singleQuote/object-key formatting. Every build regenerated formatting drift in desktopServices.generated.ts after format:check cleanup. Formatting only the TS output using Prettier format/resolveConfig before the write-if-changed guard makes repeated generation byte-stable; Go capability registry/gate emission remains unchanged.
