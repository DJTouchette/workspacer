---
title: Deployment parity must check owned launch readiness rather than the retired callback
date: 2026-09-29
suggested_doc: fly-node-deploy
related_paths:
  - deploy/fly/combined/verify-web-capabilities.py
  - deploy/fly/combined/web-capability-contract.cjs
  - deploy/fly/combined/web-capability-contract.test.cjs
promoted: false
---

# Deployment parity must check owned launch readiness rather than the retired callback

## Observation
The current combined image verifier still required plugins.prepareLaunch, a Go provider callback absent from Rust owned launch wiring. Rust Core::launch_ready requires spawn coordinator, wake service, ready embedded engine, configured facade readiness and platform support. The verifier now captures authenticated health.launchReady and requires it to be exactly true alongside public agents.spawn and plugins.manifests registration. Shared contract fixtures execute the actual Python-assembled remote JS with mocked transport and prove missing/false readiness fails even when legacy callback or runtime status strings are present.

## Impact
A retired callback inventory check rejects a correct owned backend; method registration or generic ready text alone can also falsely accept an unavailable launch service.

## Recommendation
Keep authenticated launchReady plus public service checks in read-only deployment validation; do not expose opaque LaunchPermit as an RPC or claim readiness proves a provider login or completed launch.
