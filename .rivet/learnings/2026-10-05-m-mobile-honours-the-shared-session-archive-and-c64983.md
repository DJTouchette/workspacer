---
title: /m mobile honours the shared session archive and holds the fleet until it is read
date: 2026-10-05
confidence: high
suggested_doc: remote-mobile
related_paths:
  - services/hub-rs/assets/web/mobile.html
  - apps/desktop/tests/e2e/mobileArchive.test.ts
promoted: false
---

# /m mobile honours the shared session archive and holds the fleet until it is read

## Observation
services/hub-rs/assets/web/mobile.html reads sessionArchive.get after the hello frame (scope known) on every connect, subscribes sessionArchive.changed (ignores peer-stamped copies), and renderFleet shows 'Loading fleet…' until the first read answers so archived rows never flash. listedFleet(filter) excludes archived sessions from every filter except 'archived' (chip only shown when something is archived); fleetRoster builds from that list so an archived manager's live crew become roots. Attention (Waiting tab, status pill, push) still counts archived sessions. Archive/Restore live in the chat ⋯ sheet and on archived resumable rows, gated on can('sessionArchive.set') so view tokens get read-only.

## Impact
Any new /m list must go through listedFleet or archived sessions resurface on the phone.
