---
name: checkpoint
description: Capture durable knowledge from this Fleet Manager session and file each finding to the most specific home (project brief, fleet brief, or a note for the user), then prune stale brief lines. Only useful inside a Workspacer Fleet Manager session (requires the mcp__workspacer__* tools).
---

# /checkpoint — durable memory and brief maintenance

File knowledge from this conversation that would otherwise be lost. Do not
spawn workers or invent facts. Read only briefs you need to change; preserve
user-authored lines. Add via atomic `brief_append`, never a whole-file rewrite;
inspect before editing/removing existing lines.

Every brief is `<project>/.workspacer/brief.md`:
- **Now**: live work/escalations; remove landed or abandoned items.
- **Direction**: durable goals, priorities and sequencing.
- **Recently**: newest-first dated outcomes, `- YYYY-MM-DD  <what happened>`.
- **User** (fleet only): the user's standing preferences.

Route to the most specific home:
1. Repo knowledge belongs in that project's brief or its existing context
   system. For Rivet findings, use `rivet.learn` when available or leave the
   owning worker a concise followup to record them. Without repo memory tools,
   flag permanent CLAUDE.md/AGENTS.md additions for the user.
2. Fleet-wide priorities, dispatch outcomes and escalations belong in your own
   fleet brief. Do not copy project briefs into it.
3. Stated user preferences go in the fleet brief's User section.
4. A task-specific next action goes to its worker via `send_message` when
   authorized, or into that project's Now section.

Prune only touched briefs. `brief_check({project})` identifies potentially
stale Now references; decide which are actually finished. Use
`brief_archive({project,section:"Recently",keep:20})` for old outcomes,
adjusting the count for activity. It preserves entries byte-for-byte in
`brief.archive.md` under the brief lock. Do not delete history or hand-write
archive headings. Archive other sections only when the oldest entries are
finished/superseded; never archive active goals merely to meet a count.

`brief_append` refuses lines over 4000 characters; split them. Its
entriesInSection/bytesInSection report size without another read. For worker
outcomes pass the validated result and sessionId; write only the significance
in line. Preserve evidence caveats. Finish with a short report of what was
filed, archived and left for the user.
