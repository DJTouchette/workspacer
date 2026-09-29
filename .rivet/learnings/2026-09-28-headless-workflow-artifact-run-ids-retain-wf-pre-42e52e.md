---
title: Headless workflow artifact run IDs retain wf prefix through reader lookup
date: 2026-09-28
suggested_doc: workflow-subagent-watcher
related_paths:
  - Preserve actual read path rather than deriving a filename from a guard-only expression; track separate producer parity.
promoted: false
---

# Headless workflow artifact run IDs retain wf prefix through reader lookup

## Observation
workflowWatcher.scanRuns registers each directory name verbatim, including wf_. Its readAgentTranscript/readAgentConversation resolve watch.runs.get(runId). desktopHost builds a wf_${runId} path only for an independent containment precheck; that constructed filename is not read. Rust artifact adapters must use the supplied wf_-prefixed run directory exactly and strip only the agent- file prefix. The watcher also owns separate live journal/script/final projection and telemetry; read adapters alone do not replace those producers.
