---
title: GPUI primary buttons must define hover styles once
date: 2026-09-27
suggested_doc: theme-system
related_paths:
  - apps/native/src/ui.rs
promoted: false
---

# GPUI primary buttons must define hover styles once

## Observation
Running the native ui-tests with local X11 libraries exposed a hover style already set panic: Stateful<Div>::hover asserts if a helper and its primary-button override both set hover. Resolve primary/secondary hover color inside a single hover call. The full 7-test GPUI suite catches this runtime failure, while cargo check does not.
