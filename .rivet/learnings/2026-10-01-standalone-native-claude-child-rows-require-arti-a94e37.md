---
title: Standalone native Claude child rows require artifact discovery beyond hooks
date: 2026-10-01
suggested_doc: workflow-subagent-watcher
related_paths:
  - services/claudemon/src/session/claude_subagents.rs
  - services/claudemon/src/session/conversation.rs
promoted: false
---

# Standalone native Claude child rows require artifact discovery beyond hooks

## Observation
Native embedded launches do not opt into global Claude hook settings, and stream driver argv does not install session-local hook forwarding. The daemon tailer can discover exact session JSONL under spawn-authorized project roots and bounded per-parent subagents artifacts, exposing child rows without Electron workflowWatcher. Hook events remain optional metadata enrichment and must preserve managed pending/mode/background ownership. Child replay derives canonical filenames from known parent membership and ignores hook agent_transcript_path.

## Impact
Hook-only subagent support would pass synthetic hooks while showing no child cards in an ordinary standalone native stream launch.

## Recommendation
Keep exact-session discovery and bounded canonical child replay covered alongside managed-hook ownership tests and raw sidechain block normalization.
