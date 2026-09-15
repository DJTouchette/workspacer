---
title: Jira-to-intent imports are atomic and replayable
date: 2026-09-14
promoted: false
---

# Jira-to-intent imports are atomic and replayable

## Observation
IntentWorkspaceStore schema v10 adds intent_jira_imports. importJiraIntent resolves a versioned enabled Jira connection by projectRoot, reads through the existing SourceSyncStore account lease, then rechecks the connection and atomically saves a Draft, pinned Jira source/external state and operation response. Reusing operationId with the same arguments returns that response; changed arguments fail. Jira summary and description seed title/outcome; constraints and success criteria are left for user review and Jira status never activates or completes work. jiraIntegrations lists enabled project Jira connections without requiring a throwaway intent. Source imports share the five-second account cooldown with background refresh, so a busy error should be retried with the same operation ID. UI From Jira and MCP list_jira_connections/create_intent_from_jira both use these host operations.
