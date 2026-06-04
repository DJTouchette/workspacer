import * as fs from 'fs';
import * as path from 'path';
import * as os from 'os';
import * as yaml from 'js-yaml';
import { claudemonSessionClient } from './claudemonSessionClient';

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

interface SessionTabData {
  id: string;
  title: string;
  panes: SessionPaneData[];
  activePaneId: string;
  /** Spatial-canvas placement (x/y/w/h in world coords); absent in tabs mode. */
  canvas?: { x: number; y: number; w: number; h: number };
  /** Epoch ms of last activity — the 'timeline' view's sort axis. */
  lastActiveAt?: number;
}

interface SessionAgentData {
  id: string;
  name: string;
  global?: boolean;
  cwd: string;
  profileId?: string;
  model?: string;
  skipPermissions?: boolean;
  sessionId?: string;
  tabs: SessionTabData[];
  activeTabId: string;
}

interface SessionData {
  name: string;
  timestamp: string;
  // Agent-centric layout (current): a roster of agent workspaces, each with tabs.
  activeAgentId?: string;
  agents?: SessionAgentData[];
  // Legacy flat layout — a single set of tabs/panes — kept for backward compat.
  activeTabId?: string;
  tabs?: SessionTabData[];
  activePaneId?: string;
  panes?: SessionPaneData[];
}

interface SessionListEntry {
  name: string;
  filename: string;
  timestamp: string;
  paneCount: number;
  agentCount: number;
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

function getTerminalCwd(sessionId: string): string | undefined {
  // claudemon owns the PTY in a separate process, so we can't /proc-walk it.
  // Fall back to the cwd we spawned with (claudemonSessionClient tracks it).
  return claudemonSessionClient.getCwd(sessionId);
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
          const agents = session.agents ?? [];
          const paneCount = agents.length > 0
            ? agents.reduce((n, a) => n + (a.tabs ?? []).reduce((m, t) => m + (t.panes?.length || 0), 0), 0)
            : (session.tabs?.reduce((m, t) => m + (t.panes?.length || 0), 0) ?? session.panes?.length ?? 0);
          entries.push({
            name: session.name || file.replace('.yaml', ''),
            filename: file,
            timestamp: session.timestamp || '',
            paneCount,
            agentCount: agents.filter((a) => !a.global).length,
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

  /** Enrich every pane inside an agent roster's tabs with its terminal cwd. */
  enrichAgentsWithCwd(agents: SessionAgentData[], ptyMapping: Record<string, string>): SessionAgentData[] {
    return agents.map((agent) => ({
      ...agent,
      tabs: (agent.tabs ?? []).map((tab) => ({
        ...tab,
        panes: this.enrichPanesWithCwd(tab.panes ?? [], ptyMapping),
      })),
    }));
  }
}

export const sessionService = new SessionService();
