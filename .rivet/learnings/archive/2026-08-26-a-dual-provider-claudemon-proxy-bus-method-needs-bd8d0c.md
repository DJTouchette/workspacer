---
title: A dual-provider claudemon-proxy bus method needs SEVEN registrations — a different checklist from the brief.* four
date: 2026-08-26
suggested_doc: hub-shared-cap-event-vocabulary
related_paths:
  - services/hub/cmd/brain/handlers.go
  - services/hub/internal/capspec/capspec.go
  - services/hub/internal/authtoken/authtoken.go
  - apps/desktop/src/main/services/hubCapabilities.ts
  - apps/desktop/src/renderer/src/lib/pluginPermissions.ts
promoted: true
promoted_to: registration-checklists
---

# A dual-provider claudemon-proxy bus method needs SEVEN registrations — a different checklist from the brief.* four

## Observation
`sessions.subagentConversation` (7f4cf276) is the worked example of the OTHER new-bus-method shape: not path-bearing, but registered by BOTH the Go brain and the desktop. It needed seven entries, each with its own drift guard that fails the build if you miss it:

1. `cmd/brain/handlers.go` — the `registry.methods()` list AND the `handle()` switch (two edits, one file).
2. `main/services/hubCapabilities.ts` — `registerCapability`, the desktop's competing provider.
3. `cmd/brain/delegation_guard_test.go` `declaredOverlap` — required PRECISELY BECAUSE it now has two providers; the router is single-owner per method, so an undeclared collision fails `TestMainOwnedCapabilitiesDoNotCollideWithTheBrain`.
4. `internal/capspec/capspec.go` `inertMethods` — else `capspec_guard_test.go` / `capspec_test.go` reject it as "capspec says nothing about it".
5. `internal/capspec/httproutes.go` — the claudemon HTTP twin route, `TwinKind: TwinMethod`.
6. `internal/authtoken/authtoken.go` `viewMethods` — exact-name allowlist; omit it and web/remote scoped tokens are refused, which is the failure that presents as "works on desktop, null in the browser".
7. `renderer/src/lib/pluginPermissions.ts` `CAP_LABELS` — an unlabelled method renders to the user as SENSITIVE, and `pluginPermissions.test.ts`'s drift guard fails.

Separate trap when you do this: several of those Go tables are gofmt-aligned maps keyed by method name. `sessions.subagentConversation` is longer than every prior key, so adding it REALIGNS the entire map. capspec.go shows 69 changed lines and authtoken.go 21 for what is one real entry each. Review those diffs with `git show -w` or you will read whitespace as substance.</observation>
<parameter name="impact">Missing #6 in particular is the silent one from the user's perspective: everything compiles, desktop works, and only the browser/remote client gets a refusal.

## Recommendation
Use this seven-item list for any new claudemon-proxied read that both the brain and the desktop provide. It is DISTINCT from the brief.* checklist ([[a-new-brief-method-needs-four-registrations-not-214ade]]), which is about path-bearing methods and the containment corpus — a method can need both lists.</recommendation>
<parameter name="confidence">high
