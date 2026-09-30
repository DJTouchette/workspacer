---
title: Bootstrap disabled web assets require explicit empty selection
date: 2026-09-30
suggested_doc: workspacer-serve-cli
related_paths:
  - services/hub-rs/src/cli/serve.rs
  - services/hub-rs/src/cli/launcher_paths.rs
  - apps/desktop/src/main/services/hubDaemon.ts
promoted: false
---

# Bootstrap disabled web assets require explicit empty selection

## Observation
The Rust launcher previously filtered an explicit empty --webapp-dir to None before environment/bundle fallback, so a disable request could re-enable a bundled renderer. Electron also omitted that flag when sharing was disabled, which accidentally requested Rust autodiscovery. The resolver now distinguishes explicit empty from omission; Electron passes an empty argument when sharing is off or its web build is absent. Three actual Electron argv cases plus the three owning hubDaemon suites passed39 tests and main typecheck. Rust CLI22 and pure launcher_paths helper3 subsequently passed, including actual HTTP routes and explicit-empty background-service choices. Clap default PathBufValueParser rejects empty strings, so other historical disable flags must be traced individually before changing their parsers.

## Impact
Source-level helper parity does not prove launcher composition honors an operator disabled setting.

## Recommendation
Preserve explicit absence, explicit empty and omitted/default as separate states; verify actual process routes or captured spawn argv in addition to pure resolver tests.
