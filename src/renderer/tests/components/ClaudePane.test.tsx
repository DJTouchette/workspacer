import { describe, it, expect, vi, beforeEach } from 'vitest';
import { render, screen, fireEvent } from '@testing-library/react';
import React from 'react';
import type { ClaudeSessionSnapshot } from '../../src/types/claudeSession';

// We need to mock heavy dependencies before importing the component

// Mock xterm — it needs a DOM canvas which jsdom doesn't have
vi.mock('@xterm/xterm', () => {
  class MockTerminal {
    cols = 80;
    rows = 24;
    open = vi.fn();
    write = vi.fn();
    dispose = vi.fn();
    focus = vi.fn();
    blur = vi.fn();
    onData = vi.fn().mockReturnValue({ dispose: vi.fn() });
    onBinary = vi.fn().mockReturnValue({ dispose: vi.fn() });
    onResize = vi.fn().mockReturnValue({ dispose: vi.fn() });
    loadAddon = vi.fn();
    attachCustomKeyEventHandler = vi.fn();
  }
  return { Terminal: MockTerminal };
});

vi.mock('@xterm/addon-fit', () => {
  class MockFitAddon {
    fit = vi.fn();
    activate = vi.fn();
    dispose = vi.fn();
  }
  return { FitAddon: MockFitAddon };
});

// Mock usePTY
const mockWrite = vi.fn();
vi.mock('../../src/hooks/usePTY', () => ({
  usePTY: vi.fn().mockReturnValue({
    sessionId: 'mock-pty-session',
    isReady: true,
    write: mockWrite,
    resize: vi.fn(),
    attachToTerminal: vi.fn(),
  }),
}));

// Mock useConfig
vi.mock('../../src/hooks/useConfig', () => ({
  useConfig: vi.fn().mockReturnValue({
    config: {
      terminal: {
        fontSize: 14,
        fontFamily: 'monospace',
        cursorBlink: true,
        scrollback: 1000,
        cursorStyle: 'block',
        shell: '',
        shells: [],
      },
      ui: { navBarHeight: 28, paneHeaderHeight: 22 },
      panes: { peek: 80, gap: 16, insertPosition: 'after' },
      browser: { hibernateAfter: 300 },
      apps: [],
      keybindings: { mode: 'default', leader: 'ctrl' },
    },
    reload: vi.fn(),
  }),
}));

// Mock useClaudeSession with controllable state
let mockSession: ClaudeSessionSnapshot | null = null;
vi.mock('../../src/hooks/useClaudeSession', () => ({
  useClaudeSession: vi.fn().mockImplementation(() => ({
    session: mockSession,
    refresh: vi.fn(),
  })),
}));

// Now import after all mocks
const { default: ClaudePane } = await import('../../src/panes/ClaudePane');

function makeSnapshot(overrides: Partial<ClaudeSessionSnapshot> = {}): ClaudeSessionSnapshot {
  return {
    sessionId: 'test-session',
    cwd: '/test/project',
    ptyId: 'mock-pty-session',
    status: 'active',
    conversation: [],
    activeToolCalls: [],
    completedToolCalls: [],
    fileChanges: [],
    pendingApproval: null,
    subagents: [],
    ambientState: 'idle',
    lastActivity: Date.now(),
    totalToolCalls: 0,
    ...overrides,
  };
}

describe('ClaudePane', () => {
  beforeEach(() => {
    mockSession = null;
    mockWrite.mockClear();
  });

  describe('initial state', () => {
    it('should show waiting message when no session exists', () => {
      mockSession = null;
      render(<ClaudePane paneId="p1" title="Claude" isActive={true} />);

      expect(screen.getByText('Claude Code session starting...')).toBeInTheDocument();
      expect(screen.getByText(/Waiting for hook events/)).toBeInTheDocument();
    });

    it('should show connected message when session exists but no conversation', () => {
      mockSession = makeSnapshot();
      render(<ClaudePane paneId="p2" title="Claude" isActive={true} />);

      expect(screen.getByText('Session connected')).toBeInTheDocument();
    });
  });

  describe('status badge', () => {
    it('should show "no session" when session is null', () => {
      mockSession = null;
      render(<ClaudePane paneId="p3" title="Claude" isActive={true} />);
      expect(screen.getByText('no session')).toBeInTheDocument();
    });

    it('should show idle status', () => {
      mockSession = makeSnapshot({ ambientState: 'idle' });
      render(<ClaudePane paneId="p4" title="Claude" isActive={true} />);
      expect(screen.getByText('Idle')).toBeInTheDocument();
    });

    it('should show thinking status', () => {
      mockSession = makeSnapshot({ ambientState: 'thinking' });
      render(<ClaudePane paneId="p5" title="Claude" isActive={true} />);
      expect(screen.getByText('Thinking...')).toBeInTheDocument();
    });

    it('should show streaming status', () => {
      mockSession = makeSnapshot({ ambientState: 'streaming' });
      render(<ClaudePane paneId="p6" title="Claude" isActive={true} />);
      expect(screen.getByText('Streaming')).toBeInTheDocument();
    });

    it('should show needs approval status', () => {
      mockSession = makeSnapshot({ ambientState: 'waiting_approval' });
      render(<ClaudePane paneId="p7" title="Claude" isActive={true} />);
      expect(screen.getByText('Needs approval')).toBeInTheDocument();
    });
  });

  describe('conversation rendering', () => {
    it('should render user and assistant messages', () => {
      mockSession = makeSnapshot({
        conversation: [
          { role: 'user', content: 'Fix the bug in app.ts', timestamp: Date.now() },
          { role: 'assistant', content: 'I found the issue. Let me fix it.', timestamp: Date.now() },
        ],
      });
      render(<ClaudePane paneId="p8" title="Claude" isActive={true} />);

      expect(screen.getByText('Fix the bug in app.ts')).toBeInTheDocument();
      expect(screen.getByText('I found the issue. Let me fix it.')).toBeInTheDocument();
      expect(screen.getByText('You')).toBeInTheDocument();
      expect(screen.getByText('Claude')).toBeInTheDocument();
    });
  });

  describe('tool calls', () => {
    it('should show active tool calls with running status', () => {
      mockSession = makeSnapshot({
        activeToolCalls: [
          { id: 'tc-1', name: 'Read', input: { file_path: '/src/app.ts' }, status: 'running', startedAt: Date.now() },
        ],
      });
      render(<ClaudePane paneId="p9" title="Claude" isActive={true} />);

      expect(screen.getByText('Running')).toBeInTheDocument();
      expect(screen.getByText('Read')).toBeInTheDocument();
    });

    it('should show completed tool calls in the side panel', () => {
      mockSession = makeSnapshot({
        completedToolCalls: [
          { id: 'tc-2', name: 'Bash', input: { command: 'npm test' }, response: 'all passed', status: 'complete', startedAt: Date.now() - 2000, completedAt: Date.now() },
        ],
      });
      render(<ClaudePane paneId="p10" title="Claude" isActive={true} />);

      expect(screen.getByText('Tool History (1)')).toBeInTheDocument();
      expect(screen.getByText('Bash')).toBeInTheDocument();
    });
  });

  describe('file changes', () => {
    it('should show file changes in the side panel', () => {
      mockSession = makeSnapshot({
        fileChanges: [
          { path: '/src/components/App.tsx', toolName: 'Edit', input: {}, timestamp: Date.now() },
          { path: '/src/utils/new.ts', toolName: 'Write', input: {}, timestamp: Date.now() },
        ],
      });
      render(<ClaudePane paneId="p11" title="Claude" isActive={true} />);

      expect(screen.getByText('Files (2)')).toBeInTheDocument();
      expect(screen.getByText('App.tsx')).toBeInTheDocument();
      expect(screen.getByText('new.ts')).toBeInTheDocument();
    });
  });

  describe('approval prompt', () => {
    it('should render approval buttons when permission is pending', () => {
      mockSession = makeSnapshot({
        pendingApproval: {
          toolName: 'Bash',
          toolInput: { command: 'rm -rf /tmp/test' },
          suggestions: ['allow_once'],
          timestamp: Date.now(),
        },
        ambientState: 'waiting_approval',
      });
      render(<ClaudePane paneId="p12" title="Claude" isActive={true} />);

      expect(screen.getByText('Permission Required: Bash')).toBeInTheDocument();
      expect(screen.getByText('Allow')).toBeInTheDocument();
      expect(screen.getByText('Deny')).toBeInTheDocument();
    });

    it('should send "y" to PTY when Allow is clicked', () => {
      mockSession = makeSnapshot({
        pendingApproval: {
          toolName: 'Bash',
          toolInput: { command: 'echo hello' },
          timestamp: Date.now(),
        },
        ambientState: 'waiting_approval',
      });
      render(<ClaudePane paneId="p13" title="Claude" isActive={true} />);

      fireEvent.click(screen.getByText('Allow'));
      expect(mockWrite).toHaveBeenCalledWith('y\n');
    });

    it('should send "n" to PTY when Deny is clicked', () => {
      mockSession = makeSnapshot({
        pendingApproval: {
          toolName: 'Bash',
          toolInput: { command: 'echo hello' },
          timestamp: Date.now(),
        },
        ambientState: 'waiting_approval',
      });
      render(<ClaudePane paneId="p14" title="Claude" isActive={true} />);

      fireEvent.click(screen.getByText('Deny'));
      expect(mockWrite).toHaveBeenCalledWith('n\n');
    });
  });

  describe('subagents', () => {
    it('should show running subagent count in toolbar', () => {
      mockSession = makeSnapshot({
        subagents: [
          { id: 'sa-1', type: 'Explore', status: 'running', startedAt: Date.now() },
          { id: 'sa-2', type: 'general-purpose', status: 'complete', startedAt: Date.now() - 5000, completedAt: Date.now() },
        ],
      });
      render(<ClaudePane paneId="p15" title="Claude" isActive={true} />);

      expect(screen.getByText('1 subagent(s)')).toBeInTheDocument();
    });

    it('should show subagents in side panel', () => {
      mockSession = makeSnapshot({
        subagents: [
          { id: 'sa-3', type: 'Explore', status: 'running', startedAt: Date.now() },
        ],
      });
      render(<ClaudePane paneId="p16" title="Claude" isActive={true} />);

      expect(screen.getByText('Subagents (1)')).toBeInTheDocument();
      expect(screen.getByText('Explore')).toBeInTheDocument();
    });
  });

  describe('view mode toggle', () => {
    it('should show GUI and Term toggle buttons', () => {
      mockSession = makeSnapshot();
      render(<ClaudePane paneId="p17" title="Claude" isActive={true} />);

      expect(screen.getByText('GUI')).toBeInTheDocument();
      expect(screen.getByText('Term')).toBeInTheDocument();
    });
  });

  describe('input area', () => {
    it('should send input to PTY on Enter', () => {
      mockSession = makeSnapshot();
      render(<ClaudePane paneId="p18" title="Claude" isActive={true} />);

      const input = screen.getByPlaceholderText('Type a message... (sent to Claude terminal)');
      fireEvent.change(input, { target: { value: 'fix the tests' } });
      fireEvent.keyDown(input, { key: 'Enter' });

      expect(mockWrite).toHaveBeenCalledWith('fix the tests\n');
    });

    it('should clear input after sending', () => {
      mockSession = makeSnapshot();
      render(<ClaudePane paneId="p19" title="Claude" isActive={true} />);

      const input = screen.getByPlaceholderText('Type a message... (sent to Claude terminal)') as HTMLInputElement;
      fireEvent.change(input, { target: { value: 'hello' } });
      fireEvent.keyDown(input, { key: 'Enter' });

      expect(input.value).toBe('');
    });

    it('should not send empty input', () => {
      mockSession = makeSnapshot();
      render(<ClaudePane paneId="p20" title="Claude" isActive={true} />);

      const input = screen.getByPlaceholderText('Type a message... (sent to Claude terminal)');
      fireEvent.keyDown(input, { key: 'Enter' });

      expect(mockWrite).not.toHaveBeenCalled();
    });
  });

  describe('tool counter', () => {
    it('should show total tool call count', () => {
      mockSession = makeSnapshot({ totalToolCalls: 42 });
      render(<ClaudePane paneId="p21" title="Claude" isActive={true} />);

      expect(screen.getByText('42 tools')).toBeInTheDocument();
    });
  });
});
