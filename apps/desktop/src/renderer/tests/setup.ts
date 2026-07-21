import '@testing-library/jest-dom/vitest';

// jsdom has no layout, so virtualized lists (@tanstack/react-virtual) would see
// a 0-size viewport and render nothing. Give elements a fake non-zero box and
// make ResizeObserver fire once so the virtualizer renders its window.
const FAKE_RECT = {
  width: 800,
  height: 600,
  top: 0,
  left: 0,
  right: 800,
  bottom: 600,
  x: 0,
  y: 0,
  toJSON() {},
} as DOMRect;

global.ResizeObserver = class ResizeObserver {
  private cb: ResizeObserverCallback;
  constructor(cb: ResizeObserverCallback) {
    this.cb = cb;
  }
  observe(el: Element) {
    this.cb(
      [
        {
          target: el,
          contentRect: FAKE_RECT,
          borderBoxSize: [{ inlineSize: 800, blockSize: 600 }],
          contentBoxSize: [{ inlineSize: 800, blockSize: 600 }],
        } as unknown as ResizeObserverEntry,
      ],
      this,
    );
  }
  unobserve() {}
  disconnect() {}
} as any;

Element.prototype.getBoundingClientRect = function () {
  return FAKE_RECT;
};

// react-virtual reads the scroll element's offset/client size for its viewport.
for (const prop of ['offsetWidth', 'clientWidth'] as const) {
  Object.defineProperty(HTMLElement.prototype, prop, {
    configurable: true,
    get() {
      return 800;
    },
  });
}
for (const prop of ['offsetHeight', 'clientHeight'] as const) {
  Object.defineProperty(HTMLElement.prototype, prop, {
    configurable: true,
    get() {
      return 600;
    },
  });
}

// jsdom doesn't implement scrollIntoView
Element.prototype.scrollIntoView = vi.fn();

// Mock window.electronAPI for all renderer tests
const mockElectronAPI = {
  createTerminal: vi.fn().mockResolvedValue('mock-pty-id'),
  writeTerminal: vi.fn().mockResolvedValue(undefined),
  resizeTerminal: vi.fn().mockResolvedValue(undefined),
  closeTerminal: vi.fn().mockResolvedValue(undefined),
  onTerminalOutput: vi.fn().mockReturnValue(() => {}),
  onTerminalExit: vi.fn().mockReturnValue(() => {}),
  getConfig: vi.fn().mockResolvedValue({}),
  reloadConfig: vi.fn().mockResolvedValue({}),
  getConfigPath: vi.fn().mockResolvedValue('/mock/config/path'),
  saveConfig: vi.fn().mockResolvedValue({}),
  listSessions: vi.fn().mockResolvedValue([]),
  loadSession: vi.fn().mockResolvedValue(null),
  saveSession: vi.fn().mockResolvedValue('mock-session'),
  deleteSession: vi.fn().mockResolvedValue(undefined),
  spawnClaude: vi.fn().mockResolvedValue('mock-claude-session-id'),
  claudeMessage: vi.fn().mockResolvedValue({ ok: true }),
  claudeApprove: vi.fn().mockResolvedValue(undefined),
  claudeAnswer: vi.fn().mockResolvedValue(undefined),
  claudeResize: vi.fn().mockResolvedValue(undefined),
  claudeSignal: vi.fn().mockResolvedValue(undefined),
  claudeClose: vi.fn().mockResolvedValue(undefined),
  claudeGate: vi.fn().mockResolvedValue(undefined),
  claudeWrite: vi.fn(),
  onClaudeOutput: vi.fn().mockReturnValue(() => {}),
  getClaudeSession: vi.fn().mockResolvedValue(null),
  getAllClaudeSessions: vi.fn().mockResolvedValue([]),
  listLiveClaudeSessionIds: vi.fn().mockResolvedValue(null),
  listRecentAgentSessions: vi.fn().mockResolvedValue([]),
  onClaudeSessionUpdate: vi.fn().mockReturnValue(() => {}),
  onBeforeQuit: vi.fn().mockReturnValue(() => {}),
};

Object.defineProperty(window, 'electronAPI', {
  value: mockElectronAPI,
  writable: true,
});
