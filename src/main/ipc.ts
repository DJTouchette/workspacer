import { ipcMain, BrowserWindow, dialog } from 'electron';
import * as os from 'os';
import * as fs from 'fs';
import { randomUUID } from 'crypto';
import { configService } from './services/configService';
import { libraryService } from './services/libraryService';
import { sessionService } from './services/sessionService';
import { sessionHistory } from './services/sessionHistory';
import { layoutService } from './services/layoutService';
import { claudeSessionStore } from './services/claudeSessionStore';
import { agentNotifier } from './services/agentNotifier';
import { claudemonSessionClient } from './services/claudemonSessionClient';
import { buildClaudeArgv } from './services/claudeResolver';
import { supervisorMcpConfigPath, SUPERVISOR_SYSTEM_PROMPT } from './services/mcpConfig';
import { importChromeCookies, importChromeCookiesViaCDP } from './services/chromeCookieImport';
import { claudeProfiles } from './services/claudeProfiles';
import { listClaudeSessionsForDir } from './services/claudeSessionList';
import { HUB_HTTP_URL, getHubToken, getRemoteShareInfo } from './services/hubDaemon';
import { publishToHub, isHubConnected } from './services/hubClient';
import { IPC } from './shared/ipcChannels';
import type { ClaudeSessionSnapshot, AppConfig, AppConfigPartial, SessionData, LayoutInput, ProfileUpdate } from './shared/ipcTypes';

function detectDefaultShell(): string {
  if (process.platform === 'win32') {
    const gitBash = 'C:\\Program Files\\Git\\bin\\bash.exe';
    try { fs.accessSync(gitBash); return gitBash; } catch {}
    try { require('child_process').execSync('where pwsh.exe', { stdio: 'ignore' }); return 'pwsh.exe'; } catch {}
    return 'powershell.exe';
  }
  return process.env.SHELL || '/bin/sh';
}

export function registerIpcHandlers(mainWindow: BrowserWindow): void {
  claudemonSessionClient.setMainWindow(mainWindow);
  libraryService.setMainWindow(mainWindow);

  // ── Library (reusable prompts + skills) ──
  ipcMain.handle(IPC.LIBRARY_LIST, (_event, cwd?: string) => libraryService.list(cwd));
  ipcMain.handle(IPC.LIBRARY_SAVE, (_event, input: unknown) => libraryService.save(input as any));
  ipcMain.handle(IPC.LIBRARY_REMOVE, (_event, scope: 'global' | 'project' | 'claude', id: string, cwd?: string, kind?: 'prompt' | 'skill' | 'agent') =>
    libraryService.remove(scope, id, cwd, kind));

  // Renderer reports which agent session is currently on screen, so the
  // notifier can suppress alerts for the agent you're actively watching.
  ipcMain.on(IPC.NOTIFY_SET_ACTIVE_SESSION, (_event, sessionId: string | null) => {
    agentNotifier.setActiveSession(sessionId);
  });

  // ── Generic terminal (non-Claude shells) — routed through claudemon ──
  ipcMain.handle(IPC.TERMINAL_CREATE, async (_event, shell: string, cwd?: string, cols?: number, rows?: number) => {
    try {
      const resolvedShell = shell || detectDefaultShell();
      const resolvedCwd = cwd && fs.existsSync(cwd) ? cwd : os.homedir();
      return claudemonSessionClient.spawn({
        argv: [resolvedShell],
        cwd: resolvedCwd,
        cols,
        rows,
        portChannel: IPC.TERMINAL_PORT,
      });
    } catch (err: any) {
      console.error('[IPC] terminal:create failed:', err?.message);
      throw err;
    }
  });

  ipcMain.handle(IPC.TERMINAL_RESIZE, (_event, id: string, cols: number, rows: number) =>
    claudemonSessionClient.resize(id, cols, rows));
  ipcMain.handle(IPC.TERMINAL_CLOSE, (_event, id: string) =>
    claudemonSessionClient.close(id));

  // ── Claude sessions (delegated to claudemon) ──
  ipcMain.handle(IPC.CLAUDE_SPAWN, async (_event, opts: { cwd?: string; profileId?: string; model?: string; skipPermissions?: boolean; resumeSessionId?: string; cols?: number; rows?: number; supervisor?: boolean }) => {
    const profile = opts.profileId ? claudeProfiles.getProfile(opts.profileId) : undefined;
    const env: Record<string, string> = {};
    if (profile?.configDir) {
      env.CLAUDE_CONFIG_DIR = profile.configDir.replace(/^~/, os.homedir());
    }
    // Pin the session id so claude names its transcript `<id>.jsonl` and our
    // id == claude's id == the filename — correct transcripts even with many
    // sessions in one cwd. Resuming keeps the existing id.
    const sessionId = opts.resumeSessionId || randomUUID();
    const argv = buildClaudeArgv({
      extraArgs: profile?.extraArgs,
      resumeSessionId: opts.resumeSessionId,
      model: opts.model,
      skipPermissions: opts.skipPermissions,
      sessionId,
      // Supervisor sessions get the MCP facade config + pre-allowed tools +
      // role prompt injected so the agent can observe and drive the fleet.
      ...(opts.supervisor && {
        mcpConfig: supervisorMcpConfigPath(),
        appendSystemPrompt: SUPERVISOR_SYSTEM_PROMPT,
        allowedTools: ['mcp__workspacer'],
      }),
    });
    const cwd = opts.cwd ?? process.env.HOME ?? os.homedir();
    return claudemonSessionClient.spawn({ argv, cwd, cols: opts.cols, rows: opts.rows, env, sessionId });
  });

  // ── Hub (control-plane / event bus) ──
  ipcMain.handle(IPC.HUB_LIST_PLUGINS, async () => {
    try {
      const res = await fetch(`${HUB_HTTP_URL}/plugins`);
      if (!res.ok) return [];
      return await res.json();
    } catch {
      return [];
    }
  });
  ipcMain.handle(IPC.HUB_PUBLISH, (_event, ev: { type: string; source?: string; data?: unknown }) => {
    publishToHub(ev);
  });
  ipcMain.handle(IPC.HUB_GET_STATUS, () => ({ connected: isHubConnected() }));
  // Connection info for the remote-control client (URL + token for a QR/share).
  ipcMain.handle(IPC.HUB_GET_REMOTE_INFO, () => getRemoteShareInfo());
  // When remote auth is on, the hub's mutating routes require the token; the
  // local UI presents it via the same Authorization header a remote client uses.
  const hubAuthHeaders = (): Record<string, string> => {
    const token = getHubToken();
    const h: Record<string, string> = { 'Content-Type': 'application/json' };
    if (token) h['Authorization'] = `Bearer ${token}`;
    return h;
  };
  ipcMain.handle(IPC.HUB_INSTALL_PLUGIN, async (_event, url: string) => {
    try {
      const res = await fetch(`${HUB_HTTP_URL}/plugins/install`, {
        method: 'POST',
        headers: hubAuthHeaders(),
        body: JSON.stringify({ url }),
      });
      const body = await res.json() as any;
      if (!res.ok) return { ok: false, error: body?.error || `HTTP ${res.status}` };
      return { ok: true, plugin: body };
    } catch (err) {
      return { ok: false, error: String((err as Error)?.message ?? err) };
    }
  });
  ipcMain.handle(IPC.HUB_REMOVE_PLUGIN, async (_event, id: string) => {
    try {
      await fetch(`${HUB_HTTP_URL}/plugins/remove`, {
        method: 'POST',
        headers: hubAuthHeaders(),
        body: JSON.stringify({ id }),
      });
      return { ok: true };
    } catch (err) {
      return { ok: false, error: String((err as Error)?.message ?? err) };
    }
  });

  // Model choices for the spawn dialog. Dynamic by design: the aliases always
  // resolve to the latest model of each family (so they track Claude Code
  // updates with zero maintenance), and `seen` carries concrete ids observed
  // in real transcripts — past sessions plus anything persisted in config.
  ipcMain.handle(IPC.CLAUDE_LIST_MODELS, () => {
    const cfg = configService.getConfig() as any;
    const persisted: string[] = Array.isArray(cfg.claude?.seenModels) ? cfg.claude.seenModels : [];
    const live = claudeSessionStore.getAllSnapshots()
      .map((s) => s.usage?.model)
      .filter((m): m is string => !!m);
    const seen = Array.from(new Set([...persisted, ...live])).sort();
    return {
      defaultModel: typeof cfg.claude?.defaultModel === 'string' ? cfg.claude.defaultModel : '',
      skipPermissionsDefault: cfg.claude?.skipPermissionsDefault === true,
      aliases: [
        { value: 'opus', label: 'Opus — latest' },
        { value: 'sonnet', label: 'Sonnet — latest' },
        { value: 'haiku', label: 'Haiku — latest' },
      ],
      seen,
    };
  });

  ipcMain.handle(IPC.CLAUDE_MESSAGE, (_event, sessionId: string, text: string) =>
    claudemonSessionClient.message(sessionId, text));
  ipcMain.handle(IPC.CLAUDE_APPROVE, (_event, sessionId: string, decision: 'yes' | 'no' | 'always', reason?: string) =>
    claudemonSessionClient.approve(sessionId, decision, reason));
  ipcMain.handle(IPC.CLAUDE_ANSWER, (_event, sessionId: string, payload: { option?: number; text?: string; answers?: string[] }) =>
    claudemonSessionClient.answer(sessionId, payload));
  ipcMain.handle(IPC.CLAUDE_RESIZE, (_event, sessionId: string, cols: number, rows: number) =>
    claudemonSessionClient.resize(sessionId, cols, rows));
  ipcMain.handle(IPC.CLAUDE_SIGNAL, (_event, sessionId: string, signal: string) =>
    claudemonSessionClient.signal(sessionId, signal));
  ipcMain.handle(IPC.CLAUDE_CLOSE, (_event, sessionId: string) =>
    claudemonSessionClient.close(sessionId));
  ipcMain.handle(IPC.CLAUDE_ATTACH, (_event, paneId: string, sessionId: string) =>
    claudemonSessionClient.attach(paneId, sessionId, IPC.CLAUDE_PORT));
  ipcMain.handle(IPC.CLAUDE_DETACH, (_event, paneId: string) =>
    claudemonSessionClient.detach(paneId));
  ipcMain.handle(IPC.CLAUDE_GATE, (_event, sessionId: string, on: boolean) =>
    claudemonSessionClient.setGate(sessionId, on));

  ipcMain.handle(IPC.CLAUDE_SESSION_GET, (_event, sessionId: string): ClaudeSessionSnapshot | null =>
    claudeSessionStore.getSnapshot(sessionId));

  ipcMain.handle(IPC.CLAUDE_SESSION_GET_ALL, (): ClaudeSessionSnapshot[] => {
    return claudeSessionStore.getAllSnapshots();
  });

  // terminal:write — writes go through MessagePort directly
  // terminal:resize / terminal:close registered above on the daemon-backed path

  // ── Chrome cookie import ──
  //
  // Two strategies behind the same handler. `method: 'cdp'` (default) launches
  // a headless Chrome and reads cookies via the DevTools Protocol — works
  // with v20 (app-bound) encryption. `method: 'direct'` reads the SQLite +
  // DPAPI directly — fast, no Chrome needed, but only works for v10/v11.
  ipcMain.handle(IPC.CHROME_COOKIES_IMPORT, async (_e, opts?: { domainFilter?: string[]; method?: 'cdp' | 'direct'; browser?: 'chrome' | 'edge' }) => {
    const method = opts?.method ?? 'cdp';
    try {
      if (method === 'cdp') {
        return await importChromeCookiesViaCDP({ domainFilter: opts?.domainFilter, browser: opts?.browser ?? 'chrome' });
      }
      return await importChromeCookies({ domainFilter: opts?.domainFilter });
    } catch (err: any) {
      return { imported: 0, skipped: 0, errors: [err?.message ?? String(err)] };
    }
  });

  // Config handlers
  ipcMain.handle(IPC.CONFIG_GET, (): AppConfig => {
    return configService.getConfig() as AppConfig;
  });

  ipcMain.handle(IPC.CONFIG_RELOAD, (): AppConfig => {
    return configService.reloadConfig() as AppConfig;
  });

  ipcMain.handle(IPC.CONFIG_GET_PATH, () => {
    return configService.getConfigPath();
  });

  ipcMain.handle(IPC.CONFIG_SAVE, (_event, partial: AppConfigPartial): AppConfig => {
    return configService.saveConfig(partial as any) as AppConfig;
  });

  // Session handlers
  ipcMain.handle(IPC.SESSION_LIST, () => {
    return sessionService.listSessions();
  });

  ipcMain.handle(IPC.SESSION_LOAD, (_event, filename: string) => {
    return sessionService.loadSession(filename);
  });

  ipcMain.handle(IPC.SESSION_SAVE, (_event, data: SessionData) => {
    const ptyMapping = data.ptyMapping || {};

    // Current layout: a roster of agent workspaces, each with its own tabs.
    if (Array.isArray(data.agents)) {
      return sessionService.saveSession({
        name: data.name,
        timestamp: new Date().toISOString(),
        activeAgentId: data.activeAgentId,
        agents: sessionService.enrichAgentsWithCwd(data.agents as any, ptyMapping),
      });
    }

    // Legacy flat layout (single set of tabs) — kept for backward compat.
    const enrichedTabs = (data.tabs || []).map((tab: any) => ({
      ...tab,
      panes: sessionService.enrichPanesWithCwd(tab.panes || [], ptyMapping),
    }));
    return sessionService.saveSession({
      name: data.name,
      timestamp: new Date().toISOString(),
      activeTabId: data.activeTabId,
      tabs: enrichedTabs,
    });
  });

  ipcMain.handle(IPC.SESSION_DELETE, (_event, filename: string) => {
    sessionService.deleteSession(filename);
  });

  // ── Analytics (old-session metadata) ──
  ipcMain.handle(IPC.ANALYTICS_SUMMARY, () => sessionHistory.summary());
  ipcMain.handle(IPC.ANALYTICS_RECENT, (_event, limit?: number) => sessionHistory.recent(limit));

  // ── Layout templates (reusable directory + pane arrangements) ──
  ipcMain.handle(IPC.LAYOUTS_LIST, () => layoutService.list());
  ipcMain.handle(IPC.LAYOUTS_SAVE, (_event, layout: LayoutInput) => layoutService.save(layout));
  ipcMain.handle(IPC.LAYOUTS_DELETE, (_event, id: string) => layoutService.remove(id));

  // App info
  ipcMain.handle(IPC.APP_GET_CWD, () => {
    return process.cwd();
  });

  // Dialog
  ipcMain.handle(IPC.DIALOG_PICK_FOLDER, async (_event, defaultPath?: string) => {
    const result = await dialog.showOpenDialog(mainWindow, {
      title: 'Choose working directory for Claude',
      defaultPath: defaultPath || process.cwd(),
      properties: ['openDirectory'],
    });
    if (result.canceled || result.filePaths.length === 0) return null;
    return result.filePaths[0];
  });

  ipcMain.handle(IPC.DIALOG_PICK_FILES, async (_event, defaultPath?: string) => {
    const result = await dialog.showOpenDialog(mainWindow, {
      title: 'Attach files',
      defaultPath: defaultPath || process.cwd(),
      properties: ['openFile', 'multiSelections'],
    });
    if (result.canceled) return [];
    return result.filePaths;
  });

  // ── Claude Profiles ──

  // ── Claude Session Discovery ──

  ipcMain.handle(IPC.CLAUDE_SESSIONS_LIST_FOR_DIR, (_event, cwd: string) =>
    listClaudeSessionsForDir(cwd));

  ipcMain.handle(IPC.CLAUDE_PROFILES_LIST, () => claudeProfiles.getProfiles());
  ipcMain.handle(IPC.CLAUDE_PROFILES_ADD, (_event, name: string, configDir: string, extraArgs: string[]) =>
    claudeProfiles.addProfile(name, configDir, extraArgs));
  ipcMain.handle(IPC.CLAUDE_PROFILES_UPDATE, (_event, id: string, updates: ProfileUpdate) =>
    claudeProfiles.updateProfile(id, updates));
  ipcMain.handle(IPC.CLAUDE_PROFILES_REMOVE, (_event, id: string) =>
    claudeProfiles.removeProfile(id));

}
