import { Terminal as HeadlessTerminal } from '@xterm/headless';
import { SerializeAddon } from '@xterm/addon-serialize';

interface HeadlessSession {
  terminal: HeadlessTerminal;
  serialize: SerializeAddon;
  lastActivity: number;
  /** Line index marking where we last read — used for buffer diffing */
  lastReadLine: number;
}

const sessions = new Map<string, HeadlessSession>();

export function createHeadlessSession(sessionId: string, cols: number, rows: number): HeadlessTerminal {
  const terminal = new HeadlessTerminal({ cols, rows, scrollback: 1000, allowProposedApi: true });
  const serialize = new SerializeAddon();
  terminal.loadAddon(serialize);

  sessions.set(sessionId, {
    terminal,
    serialize,
    lastActivity: Date.now(),
    lastReadLine: 0,
  });

  return terminal;
}

export function feedData(sessionId: string, data: string): void {
  const session = sessions.get(sessionId);
  if (!session) return;

  session.terminal.write(data);
  session.lastActivity = Date.now();
}

export function getScreenContent(sessionId: string): string[] {
  const session = sessions.get(sessionId);
  if (!session) return [];

  const buffer = session.terminal.buffer.active;
  const lines: string[] = [];

  for (let i = 0; i < session.terminal.rows; i++) {
    const line = buffer.getLine(i + buffer.viewportY);
    if (line) {
      lines.push(line.translateToString(true));
    }
  }

  return lines;
}

export function getFullBuffer(sessionId: string): string[] {
  const session = sessions.get(sessionId);
  if (!session) return [];

  const buffer = session.terminal.buffer.active;
  const lines: string[] = [];

  for (let i = 0; i < buffer.length; i++) {
    const line = buffer.getLine(i);
    if (line) {
      lines.push(line.translateToString(true));
    }
  }

  return lines;
}

/**
 * Get new buffer content since the last snapshot and advance the read cursor.
 * Returns the new lines as a single string (empty lines trimmed from edges).
 */
export function getNewBufferContent(sessionId: string): string {
  const session = sessions.get(sessionId);
  if (!session) return '';

  const buffer = session.terminal.buffer.active;
  const lines: string[] = [];

  for (let i = session.lastReadLine; i < buffer.length; i++) {
    const line = buffer.getLine(i);
    if (line) {
      lines.push(line.translateToString(true));
    }
  }

  session.lastReadLine = buffer.length;

  // Trim empty lines from both ends
  while (lines.length > 0 && lines[0].trim() === '') lines.shift();
  while (lines.length > 0 && lines[lines.length - 1].trim() === '') lines.pop();

  return lines.join('\n');
}

/** Mark the current buffer position so getNewBufferContent starts from here */
export function markBufferPosition(sessionId: string): void {
  const session = sessions.get(sessionId);
  if (!session) return;
  session.lastReadLine = session.terminal.buffer.active.length;
}

export function getLastActivity(sessionId: string): number {
  const session = sessions.get(sessionId);
  return session?.lastActivity ?? 0;
}

export function resizeHeadless(sessionId: string, cols: number, rows: number): void {
  const session = sessions.get(sessionId);
  if (!session) return;
  session.terminal.resize(cols, rows);
}

export function destroyHeadlessSession(sessionId: string): void {
  const session = sessions.get(sessionId);
  if (!session) return;
  session.terminal.dispose();
  sessions.delete(sessionId);
}

export type SessionAmbientState = 'idle' | 'thinking' | 'streaming' | 'waiting_input' | 'waiting_approval';

export function detectAmbientState(sessionId: string): SessionAmbientState {
  const session = sessions.get(sessionId);
  if (!session) return 'idle';

  const lines = getScreenContent(sessionId);
  const screenText = lines.join('\n');
  const timeSinceActivity = Date.now() - session.lastActivity;

  // Check for permission/approval prompt
  if (/\[Y\/n\]/i.test(screenText) || /Allow|Deny|Skip/.test(screenText)) {
    return 'waiting_approval';
  }

  // Check for user input prompt (idle)
  if (timeSinceActivity > 2000 && /^>/m.test(screenText)) {
    return 'idle';
  }

  // Recently received data — streaming
  if (timeSinceActivity < 500) {
    return 'streaming';
  }

  // Brief pause — thinking
  if (timeSinceActivity < 5000) {
    return 'thinking';
  }

  return 'waiting_input';
}
