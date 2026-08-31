---
title: Routing provider enabled flag is parsed but not enforced
date: 2026-08-30
author: codex-review
confidence: high
suggested_doc: config
related_paths:
  - services/hub/internal/routing/matrix.go
  - services/hub/internal/routing/policy.go
  - services/hub/internal/routing/policy_test.go
  - services/hub/internal/routing/routing.default.yaml
promoted: false
---

# Routing provider enabled flag is parsed but not enforced

## Observation
The routing matrix accepts providers.<id>.enabled:false and matrix_test proves Provider.IsEnabled() reflects it, but routing.Select never checks Provider.IsEnabled before computing capacity or returning an eligible assignment. Select only checks assignment-level enabled:false after assignmentFor resolves a capability, so a disabled provider can still be selected if a profile maps the capability to it.

## Impact
Operators can believe a provider has been taken out of service via the documented provider-level enabled:false control while routing.select continues recommending it. This is a policy-control bypass, not a stale-window leak.

## Recommendation
Make provider-level disabled status a selection refusal (or remove the field/documentation if it is intentionally not a dispatch control) and add a policy-level test that providers.codex.enabled:false makes codex ineligible even when the assignment itself is enabled.
