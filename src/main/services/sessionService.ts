import * as fs from 'fs';
import * as path from 'path';
import * as os from 'os';
import * as yaml from 'js-yaml';
import { terminalService } from './terminalService';

interface SessionPaneData {
  id: string;
  type: string;
  title: string;
  width: number;
  widthOverride?: number;
  shell?: string;
  cwd?: string;
  url?: string;
}

interface SessionData {
  name: string;
  timestamp: string;
  activePaneId: string;
  panes: SessionPaneData[];
}

interface SessionListEntry {
  name: string;
  filename: string;
  timestamp: string;
  paneCount: number;
}

function getConfigDir(): string {
  if (process.platform === 'win32') {
    const appData = process.env.APPDATA;
    if (appData) return path.join(appData, 'workspacer');
    return path.join(os.homedir(), 'AppData', 'Roaming', 'workspacer');
  }
  const xdg = process.env.XDG_CONFIG_HOME;
  if (xdg) return path.join(xdg, 'workspacer');
  return path.join(os.homedir(), '.config', 'workspacer');
}

function getSessionsDir(): string {
  return path.join(getConfigDir(), 'sessions');
}

function sanitizeFilename(name: string): string {
  return name.toLowerCase().replace(/[^a-z0-9_-]/g, '-').replace(/-+/g, '-').substring(0, 64);
}

function getTerminalCwd(ptySessionId: string): string | undefined {
  const pid = terminalService.getTerminalPid(ptySessionId);
  if (!pid) return terminalService.getTerminalCwd(ptySessionId);

  try {
    if (process.platform === 'linux') {
      return fs.readlinkSync(`/proc/${pid}/cwd`);
    }
    if (process.platform === 'darwin') {
      const { execSync } = require('child_process');
      const output = execSync(`lsof -p ${pid} 2>/dev/null | grep cwd`, { encoding: 'utf-8' });
      const match = output.match(/cwd\s+\S+\s+\S+\s+\S+\s+\S+\s+\S+\s+\S+\s+\S+\s+(.*)/);
      if (match?.[1]?.trim()) return match[1].trim();
    }
    // Windows: no /proc or lsof — rely on stored CWD (initial + OSC 7 updates)
  } catch {
    // CWD detection failed
  }

  // Fall back to stored CWD (initial launch dir or last OSC 7 update)
  return terminalService.getTerminalCwd(ptySessionId);
}

class SessionService {
  private ensureDir(): void {
    fs.mkdirSync(getSessionsDir(), { recursive: true });
  }

  listSessions(): SessionListEntry[] {
    this.ensureDir();
    const dir = getSessionsDir();

    try {
      const files = fs.readdirSync(dir).filter((f) => f.endsWith('.yaml'));
      const entries: SessionListEntry[] = [];

      for (const file of files) {
        try {
          const data = fs.readFileSync(path.join(dir, file), 'utf-8');
          const session = yaml.load(data) as SessionData;
          entries.push({
            name: session.name || file.replace('.yaml', ''),
            filename: file,
            timestamp: session.timestamp || '',
            paneCount: session.panes?.length || 0,
          });
        } catch {
          // Skip malformed session files
        }
      }

      // Sort by timestamp descending (most recent first)
      entries.sort((a, b) => b.timestamp.localeCompare(a.timestamp));
      return entries;
    } catch {
      return [];
    }
  }

  loadSession(filename: string): SessionData | null {
    try {
      const filePath = path.join(getSessionsDir(), filename);
      const data = fs.readFileSync(filePath, 'utf-8');
      return yaml.load(data) as SessionData;
    } catch {
      return null;
    }
  }

  saveSession(data: SessionData): string {
    this.ensureDir();
    const filename = sanitizeFilename(data.name) + '.yaml';
    const filePath = path.join(getSessionsDir(), filename);
    const yamlStr = yaml.dump(data, { lineWidth: -1 });
    fs.writeFileSync(filePath, yamlStr, 'utf-8');
    return filename;
  }

  deleteSession(filename: string): void {
    try {
      const filePath = path.join(getSessionsDir(), filename);
      fs.unlinkSync(filePath);
    } catch {
      // Ignore if file doesn't exist
    }
  }

  enrichPanesWithCwd(panes: SessionPaneData[], ptyMapping: Record<string, string>): SessionPaneData[] {
    return panes
      .filter((p) => p.type !== 'settings')
      .map((pane) => {
        if (pane.type === 'terminal' && ptyMapping[pane.id]) {
          const cwd = getTerminalCwd(ptyMapping[pane.id]);
          return { ...pane, cwd: cwd || pane.cwd };
        }
        return pane;
      });
  }
}

export const sessionService = new SessionService();
