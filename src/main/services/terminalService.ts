import { BrowserWindow, MessageChannelMain } from 'electron';
import * as os from 'os';
import * as path from 'path';
import * as fs from 'fs';
import * as crypto from 'crypto';

// Prebuilt binaries — no Visual Studio Build Tools needed on Windows
import * as pty from '@homebridge/node-pty-prebuilt-multiarch';

/** Find the best way to launch Claude CLI on Windows */
function findClaudeSpawn(): { shell: string; args: string[] } | null {
  if (process.platform !== 'win32') return null;

  // Find node.exe + cli.js from nvm — spawn node directly (no cmd.exe wrapper)
  const nvmDir = path.join(os.homedir(), 'AppData', 'Local', 'nvm');
  try {
    const versions = fs.readdirSync(nvmDir)
      .filter(d => d.startsWith('v'))
      .sort((a, b) => b.localeCompare(a, undefined, { numeric: true }));
    for (const v of versions) {
      const vDir = path.join(nvmDir, v);
      for (const pkg of ['@anthropic-ai/claude-code', 'claude-code']) {
        const script = path.join(vDir, 'node_modules', pkg, 'cli.js');
        if (fs.existsSync(script)) {
          const node = path.join(vDir, 'node.exe');
          if (fs.existsSync(node)) return { shell: node, args: [script] };
        }
      }
    }
  } catch {}

  return null;
}

let _cachedClaudeSpawn: { shell: string; args: string[] } | null | undefined;
function getClaudeSpawn(): { shell: string; args: string[] } | null {
  if (_cachedClaudeSpawn === undefined) {
    _cachedClaudeSpawn = findClaudeSpawn();
    if (_cachedClaudeSpawn) console.log(`[TerminalService] found claude: ${_cachedClaudeSpawn.shell} ${_cachedClaudeSpawn.args.join(' ')}`);
  }
  return _cachedClaudeSpawn;
}

import { createHeadlessSession, feedData, resizeHeadless, destroyHeadlessSession, detectAmbientState } from './headlessTerminalManager';
import { claudeSessionStore } from './claudeSessionStore';

interface TerminalSession {
  pty: pty.IPty;
  closed: boolean;
  cwd: string;
  isClaudeSession: boolean;
  port?: Electron.MessagePortMain;
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
  private ambientPollers = new Map<string, ReturnType<typeof setTimeout>>();

  setMainWindow(win: BrowserWindow): void {
    this.mainWindow = win;
  }

  /** Create a terminal that runs Claude Code CLI with headless mirroring + hook integration */
  createClaudeTerminal(cwd?: string, cols?: number, rows?: number): string {
    // Spawn PTY first — if this throws, don't set up headless/poller
    const id = this.createTerminalInternal('claude', cwd, true, cols, rows);

    try {
      // Create a headless terminal mirror
      createHeadlessSession(id, cols || 80, rows || 24);

      // Register this PTY as pending — the SessionStart hook will bind it by cwd
      const session = this.sessions.get(id);
      const resolvedCwd = session?.cwd ?? cwd ?? '';
      claudeSessionStore.registerPendingPty(id, resolvedCwd);

      // Start adaptive ambient state polling — polls faster during activity,
      // backs off when idle to reduce CPU usage.
      this.startAdaptivePoller(id);
    } catch (err) {
      console.error('[TerminalService] Claude session setup failed:', err);
    }

    return id;
  }

  createTerminal(shell: string, cwd?: string, cols?: number, rows?: number): string {
    // Intercept sentinel value from ClaudePane
    if (shell === '__claude__') {
      return this.createClaudeTerminal(cwd, cols, rows);
    }
    return this.createTerminalInternal(shell, cwd, false, cols, rows);
  }

  private createTerminalInternal(shell: string, cwd: string | undefined, isClaudeSession: boolean, cols?: number, rows?: number): string {
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
        // Prefer native binary, fall back to node+cli.js, then cmd.exe
        const claude = getClaudeSpawn();
        if (claude) {
          spawnShell = claude.shell;
          spawnArgs = claude.args;
        } else {
          spawnShell = 'cmd.exe';
          spawnArgs = ['/c', 'claude'];
        }
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
      cols: cols || 80,
      rows: rows || 24,
      cwd: resolvedCwd,
      env,
      useConptyDll: true,
    });

    const session: TerminalSession = {
      pty: ptyProcess,
      closed: false,
      cwd: resolvedCwd,
      isClaudeSession,
    };

    this.sessions.set(id, session);

    // Create MessagePort channel for direct I/O (bypasses IPC dispatch + base64)
    if (this.mainWindow && !this.mainWindow.isDestroyed()) {
      const { port1, port2 } = new MessageChannelMain();
      session.port = port1;

      // Receive writes from renderer via port
      port1.on('message', (event) => {
        if (session.closed) return;
        session.pty.write(event.data);
      });
      port1.start();

      // Send port2 to renderer
      this.mainWindow.webContents.postMessage('terminal:port', { id }, [port2]);
    }

    // Forward PTY output to renderer via MessagePort (no base64 encoding)
    ptyProcess.onData((data: string) => {
      if (session.closed) return;
      if (session.port) {
        session.port.postMessage(data);
      }

      // Tee to headless terminal for Claude sessions
      if (session.isClaudeSession) {
        feedData(id, data);
      }
    });

    // Handle exit (stays on regular IPC — low frequency)
    ptyProcess.onExit(() => {
      if (session.closed) return;
      session.closed = true;
      this.sessions.delete(id);
      try { this.mainWindow?.webContents.send('terminal:exit', id); } catch {};
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
    try { session.pty.kill(); } catch {};
    if (session.port) { try { session.port.close(); } catch {} }

    // Cleanup headless terminal + poller + store binding for Claude sessions
    if (session.isClaudeSession) {
      destroyHeadlessSession(id);
      claudeSessionStore.unregisterPty(id);
      const poller = this.ambientPollers.get(id);
      if (poller) {
        clearTimeout(poller);
        this.ambientPollers.delete(id);
      }
    }
  }

  /** Adaptive poller: 300ms when active/streaming, 2000ms when idle */
  private startAdaptivePoller(id: string): void {
    let currentInterval = 300;
    const FAST_INTERVAL = 300;
    const SLOW_INTERVAL = 2000;

    const poll = () => {
      const session = this.sessions.get(id);
      if (!session || session.closed) {
        const timer = this.ambientPollers.get(id);
        if (timer) clearTimeout(timer);
        this.ambientPollers.delete(id);
        return;
      }

      const state = detectAmbientState(id);
      claudeSessionStore.updateAmbientStateByPty(id, state);

      // Pick interval based on current state
      const isActive = state === 'streaming' || state === 'thinking';
      const desiredInterval = isActive ? FAST_INTERVAL : SLOW_INTERVAL;
      currentInterval = desiredInterval;

      const timer = setTimeout(poll, currentInterval);
      this.ambientPollers.set(id, timer);
    };

    // Kick off with fast interval
    const timer = setTimeout(poll, FAST_INTERVAL);
    this.ambientPollers.set(id, timer);
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
      try { session.pty.kill(); } catch {};
    }
    this.sessions.clear();
  }
}

export const terminalService = new TerminalService();
