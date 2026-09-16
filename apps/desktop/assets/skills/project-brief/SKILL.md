---
name: project-brief
description: Keep the current project's .workspacer/brief.md useful across agent sessions by recording durable status, direction, and outcomes with Workspacer's atomic brief tools.
---

# Maintain the project brief

Use this for durable project knowledge, not a transcript of the current chat.
Read the existing `<project>/.workspacer/brief.md` before deciding what matters,
and preserve user-authored lines.

Add one concise line at a time with `brief_append({project, section, line})`:

- **Now**: active work, blockers, and the next concrete action.
- **Direction**: durable goals, priorities, and architectural decisions.
- **Recently**: completed outcomes, newest first; include evidence or caveats
  that will matter to the next agent.

Prefer `brief_check` for identifying stale `Now` entries. Do not rewrite the
whole file to add or clean up one line, and do not claim that deleting or
replacing existing lines is concurrency-safe. If stale user-authored content
needs removal, report it for the user instead of silently replacing it.

Keep facts in the most specific project brief. Do not copy fleet-wide state,
credentials, raw transcripts, or speculative conclusions into it.
