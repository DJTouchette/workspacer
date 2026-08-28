// What each harness can PLAN and DELEGATE — the flags that decide whether the
// Inspector shows a Plan / Flows / Agents tab at all.
//
// Every claim below is pinned to something verified about the harness itself,
// not to what our adapter happens to implement today. The comments name the
// evidence, because the cost of getting one of these wrong is either a tab that
// can never fill (a promise the harness can't keep) or a signal we hide from a
// provider that really does emit it.

import { describe, it, expect } from 'vitest';
import { capsFor, PROVIDER_CAPS } from './providerCaps';
import type { AgentProvider } from '../types/pane';

const ALL: AgentProvider[] = ['claude', 'codex', 'copilot', 'opencode', 'pi'];

describe('delegation caps', () => {
  it('claude is the reference implementation and keeps all three', () => {
    // Regression guard, and the point of the whole flag set: gating must never
    // take anything away from the harness the panes were built for.
    expect(capsFor('claude').delegation).toEqual({
      plan: true,
      subagents: true,
      subagentDrillIn: true,
      workflows: true,
    });
  });

  it('the stream transport changes nothing, because none of it rides the wire', () => {
    // Plan / subagent rows / workflow runs all come from Claude Code's hooks and
    // its on-disk artifacts, which the headless adapter writes exactly like the
    // TUI does. A transport swap that quietly dropped them would be a bug.
    expect(capsFor('claude', 'stream').delegation).toEqual(capsFor('claude', 'pty').delegation);
  });

  it('workflows are claude-only', () => {
    // `session.workflows` is read off Claude Code's own run artifacts
    // (`workflows/wf_<runId>.json`, `subagents/workflows/…`) by workflowWatcher.
    // No other harness writes them, so a Flows tab anywhere else is dead.
    for (const p of ALL) {
      expect([p, capsFor(p).delegation.workflows]).toEqual([p, p === 'claude']);
    }
  });

  it('every managed harness with a todo tool reports a plan', () => {
    // codex   — native `update_plan`.
    // copilot — a `todos` table in the session's own SQLite db.
    // opencode— the `todowrite` tool (`tool/todowrite`, v1.18.25).
    for (const p of ['codex', 'copilot', 'opencode'] as AgentProvider[]) {
      expect([p, capsFor(p).delegation.plan]).toEqual([p, true]);
    }
  });

  it('pi has no plan and no subagents, because it has no tools for either', () => {
    // pi's whole built-in tool set is bash, edit, find, grep, ls, powershell,
    // read, write (`dist/core/tools/`, v0.84.3). Nothing there can produce a
    // todo list or dispatch a child, so all three tabs would sit empty forever.
    expect(capsFor('pi').delegation).toEqual({
      plan: false,
      subagents: false,
      subagentDrillIn: false,
      workflows: false,
    });
  });

  it('drill-in is never claimed without a transcript to open', () => {
    // The narrower capability: reporting that a child RAN is not the same as
    // being able to show what it did.
    //   claude — `subagents/agent-<id>.jsonl` on disk.
    //   codex  — the child thread's rollout file under $CODEX_HOME.
    //   copilot— nothing persisted; the adapter drops the child's frames.
    //   opencode—the child is a real session, but we have no read path to it.
    expect(capsFor('copilot').delegation).toMatchObject({
      subagents: true,
      subagentDrillIn: false,
    });
    expect(capsFor('opencode').delegation).toMatchObject({
      subagents: true,
      subagentDrillIn: false,
    });
    // And the invariant that keeps the pair honest in both directions.
    for (const p of ALL) {
      const d = capsFor(p).delegation;
      if (d.subagentDrillIn) expect([p, d.subagents]).toEqual([p, true]);
    }
  });

  it('every provider declares all four flags', () => {
    // A missing `delegation` would read as `undefined.plan` at the call site and
    // throw inside the Inspector, so an added provider must fill this in.
    for (const p of ALL) {
      expect([p, Object.keys(PROVIDER_CAPS[p].delegation).sort()]).toEqual([
        p,
        ['plan', 'subagentDrillIn', 'subagents', 'workflows'],
      ]);
    }
  });
});
