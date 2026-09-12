---
title: Headless manager replacement shares the durable transaction but needs bidirectional lifecycle acknowledgements
date: 2026-09-12
confidence: high
suggested_doc: fleet-manager
related_paths:
  - apps/desktop/src/main/headless/managerReplacement.ts
  - apps/desktop/src/main/headless/hostBridge.ts
  - services/hub/cmd/brain/replacementhost.go
  - services/hub/cmd/brain/managerrequests.go
promoted: false
---

# Headless manager replacement shares the durable transaction but needs bidirectional lifecycle acknowledgements

## Observation
ManagerReplacementService previously assumed synchronous in-process ownership transfer and synchronous receipt reads. The headless adapter awaits private Go lifecycle callbacks while reusing the same journal, checkpoint validator and inbox. Worker/task attribution uses durable transferIntent then rolls forward; normal prompts and fleet wakes must both route through holdMessage plus in-flight acknowledgement ids. No lifecycle callback is a public bus method. Browser bind actions carry the actual local viewerSessions map so kickoff waits for a successor viewer.
