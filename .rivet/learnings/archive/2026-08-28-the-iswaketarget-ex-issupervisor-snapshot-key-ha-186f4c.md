---
title: The isWakeTarget (ex-isSupervisor) snapshot key has a 5-file lockstep set, and one of them is a screenshot fixture
date: 2026-08-28
confidence: high
suggested_doc: session-lifecycle
related_paths:
  - services/hub/cmd/brain/enrich.go
  - services/hub/cmd/brain/fleetview.go
  - services/hub/cmd/hub/mobile.html
  - apps/desktop/src/main/services/hubCapabilities.ts
  - apps/desktop/scripts/shootMobile.mjs
promoted: true
promoted_to: session-lifecycle
---

# The isWakeTarget (ex-isSupervisor) snapshot key has a 5-file lockstep set, and one of them is a screenshot fixture

## Observation
Renamed isSupervisor -> isWakeTarget 2026-08-28. The flag is NOT persisted anywhere (no claudemon field — grep of services/claudemon is empty; no sqlite, no config.yaml, no boot doc; the brain's metaStore and claudeSessionStore are both process memory), but it IS a live hub-bus JSON key, so the producers and consumers can only move together. The complete lockstep set is five files, and only four are findable by thinking about "the wire": services/hub/cmd/brain/enrich.go (m["isWakeTarget"] on enrichSnapshot), cmd/brain/fleetview.go (json tag), cmd/hub/mobile.html (isManager reads s.isWakeTarget), apps/desktop/src/main/services/hubCapabilities.ts (agents.list row + sessions.snapshots spread) — plus apps/desktop/scripts/shootMobile.mjs, whose session() helper spreads the key STRAIGHT into the snapshot mobile.html renders. Miss that fifth and nothing fails: the staged mobile screenshots just quietly lose the MANAGER chip and the crew nesting. parity_test.go's nestingFieldsRequired greps mobile.html for the literal field name, so Go and mobile.html are forced into one commit by the test, but nothing at all guards the fixture. Rust is not in the set: apps/tui and services/claudemon have zero references. The desktop renderer has zero FUNCTIONAL references too (two comments only).</observation>
<parameter name="recommendation">When renaming or removing a field on the enriched snapshot, treat apps/desktop/scripts/shoot*.mjs as first-class wire consumers alongside mobile.html — they are fixtures by directory but producers by behaviour, and no test covers them. Also note the field named isSupervisor never meant a role: its only input is opts.manager, which is why the rename was safe. Do NOT confuse it with the three surviving namesakes that mean different things: the '[supervisor]' blocked-wake wire prefix in main/shared/fleetMessages.ts (parsed in three places incl. mobile.html, round-trip tested), app.supervisorHome / ensureSupervisorHome (persisted IPC channel), and services/hub/internal/supervisor + internal/nodes/supervisor.go (OS process supervision).</recommendation>
<parameter name="impact">A partial rename of this key degrades silently rather than failing: /m flattens the fleet into an undifferentiated list with no manager chip, and the worker-finished wake router stops recognising managers.
