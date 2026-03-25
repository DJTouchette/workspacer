import { BrowserWindow } from 'electron';
import * as os from 'os';
import * as crypto from 'crypto';

// Prebuilt binaries — no Visual Studio Build Tools needed on Windows
import * as pty from '@homebridge/node-pty-prebuilt-multiarch';

interface TerminalSession {
  pty: pty.IPty;
  closed: boolean;
  cwd: string;
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

  setMainWindow(win: BrowserWindow): void {
    this.mainWindow = win;
  }

  createTerminal(shell: string, cwd?: string): string {
    if (!shell) {
      shell = detectDefaultShell();
    }

    const id = crypto.randomUUID();

    const env = { ...process.env, TERM: 'xterm-256color' } as Record<string, string>;

    // Set up OSC 7 CWD reporting via environment for bash/zsh
    // This makes the shell emit its working directory on every prompt
    const osc7Cmd = 'printf "\\e]7;file://%s%s\\a" "$(hostname)" "$PWD"';
    if (env.PROMPT_COMMAND) {
      env.PROMPT_COMMAND = `${osc7Cmd};${env.PROMPT_COMMAND}`;
    } else {
      env.PROMPT_COMMAND = osc7Cmd;
    }
    // For zsh: set precmd via ZDOTDIR or just rely on PROMPT_COMMAND (zsh 5.9+ supports it)
    // Alternatively, set the precmd function via zshrc eval — but env var is cleaner

    const resolvedCwd = cwd || process.env.HOME || os.homedir();

    const ptyProcess = pty.spawn(shell, [], {
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
    };

    this.sessions.set(id, session);

    // Forward output to renderer as base64
    ptyProcess.onData((data: string) => {
      if (session.closed) return;

      const encoded = Buffer.from(data, 'binary').toString('base64');
      this.mainWindow?.webContents.send('terminal:output', id, encoded);

      // Track CWD from OSC 7 off the hot path — only parse if prefix is present
      if (data.includes('\x1b]7;')) {
        setImmediate(() => {
          const match = data.match(/\x1b\]7;file:\/\/[^/]*([^\x07\x1b]*)/);
          if (match) {
            try {
              const decoded = decodeURIComponent(match[1]);
              if (decoded) session.cwd = decoded;
            } catch {
              // Malformed URI
            }
          }
        });
      }
    });

    // Handle exit
    ptyProcess.onExit(() => {
      if (session.closed) return;
      session.closed = true;
      this.sessions.delete(id);
      this.mainWindow?.webContents.send('terminal:exit', id);
    });

    // Inject OSC 7 shell integration for PowerShell
    // Must use $([char]27) and $([char]7) — PowerShell doesn't understand \e
    const shellLower = shell.toLowerCase();
    if (shellLower.includes('powershell') || shellLower.includes('pwsh')) {
      setTimeout(() => {
        if (session.closed) return;
        // Single line: define prompt that emits OSC 7 then calls original prompt
        const cmd = `function prompt{[Console]::Write("$([char]27)]7;file://$($env:COMPUTERNAME)/$($executionContext.SessionState.Path.CurrentLocation.ProviderPath)$([char]7)");return "PS $($executionContext.SessionState.Path.CurrentLocation)> "}`;
        ptyProcess.write(cmd + '\r');
        // Clear so user doesn't see the injected command
        setTimeout(() => {
          if (!session.closed) ptyProcess.write('cls\r');
        }, 300);
      }, 800);
    }

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
  }

  closeTerminal(id: string): void {
    const session = this.sessions.get(id);
    if (!session) return;

    session.closed = true;
    this.sessions.delete(id);
    session.pty.kill();
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
