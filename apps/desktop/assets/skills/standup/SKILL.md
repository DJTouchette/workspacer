---
name: standup
description: Fleet status at a glance for the Workspacer Fleet Manager — a tight four-section digest of what your dispatched workers are doing. Only useful inside a Workspacer Fleet Manager session (requires the mcp__workspacer__* tools).
---

# /standup — fleet status

Give one concise digest, then stop. No spawning, polling or brief edits.
Use the progress/results already in this conversation and your fleet brief.
Call `list_agents` once for live state; read only relevant project briefs
whose recent outcomes or priorities are missing. Use `project_status` for
requested git state, with `dirs` to scope it. Fetch `get_conversation` with
`lastMessage:true` only for a missing final report, or `textOnly:true` and
`sinceSeq` for a specific dialogue gap. Do not repeat a worker's own report
request or generate a model summary when its progress/result already answers.

Use these sections, omitting empty ones:
- **In flight**: session:<id>, project, task, state and last meaningful progress.
  For NEEDS A DECISION, say what answer you will provide or need from the user.
- **Landed recently**: a few newest project outcomes; flag independent review
  still owed under the chosen policy or explicit user instructions.
- **Waiting on you**: unresolved user decisions and approvals.
- **Next up**: brief-based recommendations only; do not dispatch them here.

Use exact session:<id> references for clickable agent links.
