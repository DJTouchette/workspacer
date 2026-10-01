---
title: Native failed events and Workspacer spawn receipts carry distinct semantics
date: 2026-10-01
promoted: false
---

# Native failed events and Workspacer spawn receipts carry distinct semantics

## Observation
Canonical claudemon tool_result and command_output can fail with empty output. Native reducer previously silently dropped orphan empty failures and labelled stderr as normal output. Preserve visible failure rows. Workspacer spawn_agent receipts expose sessionId directly or via MCP structuredContent/content text JSON; these are fleet session IDs unlike provider Agent/Task agent_id values. Native child-session links must be restricted to successful completed spawn receipts.
