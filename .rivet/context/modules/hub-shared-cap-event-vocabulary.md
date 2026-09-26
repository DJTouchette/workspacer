---
title: Hub Shared Capability and Event Vocabulary
tags: [hub, go, capabilities, event-bus, compatibility]
related_paths:
  - "services/hub/internal/capspec/*.go"
  - "services/hub/internal/event/*.go"
owner: Damien Touchette
last_reviewed: 2026-09-26
---

# Hub Shared Capability and Event Vocabulary

`capspec` and `event` remain dependency-free shared vocabularies between the bus
and plugin loader. Legacy caller capability/path scopes and emit/consume declarations are
retained for compatibility and drift tests, not as enabled-plugin authority.
Do not treat every field in those structures as inert: `EventGrants.Provides`
still carries the enforced provider-registration namespace. Authenticated enabled plugins receive
ambient methods, ordinary events, and host filesystem paths.

Active boundaries are identity/provenance: manual remote token tiers,
host-owned routes/topics, revocation, provider namespace ownership, and
single-owner method registration. `event.Matches`/`MatchesAny` still define
topic-pattern syntax. Capability composition records still classify parameters
and pin canonicalization/object-containment mechanisms, but must not describe
legacy workspace roots as runtime grants.

When adding a capability or topic, keep the desktop and brain registrations,
MCP schema, labels/help, provider namespace, and relevant composition records
in sync. `plugin.eventGrantsFor` revalidates `Provides` even for manifests that
arrived through a path bypassing normal validation; bad namespaces are dropped
and logged. Preserve that boundary when simplifying legacy grant structures.
