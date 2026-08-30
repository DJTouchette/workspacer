---
title: A new brief.* (or any path-bearing) capability needs five registrations, not one
date: 2026-08-24
confidence: high
suggested_doc: mcp-tool-facade
promoted: true
promoted_to: registration-checklists
---

# A new brief.* (or any path-bearing) capability needs five registrations, not one

## Observation
Adding `brief.archive` to hubCapabilities.ts is only the first of five places. The name matches `pathVerbPrefixes` ("brief."), so `LooksPathBearing` is true and the bus FAILS CLOSED until it is classified everywhere: (1) capspec.PathParam needs the params field that carries the path; (2) compositionInert needs an entry whose Reason literally contains the witness symbol `assertPathAllowed` (TestInertClaimsCarryACheckedWitness greps the prose for it) and is at least 60 chars; (3) contracts/path-containment-cases.json needs a `methods` entry with the same field name, since capspec_test fails in BOTH directions if the fixture and PathParam disagree; (4) hubCapabilitiesGuards.test.ts sweeps that fixture entry and has TWO hard-coded ratchets on the count of 'main'-owned methods (mainSweep and tildeSweep) that must be bumped together; (5) CAP_LABELS in renderer/src/lib/pluginPermissions.ts, whose drift guard greps hubCapabilities.ts for `registerCapability|cat` and fails on any unlabelled method. Miss any one and a different suite fails, each with an unrelated-looking message.</observation>
<parameter name="impact">Costs a full round of confusing failures across Go and TS suites if discovered one at a time. The fixture/ratchet pair in particular fails with "the corpus was not swept" rather than anything naming the new method.</impact>
<parameter name="recommendation">When adding any capability under fs./search./library./git./providers./brief., do all five in one pass before running anything, then `go test ./internal/capspec/...` plus the desktop hubCapabilitiesGuards suite.</recommendation>
<parameter name="related_paths">["services/hub/internal/capspec/*.go", "contracts/path-containment-cases.json", "apps/desktop/src/main/services/hubCapabilities.ts", "apps/desktop/src/main/services/hubCapabilitiesGuards.test.ts", "apps/desktop/src/renderer/src/lib/pluginPermissions.ts"]
