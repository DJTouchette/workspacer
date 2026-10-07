---
title: A new claudemon-proxied hub-rs bus method touches ten registries (hub-rs era)
date: 2026-10-07
confidence: high
related_paths:
  - services/hub-rs/src/services/sessions.rs
  - services/hub-rs/assets/hub-vocabulary.json
  - apps/desktop/src/main/services/rustParameterBindings.test.ts
promoted: false
---

# A new claudemon-proxied hub-rs bus method touches ten registries (hub-rs era)

## Observation
sessions.taskOutput/taskStop (2026-10-07) needed: sessions.rs method list + dispatch arm
(+ params.rs validation); hub-vocabulary.json methods/scopes/claudemon twin routes;
brain-capabilities.json `full`; contracts/backend-capabilities.json; contracts/
http-route-registry.json (twin routes); provider_relay/methods.rs FULL (remote nodes);
quiescence.rs passive list (reads only); tests/fixtures/authorization-compositions.json
(actors/inert/acknowledgedActors.triage); desktop capability-parameter-policy.json
(methodDecisions/inertMethods) + tests/support/compositionDecisions.ts (claim,
parameterDecision, actors); and the rustParameterBindings.test.ts row ratchet
(+1 per dispatch arm). New claudemon routes also need `make claudemon-routes` and the
API_ROUTE_FLOOR bump. The desktop suite's ipc/webviewGuard confinement tests fail when
TMPDIR points under $HOME; run desktop vitest with the default TMPDIR.
