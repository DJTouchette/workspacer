import { describe, it, expect } from 'vitest';
import fs from 'fs';
import path from 'path';

import { applyHookEvent } from './hookEventRouter';
import type { ClaudeSessionState } from '../claudeSessionStore';

// The desktop half of contracts/permission-request-hook-cases.json. The Rust
// half is `permission_request_contract_cases` in claudemon's session/state.rs.
//
// One hook frame, two readers picking DIFFERENT fields off it: claudemon reads
// tool_name + summary|message (it is the only thing that can put a PTY session
// into SessionMode::Approval), the desktop reads tool_name + tool_input +
// permission_suggestions to render the card the user clicks. A CLI rename of
// either set would otherwise break exactly one side, quietly — and the failure
// mode is a permission prompt with no answerable record behind it.

type Case = {
  name: string;
  payload: Record<string, unknown>;
  desktop: {
    ambientState: string;
    toolName: string;
    toolInput: unknown;
    suggestions: unknown;
  };
};

const fixture = JSON.parse(
  fs.readFileSync(
    path.join(__dirname, '../../../../../../contracts/permission-request-hook-cases.json'),
    'utf8',
  ),
) as { hookEventName: string; registerable: boolean; cases: Case[] };

function mkSession(): ClaudeSessionState {
  return {
    sessionId: 's1',
    // PTY: the transport whose approvals exist ONLY because this hook is
    // registered. Cards on the stream transport come from the daemon's
    // `pending` slot instead (see hookEventRouter's `hooksOwnPending`).
    transport: 'pty',
    conversation: [],
    activeToolCalls: [],
    completedToolCalls: [],
    fileChanges: [],
    subagents: [],
    workflows: [],
    pendingApproval: null,
    pendingQuestions: null,
    ambientState: 'streaming',
    totalToolCalls: 0,
  } as unknown as ClaudeSessionState;
}

describe('contracts/permission-request-hook-cases.json — desktop reader', () => {
  it('has cases', () => {
    expect(fixture.cases.length).toBeGreaterThan(0);
    expect(fixture.hookEventName).toBe('PermissionRequest');
    expect(fixture.registerable).toBe(true);
  });

  for (const c of fixture.cases) {
    it(c.name, () => {
      const s = mkSession();
      applyHookEvent(s, { hook_event_name: fixture.hookEventName, ...c.payload });

      expect(s.ambientState).toBe(c.desktop.ambientState);
      expect(s.pendingApproval).not.toBeNull();
      expect(s.pendingApproval?.toolName).toBe(c.desktop.toolName);
      expect(s.pendingApproval?.toolInput).toEqual(c.desktop.toolInput);
      expect(s.pendingApproval?.suggestions ?? null).toEqual(c.desktop.suggestions);
    });
  }
});
