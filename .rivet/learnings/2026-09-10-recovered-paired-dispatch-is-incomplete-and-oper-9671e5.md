---
title: Recovered paired dispatch is incomplete and operator event trust is not origin provenance
date: 2026-09-10
suggested_doc: hub-federation
related_paths:
  - services/hub/internal/bus/remotedispatch.go
  - apps/desktop/src/main/services/remoteDispatchRegistry.ts
promoted: false
---

# Recovered paired dispatch is incomplete and operator event trust is not origin provenance

## Observation
Recovered ancestry 091583e7 -> 3fc1ef0c plus exact desktop edits is banked in 23e64e83. Source inspection shows remoteDispatchRegistry.start has no caller; dispatch discovery requires peers.json dispatch rather than existing pairing; local workflow/task/request capture is not connected to remote callbacks. Router forwarding failures are incorrectly called definite spawn failures, registry acknowledges before best-effort wake, and persistence errors are swallowed. The event publish path promotes paired operator tokens to trusted: without a dispatch-specific guard they can forge admission events and result hub stamps. Added guards and unexecuted hosted regression fixtures; no end-to-end proof exists.

## Impact
Do not deploy or present this recovery as a complete paired execution target. A visible remote card or Go provenance stamp does not prove local manager wake, task lineage, admission durability, or isolated remote ship work.

## Recommendation
Complete paired host credential reuse and explicit local target routing, durable acknowledged admission and return delivery, local workflow/request integration, remote worktree/provider preflight, UI/backend parity, and hosted real MCP-to-brain-to-local-wake tests before both-endpoint rollout.
