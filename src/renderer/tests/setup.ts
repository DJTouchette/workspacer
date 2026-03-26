import '@testing-library/jest-dom/vitest';

// jsdom doesn't have ResizeObserver — stub it
global.ResizeObserver = class ResizeObserver {
  observe() {}
  unobserve() {}
  disconnect() {}
} as any;

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
  createClaudeTerminal: vi.fn().mockResolvedValue('mock-claude-pty-id'),
  getClaudeSessionByPty: vi.fn().mockResolvedValue(null),
  getAllClaudeSessions: vi.fn().mockResolvedValue([]),
  onClaudeSessionUpdate: vi.fn().mockReturnValue(() => {}),
  onBeforeQuit: vi.fn().mockReturnValue(() => {}),
};

Object.defineProperty(window, 'electronAPI', {
  value: mockElectronAPI,
  writable: true,
});
