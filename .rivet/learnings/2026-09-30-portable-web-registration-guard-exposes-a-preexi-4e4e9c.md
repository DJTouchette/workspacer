---
title: Portable web registration guard exposes a preexisting unprovided publish call
date: 2026-09-30
confidence: high
suggested_doc: renderer-backend-seam
related_paths:
  - apps/desktop/src/renderer/tests/backend/backendParity.test.ts
  - apps/desktop/src/renderer/src/backend/webBackend.ts
promoted: false
---

# Portable web registration guard exposes a preexisting unprovided publish call

## Observation
Prep backendParity test formerly read Go main.go plus a manual HUB_CORE allowlist. Replacing it with backend-capabilities hub43 and explicit architectural retirements retains measured web75/desktop88 floors and missing/retired/population mutation refusals. It also exposes __publish as unprovided: webBackend.hubPublish serializes a normal RPC and swallows errors; uiEvents, plugin hotkeys, Overview spawn actions and failover notifications call it. No tracked Rust/Go handler or transport translation exists. This is live preexisting behavior, not evidence of a Rust-only regression or an intended wire pseudo-method.

## Recommendation
Keep the known missing set explicit instead of falsely certifying all web calls. Independently reproduce and fix publication in prep under its own scope; the actual Rust headless completeness test remains the live Backend registration counterpart but its dotted-method regex does not cover __publish. Do not alter the frozen release candidate.

## Separate prep-only repair
The production web backend/client wire test reproduced op:call/method:__publish
instead of op:publish before repair. HubBusClient now has connected-only
best-effort publish with no durable queue or success-ack claim; webBackend uses
it. Server authority is untouched: Rust publish checks scope/topic and reports
uncorrelated refusal. Positive wire, disconnected/reconnect no replay, send-race
drop and refusal/RPC-correlation controls pass together with related suites43.
The portable guard now expects an empty missing set (74 RPC calls; publication is
no longer an RPC). appPublish.test.ts is discovered by the existing app project
and will execute the actual Rust broker subscriber/view-token refusal barrier.
No prebuilt fixture was available locally, so that real-server test has not run;
listing it and passing wire tests are not an end-to-end receipt. Frozen main was
not changed. Logs: /dev/shm/workspacer-prep-publish-before.log and
/dev/shm/workspacer-prep-publish-final.log.

## Actual Rust/Chromium receipt
The authorized single hub_contract_fixture example build succeeded with its
required test-support feature (no standalone binary/all-tests build). Browser
mode uses ordinary Rust Hub/auth/publish implementations; fixture injection hooks
are for MCP/paired modes, not this run. Existing cached Chromium libraries were
used via process-local LD_LIBRARY_PATH; no browser download or system install.

The final appPublish Chromium test passed against that actual Rust fixture in
/dev/shm/workspacer-prep-publish-rust-browser-final.log. It proves the legacy
serialized __publish RPC receives exactly 'no provider for __publish', then the
fixed production browser client delivers the exact event to a real subscriber.
A view-token hello and actual uncorrelated authorization refusal are observed;
a same-client RPC and ordered observer marker prove no unauthorized delivery.

Initial scoped-page setup incorrectly expected view authority to load /app,
which intentionally requires operator access. The corrected fixture first
asserts401, authorizes only the HTML document fetch with the owner header, and
leaves the browser /bus connection on its view query token. No server policy was
relaxed. Local-only navigation avoids unrelated third-party font loads; action
deadlines were not inflated. All source and frontend build outputs stay in prep;
main source remains frozen. This verifies publication, not every web feature.
