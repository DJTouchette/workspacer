import { describe, it, expect, beforeEach, vi } from 'vitest';

// We need to mock Electron's BrowserWindow before importing the store
vi.mock('electron', () => ({
  BrowserWindow: vi.fn(),
}));

// Dynamic import so mocks are in place
const { claudeSessionStore } = await import('../../src/main/services/claudeSessionStore');

// Helper to reset the store between tests (it's a singleton)
// We access internal state via the public API
function makeSessionStartEvent(sessionId: string, cwd: string) {
  return {
    hook_event_name: 'SessionStart',
    session_id: sessionId,
    cwd,
    source: 'startup',
  };
}

function makeUserPromptEvent(sessionId: string, cwd: string, prompt: string) {
  return {
    hook_event_name: 'UserPromptSubmit',
    session_id: sessionId,
    cwd,
    prompt,
  };
}

function makePreToolUseEvent(sessionId: string, cwd: string, toolName: string, toolInput: any, toolUseId: string) {
  return {
    hook_event_name: 'PreToolUse',
    session_id: sessionId,
    cwd,
    tool_name: toolName,
    tool_input: toolInput,
    tool_use_id: toolUseId,
  };
}

function makePostToolUseEvent(sessionId: string, cwd: string, toolUseId: string, response: any) {
  return {
    hook_event_name: 'PostToolUse',
    session_id: sessionId,
    cwd,
    tool_use_id: toolUseId,
    tool_response: response,
  };
}

function makePostToolUseFailureEvent(sessionId: string, cwd: string, toolUseId: string) {
  return {
    hook_event_name: 'PostToolUseFailure',
    session_id: sessionId,
    cwd,
    tool_use_id: toolUseId,
  };
}

function makePermissionRequestEvent(sessionId: string, cwd: string, toolName: string, toolInput: any) {
  return {
    hook_event_name: 'PermissionRequest',
    session_id: sessionId,
    cwd,
    tool_name: toolName,
    tool_input: toolInput,
    permission_suggestions: ['allow_once'],
  };
}

function makeStopEvent(sessionId: string, cwd: string) {
  return {
    hook_event_name: 'Stop',
    session_id: sessionId,
    cwd,
  };
}

function makeSessionEndEvent(sessionId: string, cwd: string) {
  return {
    hook_event_name: 'SessionEnd',
    session_id: sessionId,
    cwd,
  };
}

function makeSubagentStartEvent(sessionId: string, cwd: string, agentId: string, agentType: string) {
  return {
    hook_event_name: 'SubagentStart',
    session_id: sessionId,
    cwd,
    agent_id: agentId,
    agent_type: agentType,
  };
}

function makeSubagentStopEvent(sessionId: string, cwd: string, agentId: string) {
  return {
    hook_event_name: 'SubagentStop',
    session_id: sessionId,
    cwd,
    agent_id: agentId,
  };
}

function makeNotificationEvent(sessionId: string, cwd: string, message: string) {
  return {
    hook_event_name: 'Notification',
    session_id: sessionId,
    cwd,
    message,
  };
}

// Since claudeSessionStore is a singleton, we need a fresh instance per test.
// We'll use a factory approach with dynamic imports.
async function createFreshStore() {
  // Re-import to get a fresh module — vitest caches, so we reset via the mock
  const mod = await import('../../src/main/services/claudeSessionStore');
  // The singleton is already created, but we can test flows sequentially
  // by using unique session IDs and cwds per test
  return mod.claudeSessionStore;
}

describe('ClaudeSessionStore', () => {
  const store = claudeSessionStore;

  describe('PTY registration and binding', () => {
    it('should register a pending PTY and bind on SessionStart', () => {
      const ptyId = 'pty-bind-1';
      const cwd = '/test/bind/project1';
      const sessionId = 'session-bind-1';

      store.registerPendingPty(ptyId, cwd);

      // Before SessionStart, no snapshot
      expect(store.getSnapshotByPty(ptyId)).toBeNull();

      // Fire SessionStart — should claim the pending PTY
      store.handleHookEvent(makeSessionStartEvent(sessionId, cwd));

      const snapshot = store.getSnapshotByPty(ptyId);
      expect(snapshot).not.toBeNull();
      expect(snapshot!.sessionId).toBe(sessionId);
      expect(snapshot!.ptyId).toBe(ptyId);
      expect(snapshot!.cwd).toBe(cwd);
      expect(snapshot!.status).toBe('active');
    });

    it('should bind multiple PTYs with different cwds independently', () => {
      const pty1 = 'pty-multi-1';
      const pty2 = 'pty-multi-2';
      const cwd1 = '/test/multi/project-a';
      const cwd2 = '/test/multi/project-b';

      store.registerPendingPty(pty1, cwd1);
      store.registerPendingPty(pty2, cwd2);

      store.handleHookEvent(makeSessionStartEvent('session-multi-1', cwd1));
      store.handleHookEvent(makeSessionStartEvent('session-multi-2', cwd2));

      const snap1 = store.getSnapshotByPty(pty1);
      const snap2 = store.getSnapshotByPty(pty2);

      expect(snap1!.sessionId).toBe('session-multi-1');
      expect(snap1!.ptyId).toBe(pty1);
      expect(snap2!.sessionId).toBe('session-multi-2');
      expect(snap2!.ptyId).toBe(pty2);
    });

    it('should use FIFO ordering for same-cwd PTYs', () => {
      const cwd = '/test/fifo/same-cwd';
      const pty1 = 'pty-fifo-1';
      const pty2 = 'pty-fifo-2';

      store.registerPendingPty(pty1, cwd);
      store.registerPendingPty(pty2, cwd);

      // First SessionStart claims first PTY
      store.handleHookEvent(makeSessionStartEvent('session-fifo-1', cwd));
      expect(store.getSnapshotByPty(pty1)!.sessionId).toBe('session-fifo-1');
      expect(store.getSnapshotByPty(pty2)).toBeNull();

      // Second SessionStart claims second PTY
      store.handleHookEvent(makeSessionStartEvent('session-fifo-2', cwd));
      expect(store.getSnapshotByPty(pty2)!.sessionId).toBe('session-fifo-2');
    });

    it('should unregister a PTY and clean up bindings', () => {
      const ptyId = 'pty-unreg-1';
      const cwd = '/test/unreg/project';
      const sessionId = 'session-unreg-1';

      store.registerPendingPty(ptyId, cwd);
      store.handleHookEvent(makeSessionStartEvent(sessionId, cwd));

      expect(store.getSnapshotByPty(ptyId)).not.toBeNull();

      store.unregisterPty(ptyId);
      expect(store.getSnapshotByPty(ptyId)).toBeNull();
    });

    it('should unregister an unbound PTY from the pending queue', () => {
      const cwd = '/test/unreg-pending/project';
      const pty1 = 'pty-unreg-pending-1';
      const pty2 = 'pty-unreg-pending-2';

      store.registerPendingPty(pty1, cwd);
      store.registerPendingPty(pty2, cwd);

      // Remove the first one before binding
      store.unregisterPty(pty1);

      // Now SessionStart should claim pty2, not pty1
      store.handleHookEvent(makeSessionStartEvent('session-unreg-pending', cwd));
      expect(store.getSnapshotByPty(pty1)).toBeNull();
      expect(store.getSnapshotByPty(pty2)!.sessionId).toBe('session-unreg-pending');
    });
  });

  describe('hook event processing', () => {
    const ptyId = 'pty-events-1';
    const cwd = '/test/events/project';
    const sessionId = 'session-events-1';

    it('should set up a session and process a full conversation flow', () => {
      store.registerPendingPty(ptyId, cwd);
      store.handleHookEvent(makeSessionStartEvent(sessionId, cwd));

      // User prompt
      store.handleHookEvent(makeUserPromptEvent(sessionId, cwd, 'Hello Claude'));
      let snap = store.getSnapshotByPty(ptyId)!;
      expect(snap.conversation).toHaveLength(1);
      expect(snap.conversation[0].role).toBe('user');
      expect(snap.conversation[0].content).toBe('Hello Claude');
      expect(snap.ambientState).toBe('thinking');

      // PreToolUse
      store.handleHookEvent(makePreToolUseEvent(sessionId, cwd, 'Read', { file_path: '/foo/bar.ts' }, 'tu-1'));
      snap = store.getSnapshotByPty(ptyId)!;
      expect(snap.activeToolCalls).toHaveLength(1);
      expect(snap.activeToolCalls[0].name).toBe('Read');
      expect(snap.activeToolCalls[0].status).toBe('running');

      // PostToolUse
      store.handleHookEvent(makePostToolUseEvent(sessionId, cwd, 'tu-1', 'file contents here'));
      snap = store.getSnapshotByPty(ptyId)!;
      expect(snap.activeToolCalls).toHaveLength(0);
      expect(snap.completedToolCalls).toHaveLength(1);
      expect(snap.completedToolCalls[0].status).toBe('complete');
      expect(snap.completedToolCalls[0].response).toBe('file contents here');
      expect(snap.totalToolCalls).toBe(1);

      // Stop
      store.handleHookEvent(makeStopEvent(sessionId, cwd));
      snap = store.getSnapshotByPty(ptyId)!;
      expect(snap.ambientState).toBe('idle');
    });

    it('should track file changes for Edit/Write/MultiEdit tools', () => {
      const pty = 'pty-filechange-1';
      const cwd2 = '/test/filechange/project';
      const sid = 'session-filechange-1';

      store.registerPendingPty(pty, cwd2);
      store.handleHookEvent(makeSessionStartEvent(sid, cwd2));

      store.handleHookEvent(makePreToolUseEvent(sid, cwd2, 'Edit', { file_path: '/src/app.ts', old_string: 'a', new_string: 'b' }, 'tu-fc-1'));
      store.handleHookEvent(makePreToolUseEvent(sid, cwd2, 'Write', { file_path: '/src/new.ts', content: 'hello' }, 'tu-fc-2'));
      store.handleHookEvent(makePreToolUseEvent(sid, cwd2, 'Bash', { command: 'ls' }, 'tu-fc-3'));

      const snap = store.getSnapshotByPty(pty)!;
      // Only Edit and Write create file changes, not Bash
      expect(snap.fileChanges).toHaveLength(2);
      expect(snap.fileChanges[0].path).toBe('/src/app.ts');
      expect(snap.fileChanges[0].toolName).toBe('Edit');
      expect(snap.fileChanges[1].path).toBe('/src/new.ts');
      expect(snap.fileChanges[1].toolName).toBe('Write');
    });

    it('should handle tool failures', () => {
      const pty = 'pty-fail-1';
      const cwd3 = '/test/fail/project';
      const sid = 'session-fail-1';

      store.registerPendingPty(pty, cwd3);
      store.handleHookEvent(makeSessionStartEvent(sid, cwd3));

      store.handleHookEvent(makePreToolUseEvent(sid, cwd3, 'Bash', { command: 'exit 1' }, 'tu-fail-1'));
      store.handleHookEvent(makePostToolUseFailureEvent(sid, cwd3, 'tu-fail-1'));

      const snap = store.getSnapshotByPty(pty)!;
      expect(snap.activeToolCalls).toHaveLength(0);
      expect(snap.completedToolCalls).toHaveLength(1);
      expect(snap.completedToolCalls[0].status).toBe('failed');
    });

    it('should handle permission requests and clear on Stop', () => {
      const pty = 'pty-perm-1';
      const cwd4 = '/test/perm/project';
      const sid = 'session-perm-1';

      store.registerPendingPty(pty, cwd4);
      store.handleHookEvent(makeSessionStartEvent(sid, cwd4));

      store.handleHookEvent(makePermissionRequestEvent(sid, cwd4, 'Bash', { command: 'rm -rf /' }));

      let snap = store.getSnapshotByPty(pty)!;
      expect(snap.pendingApproval).not.toBeNull();
      expect(snap.pendingApproval!.toolName).toBe('Bash');
      expect(snap.ambientState).toBe('waiting_approval');

      store.handleHookEvent(makeStopEvent(sid, cwd4));
      snap = store.getSnapshotByPty(pty)!;
      expect(snap.pendingApproval).toBeNull();
      expect(snap.ambientState).toBe('idle');
    });

    it('should track subagents', () => {
      const pty = 'pty-sub-1';
      const cwd5 = '/test/sub/project';
      const sid = 'session-sub-1';

      store.registerPendingPty(pty, cwd5);
      store.handleHookEvent(makeSessionStartEvent(sid, cwd5));

      store.handleHookEvent(makeSubagentStartEvent(sid, cwd5, 'agent-1', 'Explore'));
      let snap = store.getSnapshotByPty(pty)!;
      expect(snap.subagents).toHaveLength(1);
      expect(snap.subagents[0].status).toBe('running');
      expect(snap.subagents[0].type).toBe('Explore');

      store.handleHookEvent(makeSubagentStopEvent(sid, cwd5, 'agent-1'));
      snap = store.getSnapshotByPty(pty)!;
      expect(snap.subagents[0].status).toBe('complete');
    });

    it('should handle notifications as assistant messages', () => {
      const pty = 'pty-notif-1';
      const cwd6 = '/test/notif/project';
      const sid = 'session-notif-1';

      store.registerPendingPty(pty, cwd6);
      store.handleHookEvent(makeSessionStartEvent(sid, cwd6));

      store.handleHookEvent(makeNotificationEvent(sid, cwd6, 'Task completed!'));
      const snap = store.getSnapshotByPty(pty)!;
      expect(snap.conversation).toHaveLength(1);
      expect(snap.conversation[0].role).toBe('assistant');
      expect(snap.conversation[0].content).toBe('Task completed!');
    });

    it('should mark session as ended on SessionEnd', () => {
      const pty = 'pty-end-1';
      const cwd7 = '/test/end/project';
      const sid = 'session-end-1';

      store.registerPendingPty(pty, cwd7);
      store.handleHookEvent(makeSessionStartEvent(sid, cwd7));

      store.handleHookEvent(makeSessionEndEvent(sid, cwd7));
      const snap = store.getSnapshotByPty(pty)!;
      expect(snap.status).toBe('ended');
      expect(snap.ambientState).toBe('idle');
    });
  });

  describe('getAllSnapshots', () => {
    it('should return all active sessions', () => {
      const all = store.getAllSnapshots();
      // We've created many sessions in previous tests
      expect(all.length).toBeGreaterThan(0);
      expect(all.every(s => s.sessionId)).toBe(true);
    });
  });

  describe('ambient state', () => {
    it('should update ambient state by PTY id', () => {
      const pty = 'pty-ambient-1';
      const cwd = '/test/ambient/project';
      const sid = 'session-ambient-1';

      store.registerPendingPty(pty, cwd);
      store.handleHookEvent(makeSessionStartEvent(sid, cwd));

      store.updateAmbientStateByPty(pty, 'streaming');
      const snap = store.getSnapshotByPty(pty)!;
      expect(snap.ambientState).toBe('streaming');
    });

    it('should not override ambient state when approval is pending', () => {
      const pty = 'pty-ambient-perm-1';
      const cwd = '/test/ambient-perm/project';
      const sid = 'session-ambient-perm-1';

      store.registerPendingPty(pty, cwd);
      store.handleHookEvent(makeSessionStartEvent(sid, cwd));

      // Set pending approval
      store.handleHookEvent(makePermissionRequestEvent(sid, cwd, 'Bash', { command: 'test' }));
      expect(store.getSnapshotByPty(pty)!.ambientState).toBe('waiting_approval');

      // Try to override with streaming — should be ignored
      store.updateAmbientStateByPty(pty, 'streaming');
      expect(store.getSnapshotByPty(pty)!.ambientState).toBe('waiting_approval');
    });
  });
});
