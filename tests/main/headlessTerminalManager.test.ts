import { describe, it, expect, afterEach } from 'vitest';
import {
  createHeadlessSession,
  feedData,
  getScreenContent,
  getFullBuffer,
  getLastActivity,
  resizeHeadless,
  destroyHeadlessSession,
  detectAmbientState,
} from '../../src/main/services/headlessTerminalManager';

// Helper: xterm.write() is async — we need to wait for it to flush
function waitForFlush(ms = 50): Promise<void> {
  return new Promise(resolve => setTimeout(resolve, ms));
}

describe('HeadlessTerminalManager', () => {
  const sessions: string[] = [];

  afterEach(() => {
    for (const id of sessions) {
      try { destroyHeadlessSession(id); } catch {}
    }
    sessions.length = 0;
  });

  function createSession(id: string, cols = 80, rows = 24) {
    sessions.push(id);
    return createHeadlessSession(id, cols, rows);
  }

  describe('createHeadlessSession / destroyHeadlessSession', () => {
    it('should create a headless terminal and return it', () => {
      const terminal = createSession('test-create-1');
      expect(terminal).toBeDefined();
      expect(terminal.cols).toBe(80);
      expect(terminal.rows).toBe(24);
    });

    it('should allow destroying a session', () => {
      createSession('test-destroy-1');
      destroyHeadlessSession('test-destroy-1');
      expect(getScreenContent('test-destroy-1')).toEqual([]);
    });
  });

  describe('feedData / getScreenContent', () => {
    it('should write data and read it back from screen', async () => {
      createSession('test-feed-1');
      feedData('test-feed-1', 'Hello, World!\r\n');
      await waitForFlush();

      const lines = getScreenContent('test-feed-1');
      expect(lines.some(l => l.includes('Hello, World!'))).toBe(true);
    });

    it('should handle multiple lines of data', async () => {
      createSession('test-feed-multi');
      feedData('test-feed-multi', 'Line 1\r\nLine 2\r\nLine 3\r\n');
      await waitForFlush();

      const lines = getScreenContent('test-feed-multi');
      const nonEmpty = lines.filter(l => l.trim().length > 0);
      expect(nonEmpty.length).toBeGreaterThanOrEqual(3);
      expect(nonEmpty[0]).toContain('Line 1');
      expect(nonEmpty[1]).toContain('Line 2');
      expect(nonEmpty[2]).toContain('Line 3');
    });

    it('should return empty array for unknown session', () => {
      expect(getScreenContent('nonexistent')).toEqual([]);
    });
  });

  describe('getFullBuffer', () => {
    it('should return the full scrollback buffer', async () => {
      createSession('test-buffer-1');
      feedData('test-buffer-1', 'Buffer content\r\n');
      await waitForFlush();

      const lines = getFullBuffer('test-buffer-1');
      expect(lines.some(l => l.includes('Buffer content'))).toBe(true);
    });

    it('should return empty array for unknown session', () => {
      expect(getFullBuffer('nonexistent')).toEqual([]);
    });
  });

  describe('getLastActivity', () => {
    it('should return 0 for unknown session', () => {
      expect(getLastActivity('nonexistent')).toBe(0);
    });

    it('should update on feedData', () => {
      createSession('test-activity-1');
      const before = getLastActivity('test-activity-1');
      expect(before).toBeGreaterThan(0);

      feedData('test-activity-1', 'data');
      const after = getLastActivity('test-activity-1');
      expect(after).toBeGreaterThanOrEqual(before);
    });
  });

  describe('resizeHeadless', () => {
    it('should resize the terminal', () => {
      const terminal = createSession('test-resize-1', 80, 24);
      expect(terminal.cols).toBe(80);
      expect(terminal.rows).toBe(24);

      resizeHeadless('test-resize-1', 120, 40);
      expect(terminal.cols).toBe(120);
      expect(terminal.rows).toBe(40);
    });

    it('should not throw for unknown session', () => {
      expect(() => resizeHeadless('nonexistent', 80, 24)).not.toThrow();
    });
  });

  describe('detectAmbientState', () => {
    it('should return idle for unknown session', () => {
      expect(detectAmbientState('nonexistent')).toBe('idle');
    });

    it('should detect waiting_approval when screen contains [Y/n]', async () => {
      createSession('test-detect-approval');
      feedData('test-detect-approval', 'Allow this tool? [Y/n]\r\n');
      await waitForFlush();

      const state = detectAmbientState('test-detect-approval');
      expect(state).toBe('waiting_approval');
    });

    it('should detect streaming when data was very recently received', () => {
      createSession('test-detect-stream');
      feedData('test-detect-stream', 'Some streaming output...');

      // Called immediately after feedData — lastActivity < 500ms ago
      const state = detectAmbientState('test-detect-stream');
      expect(state).toBe('streaming');
    });
  });
});
