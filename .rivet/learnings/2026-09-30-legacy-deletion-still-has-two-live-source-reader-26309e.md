---
title: Legacy deletion still has two live source-reader blockers beyond packaged examples
date: 2026-09-30
confidence: high
suggested_doc: headless-desktop-services
related_paths:
  - services/hub-rs/LEGACY_DELETION_PREPARATION.md
  - tools/capability-source-check/src/reference.rs
  - apps/desktop/src/renderer/tests/backend/backendParity.test.ts
promoted: false
---

# Legacy deletion still has two live source-reader blockers beyond packaged examples

## Observation
Tracked-only inventory found572 paths under services/hub, including40 non-Go files and13 shipped example files. Web twins11 and plugin SDK1 are byte-identical to current Rust owners, but routing preferences-view.json is still imported by active renderer harness/tests. Capability-source-check reference.rs requires original Go bytes; backendParity.test.ts still reads Go main.go registrations. MCP catalog generation and six captured-reference checks are already portable and must not be misclassified as runtime Go dependencies.

## Recommendation
Follow LEGACY_DELETION_PREPARATION.md: preserve plugin tree and exact fixtures, update package/container/archive/import consumers atomically, transition explicit original-byte guards to immutable reviewed provenance without deleting binding floors, and retain public Electron/Node owners. Do not remove legacy sources until separate gates are proven.
