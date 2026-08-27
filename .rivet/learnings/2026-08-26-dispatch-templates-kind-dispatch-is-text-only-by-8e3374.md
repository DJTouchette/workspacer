---
title: Dispatch templates: kind 'dispatch' is text-only by construction
date: 2026-08-26
promoted: false
---

# Dispatch templates: kind 'dispatch' is text-only by construction

## Observation
Library kind 'dispatch' (7317b1c5) carries template text + optional default resultSchema and NO spawn-argument fields, so a template file cannot smuggle toolScope/cwd/model/worktree — the no-trust-boundary property is pinned in libraryDispatch.test.ts (TS) and TestLibraryDispatchRoundTrip (Go). Rendering is host-side in hubCapabilities agents.spawn via lib/dispatchTemplate.ts, which deliberately does NOT reuse the renderer's applyTemplate: placeholders are required by default and an unfilled one REFUSES the spawn naming the param (applyTemplate silently defaults, which is right for its form dialog and wrong for dispatch). The headless brain declines template/templateParams in spawnParamsDeclined because the default-schema half rides the already-declined resultSchema machinery. Seeds (ship-task, scout-task, two-explanations) exist in BOTH seeders — libraryService.seedGlobalIfEmpty and brain seedLibraryIfEmpty — and TestLibrarySeedAndList pins the seed count (7), so adding a seed means touching both plus that test.
