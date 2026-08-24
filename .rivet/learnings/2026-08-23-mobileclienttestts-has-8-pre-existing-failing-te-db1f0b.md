---
title: mobileClient.test.ts has 8 pre-existing failing tests on current master
date: 2026-08-23
confidence: high
related_paths:
  - apps/desktop/tests/e2e/mobileClient.test.ts
  - apps/desktop/tests/e2e/fixtures/mobileHub.ts
  - services/hub/cmd/hub/mobile.html
promoted: false
---

# mobileClient.test.ts has 8 pre-existing failing tests on current master

## Observation
As of commit 52730223, running `npx playwright test tests/e2e/mobileClient.test.ts` fails 8/21 tests (fleet ordering, filters, needs-you inbox, chat tab bar, pending question composer, spawn flow, deep-link, empty-conversation re-render) even on a completely clean checkout with no code changes. Verified by stashing all edits and re-running — same 8 fail, same 13 pass. Not caused by any Stop/SIGTERM work.

## Impact
Anyone touching services/hub/cmd/hub/mobile.html and running this suite for verification will see red and may wrongly attribute it to their change. Always diff against a stash/clean-checkout run before concluding a change broke something here.

## Recommendation
Someone should investigate why these 8 fail (possibly a snapshot/fixture drift in mobileHub.ts vs mobile.html's current rendering) and fix or skip them; until then, treat them as known-red baseline.
