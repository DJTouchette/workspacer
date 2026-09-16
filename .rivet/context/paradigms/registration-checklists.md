---
title: Registration checklists
tags: [checklist, bus-method, capspec, contracts, plugins, notifications]
related_paths:
  - "services/hub/internal/capspec/capspec.go"
  - "services/hub/internal/capspec/composition.go"
  - "services/hub/internal/authtoken/authtoken.go"
  - "services/hub/cmd/brain/handlers.go"
  - "apps/desktop/src/main/services/hubCapabilities.ts"
  - "contracts/README.md"
owner: Damien Touchette
last_reviewed: 2026-09-16
---

# Registration checklists

## New bus method

1. Register it in each intended provider and dispatch switch.
2. Declare intentional desktop/headless overlap; the router is single-owner.
3. Classify parameters in capspec and record composition reasoning for bytes
   that become code/config/argv/policy or state another guard consumes.
4. Decide whether remote view, triage or provider credentials may call it.
5. Add an MCP facade tool only when useful to agents; keep provider metadata in
   parity.
6. Update event vocabulary when it publishes or consumes a topic.
7. Add provider, authentication/provenance and parity coverage.

Do not add per-agent tool tiers, workspace-root grants, plugin root grants or
manifest-derived plugin authority. Authenticated agent tools and enabled-plugin
tools are ambient. Legacy grant-shaped fields are compatibility-only.

## Path-bearing method

Choose one contract:

- **Ambient host path:** canonicalize an absolute caller path once and open the
  exact returned spelling. The OS/provider owns access.
- **Selected-object path:** after selecting a repository, library, replay
  worktree or plugin asset root, canonicalize the derived path and require it to
  remain inside that object.

Record the parameter in capspec so ambiguous case-variant JSON keys cannot make
the checked value differ from the used value. Do not call that classification a
directory grant.

## Contract fixture

Add a README row, two cross-language loaders when applicable, a complete
`vocabulary.blocks` registry, fixture-count updates/exemptions and non-zero case
floors in each loader. Platform skips must remain visible.

## Plugin surface

Enabled plugins are trusted local extensions. Manifest capabilities, paths and
child tiers are advisory/legacy. Preserve plugin identity, provider ownership,
result correlation, webview origin/CSP/navigation and asset-root containment,
settings secrecy, exact pending-spawn ownership and lifecycle cleanup.

## Notifications

Update the shared shape, serializers, renderer store, delivery transports and
tests. Avoid routine success receipts for actions already visible in chat;
reserve notifications for failures, questions, blockers and meaningful
background events.
