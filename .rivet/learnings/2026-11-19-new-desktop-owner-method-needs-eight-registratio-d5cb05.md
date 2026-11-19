---
title: New desktop.* owner method needs eight registration sites; diagnose reuses the brief one-shot
date: 2026-11-19
confidence: high
suggested_doc: registration-checklists
related_paths:
  - services/hub-rs/src/services/provider_utilities/**
  - contracts/desktop-service-methods.json
  - apps/native/src/diagnose.rs
  - apps/native/src/ui/diagnose.rs
promoted: false
---

# New desktop.* owner method needs eight registration sites; diagnose reuses the brief one-shot

## Observation
desktop.diagnose (hub-rs provider_utilities/diagnose.rs) reuses the title/brief one-shot: Service::diagnose calls generator.brief (Limits::BRIEF, tool-less, shares the 2 brief slots). Target comes from agents.diagnose {provider, models[harness]}: blank provider = the error's harness hint, else agents.managerProvider, else claude; blank Claude model = "sonnet". Prompt text is credential-masked with live_controls::summary::redact (made pub(crate)). Marker lines WORKSPACER_BUG: yes|no and ISSUE_TITLE: are stripped from the reply. A new desktop.* owner method must be listed in contracts/desktop-service-methods.json (hub auth.rs reads it for owner gating), contracts/backend-capabilities.json, hub assets brain-capabilities.json + hub-vocabulary.json, hub tests/fixtures/authorization-compositions.json, desktop tests/fixtures/capability-parameter-policy.json, tests/support/compositionDecisions.ts (object entry + sorted list), plus a `case` in apps/desktop/src/main/headless/desktopHost.ts (desktopServices.test.ts asserts one exists), then `node scripts/gen-desktop-services.mjs`.

## Impact
Missing any site fails owner gating or the Electron contract tests. Native UI tests run the fixture in demo mode, so controls gated on !self.demo are invisible unless the test sets this.demo = false.

## Recommendation
Copy the desktop.diagnose / desktop.toolsStatus registration set when adding a desktop.* method; set demo=false in UI tests for hub-backed controls.
