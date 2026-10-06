---
title: Native GPUI UI tests: demo fixture, page scroll, witness gaps
date: 2026-10-05
confidence: high
suggested_doc: native-embedded-backend
related_paths:
  - apps/native/src/ui/**
promoted: false
---

# Native GPUI UI tests: demo fixture, page scroll, witness gaps

## Observation
apps/native ui-tests fixtures construct Workspace with demo=true, so load_models/create/continue paths no-op until a test sets this.demo=false. Feature pages (page_view) scroll: at the usual 900px test viewport a footer button below the fold has debug_bounds but simulate_click at its center does nothing, so tests silently see no Command. witness select ignores new untracked files, and witness run emitted one 'npx jest' line holding .rs targets for a native-only diff.

## Impact
Tests that click a page footer fail mysteriously, or pass vacuously when they assert that nothing happened.

## Recommendation
Set this.demo=false; give long-page tests a taller viewport (see ui/handoff.rs tests, 1600px); run cargo test for native targets yourself instead of the witness run line.
