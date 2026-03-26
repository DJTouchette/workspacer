import { BrowserWindow } from 'electron';
import * as os from 'os';
import * as crypto from 'crypto';

// Prebuilt binaries — no Visual Studio Build Tools needed on Windows
import * as pty from '@homebridge/node-pty-prebuilt-multiarch';

import { createHeadlessSession, feedData, resizeHeadless, destroyHeadlessSession, detectAmbientState } from './headlessTerminalManager';
import { claudeSessionStore } from './claudeSessionStore';

interface TerminalSession {
  pty: pty.IPty;
  closed: boolean;
  cwd: string;
  isClaudeSession: boolean;
}

function detectDefaultShell(): string {
  if (process.platform === 'win32') {
    // Try pwsh first, fall back to powershell
    try {
      require('child_process').execSync('where pwsh.exe', { stdio: 'ignore' });
      return 'pwsh.exe';
    } catch {
      return 'powershell.exe';
    }
  }
  return process.env.SHELL || '/bin/sh';
}

class TerminalService {
  private sessions: Map<string, TerminalSession> = new Map();
  private mainWindow: BrowserWindow | null = null;
  private ambientPollers = new Map<string, ReturnType<typeof setInterval>>();

  setMainWindow(win: BrowserWindow): void {
    this.mainWindow = win;
  }

  /** Create a terminal that runs Claude Code CLI with headless mirroring + hook integration */
  createClaudeTerminal(cwd?: string): string {
    const id = this.createTerminalInternal('claude', cwd, true);

    // Create a headless terminal mirror
    createHeadlessSession(id, 80, 24);

    // Register this PTY as pending — the SessionStart hook will bind it by cwd
    const session = this.sessions.get(id);
    const resolvedCwd = session?.cwd ?? cwd ?? '';
    claudeSessionStore.registerPendingPty(id, resolvedCwd);

    // Start ambient state polling (routes by ptyId, works once binding is established)
    const poller = setInterval(() => {
      const state = detectAmbientState(id);
      claudeSessionStore.updateAmbientStateByPty(id, state);
    }, 300);
    this.ambientPollers.set(id, poller);

    return id;
  }

  createTerminal(shell: string, cwd?: string): string {
    // Intercept sentinel value from ClaudePane
    if (shell === '__claude__') {
      return this.createClaudeTerminal(cwd);
    }
    return this.createTerminalInternal(shell, cwd, false);
  }

  private createTerminalInternal(shell: string, cwd: string | undefined, isClaudeSession: boolean): string {
    if (!shell) {
      shell = detectDefaultShell();
    }

    const id = crypto.randomUUID();

    const env = { ...process.env, TERM: 'xterm-256color' } as Record<string, string>;

    const homedir = process.env.HOME || os.homedir();
    let resolvedCwd = cwd || homedir;

    // Validate CWD exists — fall back to home if it doesn't (e.g. Linux path on Windows)
    try {
      if (!require('fs').existsSync(resolvedCwd)) resolvedCwd = homedir;
    } catch {
      resolvedCwd = homedir;
    }

    // For Claude sessions, launch the claude CLI
    let spawnShell: string;
    let spawnArgs: string[];
    if (isClaudeSession) {
      if (process.platform === 'win32') {
        // Use the user's PowerShell so its profile loads (adds nvm/node to PATH)
        spawnShell = detectDefaultShell();
        spawnArgs = ['-NoLogo', '-Command', 'claude'];
      } else {
        spawnShell = 'claude';
        spawnArgs = [];
      }
    } else {
      spawnShell = shell;
      spawnArgs = [];
    }

    const ptyProcess = pty.spawn(spawnShell, spawnArgs, {
      name: 'xterm-256color',
      cols: 80,
      rows: 24,
      cwd: resolvedCwd,
      env,
    });

    const session: TerminalSession = {
      pty: ptyProcess,
      closed: false,
      cwd: resolvedCwd,
      isClaudeSession,
    };

    this.sessions.set(id, session);

    // Forward output to renderer as base64, and tee to headless terminal for Claude sessions
    ptyProcess.onData((data: string) => {
      if (session.closed) return;
      const encoded = Buffer.from(data, 'binary').toString('base64');
      this.mainWindow?.webContents.send('terminal:output', id, encoded);

      // Tee to headless terminal for Claude sessions
      if (session.isClaudeSession) {
        feedData(id, data);
      }
    });

    // Handle exit
    ptyProcess.onExit(() => {
      if (session.closed) return;
      session.closed = true;
      this.sessions.delete(id);
      this.mainWindow?.webContents.send('terminal:exit', id);
    });

    return id;
  }

  writeTerminal(id: string, base64Data: string): void {
    const session = this.sessions.get(id);
    if (!session || session.closed) return;

    const decoded = Buffer.from(base64Data, 'base64').toString('binary');
    session.pty.write(decoded);
  }

  resizeTerminal(id: string, cols: number, rows: number): void {
    const session = this.sessions.get(id);
    if (!session || session.closed) return;

    session.pty.resize(cols, rows);

    if (session.isClaudeSession) {
      resizeHeadless(id, cols, rows);
    }
  }

  closeTerminal(id: string): void {
    const session = this.sessions.get(id);
    if (!session) return;

    session.closed = true;
    this.sessions.delete(id);
    session.pty.kill();

    // Cleanup headless terminal + poller + store binding for Claude sessions
    if (session.isClaudeSession) {
      destroyHeadlessSession(id);
      claudeSessionStore.unregisterPty(id);
      const poller = this.ambientPollers.get(id);
      if (poller) {
        clearInterval(poller);
        this.ambientPollers.delete(id);
      }
    }
  }

  getTerminalPid(id: string): number | undefined {
    const session = this.sessions.get(id);
    if (!session || session.closed) return undefined;
    return session.pty.pid;
  }

  getTerminalCwd(id: string): string | undefined {
    const session = this.sessions.get(id);
    if (!session || session.closed) return undefined;
    return session.cwd;
  }

  closeAll(): void {
    for (const [id, session] of this.sessions) {
      session.closed = true;
      session.pty.kill();
    }
    this.sessions.clear();
  }
}

export const terminalService = new TerminalService();
