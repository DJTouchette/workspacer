import { ipcMain, BrowserWindow, dialog } from 'electron';
import * as os from 'os';
import * as fs from 'fs';
import { randomUUID } from 'crypto';
import { configService } from './services/configService';
import { libraryService } from './services/libraryService';
import { sessionService } from './services/sessionService';
import { pluginSettings } from './services/pluginSettingsService';
import { sessionHistory } from './services/sessionHistory';
import { layoutService } from './services/layoutService';
import { claudeSessionStore } from './services/claudeSessionStore';
import { listClaudeModels } from './services/claudeModels';
import { agentNotifier } from './services/agentNotifier';
import { claudemonSessionClient } from './services/claudemonSessionClient';
import { buildClaudeArgv } from './services/claudeResolver';
import { resolveAgentBinary } from './services/agentProviders';
import { facadeSpawnArgs, buildSessionMcpConfig, MCP_FACADE_URL, managedFacadeInstructions } from './services/mcpConfig';
import { installSupervisorSkill, ensureSupervisorHome } from './services/supervisorSkill';
import { importChromeCookies, importChromeCookiesViaCDP } from './services/chromeCookieImport';
import { claudeProfiles } from './services/claudeProfiles';
import { listClaudeSessionsForDir } from './services/claudeSessionList';
import { readTextFile, writeTextFile, listDir } from './services/fileService';
import { startWatch, stopWatch, setEmitSink } from './services/fileWatchService';
import { searchProject } from './services/searchService';
import { HUB_HTTP_URL, getHubToken, getRemoteShareInfo } from './services/hubDaemon';
import { publishToHub, isHubConnected, callHub } from './services/hubClient';
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

let ipcHandlersRegistered = false;

export function registerIpcHandlers(mainWindow: BrowserWindow): void {
  claudemonSessionClient.setMainWindow(mainWindow);
  libraryService.setMainWindow(mainWindow);

  // One file watcher serves both transports: push the coalesced change to the
  // desktop renderer (file:changed) AND mirror it onto the hub bus (fs.changed)
  // so the web client — which subscribes there via the fs.watch capability —
  // sees the same event. publishToHub is a no-op when remote sharing is off.
  setEmitSink(({ path, eventType }) => {
    if (mainWindow && !mainWindow.isDestroyed()) {
      mainWindow.webContents.send(IPC.FILE_CHANGED, { path, eventType });
    }
    publishToHub({ type: 'fs.changed', data: { path, eventType } });
  });

  // ipcMain.handle registrations throw if a channel is already registered.
  // Guard so a second createWindow() call (macOS dock 'activate') is safe.
  if (ipcHandlersRegistered) return;
  ipcHandlersRegistered = true;

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

  // The Windows native caption buttons (min/max/close) live in a titleBarOverlay
  // whose color is fixed at window-creation time. The renderer re-paints it to
  // match the active theme so the buttons blend into the title bar. No-op off
  // Windows, where setTitleBarOverlay is unavailable.
  ipcMain.on(IPC.WINDOW_SET_OVERLAY, (_event, opts: { color: string; symbolColor: string }) => {
    if (process.platform !== 'win32') return;
    if (!mainWindow || mainWindow.isDestroyed()) return;
    try {
      mainWindow.setTitleBarOverlay({ color: opts.color, symbolColor: opts.symbolColor });
    } catch (err: any) {
      console.error('[IPC] window:setOverlay failed:', err?.message);
    }
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
  ipcMain.handle(IPC.CLAUDE_SPAWN, async (_event, opts: { cwd?: string; provider?: 'claude' | 'codex' | 'opencode'; profileId?: string; model?: string; skipPermissions?: boolean; resumeSessionId?: string; cols?: number; rows?: number; supervisor?: boolean; mcpFacade?: boolean; label?: string; parentSessionId?: string; mcpItemIds?: string[] }) => {
    // Provider selects the coding-agent backend. OpenCode and Codex are Tier-2
    // managed: claudemon drives their machine interface (`opencode serve` HTTP+SSE
    // / `codex app-server` JSON-RPC) and translates events into the shared session
    // model, so they light up the GUI / Fleet Deck like a Claude session — no PTY.
    const provider = opts.provider ?? 'claude';
    if (provider !== 'claude') {
      let cwd = opts.cwd || process.env.HOME || os.homedir();
      const bin = resolveAgentBinary(provider);
      // A managed supervisor / facade worker gets the workspacer MCP facade
      // (registered with the provider's own MCP config by the adapter) plus the
      // role instructions injected on its opening turn.
      const wantsFacade = opts.supervisor || opts.mcpFacade;
      if (opts.supervisor && !opts.cwd) cwd = ensureSupervisorHome();
      return claudemonSessionClient.spawnManaged({
        provider, cwd, model: opts.model, bin, yolo: opts.skipPermissions,
        ...(wantsFacade && { mcp: MCP_FACADE_URL, instructions: managedFacadeInstructions(!!opts.supervisor) }),
      });
    }
    const profile = opts.profileId ? claudeProfiles.getProfile(opts.profileId) : undefined;
    const env: Record<string, string> = {};
    if (profile?.configDir) {
      env.CLAUDE_CONFIG_DIR = profile.configDir.replace(/^~/, os.homedir());
    }
    // Pin the session id so claude names its transcript `<id>.jsonl` and our
    // id == claude's id == the filename — correct transcripts even with many
    // sessions in one cwd. Resuming keeps the existing id.
    const sessionId = opts.resumeSessionId || randomUUID();
    // Record name/parent before the session registers so adopted cards are
    // enriched from the very first hook event.
    claudeSessionStore.setSpawnMeta(sessionId, { label: opts.label, parentSessionId: opts.parentSessionId, isSupervisor: opts.supervisor });

    // Per-spawn MCP servers selected from the Library (kind 'mcp'). Resolve the
    // chosen item ids to their configs, write a session-scoped --mcp-config, and
    // pre-allow their tools. `--strict-mcp-config` so the session sees exactly
    // these servers, not the user's global ones. Skipped for supervisors, which
    // get the workspacer facade config instead.
    // Sessions with the workspacer MCP facade (full supervisors, or plain
    // facade workers a supervisor spawns) take the facade config instead of the
    // user's library MCP servers.
    const wantsFacade = opts.supervisor || opts.mcpFacade;
    let userMcp: { path: string; toolNames: string[] } | null = null;
    if (!wantsFacade && opts.mcpItemIds && opts.mcpItemIds.length) {
      const wanted = new Set(opts.mcpItemIds);
      const servers = libraryService.list(opts.cwd)
        .filter((it) => it.kind === 'mcp' && it.mcp && wanted.has(it.id))
        .map((it) => ({ id: it.id, mcp: it.mcp! }));
      userMcp = buildSessionMcpConfig(sessionId, servers);
    }

    // Supervisors: install the /supervise skill and default to the configured
    // supervisor model when none was passed explicitly.
    const supCfg = configService.getConfig().supervisor;
    let model = opts.model;
    if (opts.supervisor) {
      installSupervisorSkill();
      if (!model) model = supCfg?.model || undefined;
    }

    const argv = buildClaudeArgv({
      extraArgs: profile?.extraArgs,
      resumeSessionId: opts.resumeSessionId,
      model,
      skipPermissions: opts.skipPermissions,
      sessionId,
      // Facade sessions get the MCP config + pre-allowed tools + a role prompt.
      // A supervisor also learns its session id and is kicked into /supervise;
      // a plain facade worker just gets the tools.
      ...(wantsFacade && facadeSpawnArgs({
        sessionId,
        supervisor: opts.supervisor,
        summarizerModel: supCfg?.summarizerModel,
        pollSeconds: supCfg?.pollSeconds,
      })),
      // User-selected MCP servers (non-facade sessions).
      ...(userMcp && {
        mcpConfig: userMcp.path,
        strictMcpConfig: true,
        allowedTools: userMcp.toolNames,
      }),
    });
    // Fleet supervisors with no explicit cwd open in their dedicated home
    // (~/.workspacer) rather than inheriting some agent's repo.
    let cwd = opts.cwd || process.env.HOME || os.homedir();
    if (opts.supervisor && !opts.cwd) cwd = ensureSupervisorHome();
    return claudemonSessionClient.spawn({ argv, cwd, cols: opts.cols, rows: opts.rows, env, sessionId });
  });

  // ── Hub (control-plane / event bus) ──
  ipcMain.handle(IPC.HUB_LIST_PLUGINS, async () => {
    try {
      const res = await fetch(`${HUB_HTTP_URL}/plugins`);
      if (!res.ok) return [];
      const plugins = await res.json() as Array<{ id: string; [k: string]: unknown }>;
      // Merge each plugin's per-plugin bus token (served only on the token-guarded
      // /plugins/tokens, never on public /plugins) so the renderer can inject it
      // into that plugin's webview URL. Best-effort: no tokens → webviews can't
      // call capabilities, but the list still renders.
      try {
        const tokRes = await fetch(`${HUB_HTTP_URL}/plugins/tokens`, { headers: hubAuthHeaders() });
        if (tokRes.ok) {
          const tokens = await tokRes.json() as Record<string, string>;
          for (const p of plugins) {
            if (tokens[p.id]) (p as { busToken?: string }).busToken = tokens[p.id];
          }
        }
      } catch { /* tokens unavailable — degrade to no webview capability calls */ }
      // Tell the renderer where to load webview-only plugins' static UI from
      // (it serves at <hub>/plugins/ui/<id>/). main knows the hub address.
      for (const p of plugins) {
        if ((p as { ui?: string }).ui) (p as { uiBase?: string }).uiBase = HUB_HTTP_URL;
      }
      return plugins;
    } catch {
      return [];
    }
  });
  ipcMain.handle(IPC.HUB_PUBLISH, (_event, ev: { type: string; source?: string; data?: unknown }) => {
    publishToHub(ev);
  });
  ipcMain.handle(IPC.HUB_GET_STATUS, () => ({ connected: isHubConnected() }));

  // ── Shared layout document (hub-owned) ──
  // The hub owns the workspace layout so desktop + web mirror each other. These
  // proxy the renderer's reads/writes onto the hub's in-process layout.get /
  // layout.set capabilities; live changes arrive as layout.changed events,
  // forwarded to the renderer over LAYOUT_CHANGED (see hubClient / setupHub).
  ipcMain.handle(IPC.LAYOUT_GET, () => callHub('layout.get', {}));
  ipcMain.handle(IPC.LAYOUT_SET, (_event, data: unknown) => callHub('layout.set', { data }));
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
  // Mint an ephemeral, capability-scoped bus token for one agent-scoped plugin
  // pane, with ${agentCwd} bound to that agent's working directory. The renderer
  // injects it into the pane's webview URL so the plugin is confined to that
  // project's files instead of getting the static per-plugin token's (broader)
  // scope. Returns null on any failure — the renderer falls back to the static token.
  ipcMain.handle(IPC.HUB_PLUGIN_PANE_TOKEN, async (_event, pluginId: string, agentCwd?: string) => {
    try {
      const res = await fetch(`${HUB_HTTP_URL}/plugins/pane-token`, {
        method: 'POST',
        headers: hubAuthHeaders(),
        body: JSON.stringify({ pluginId, agentCwd: agentCwd ?? '' }),
      });
      if (!res.ok) return null;
      const body = await res.json() as { token?: string };
      return body?.token ?? null;
    } catch {
      return null;
    }
  });
  ipcMain.handle(IPC.HUB_PLUGIN_SETTINGS_GET, (_event, pluginId: string) => pluginSettings.get(pluginId));
  ipcMain.handle(IPC.HUB_PLUGIN_SETTINGS_SET, (_event, pluginId: string, values: Record<string, unknown>) => {
    const merged = pluginSettings.set(pluginId, values);
    // Tell any open pane of this plugin to re-apply live (the bridge listens).
    mainWindow.webContents.send(IPC.HUB_PLUGIN_SETTINGS_CHANGED, pluginId, merged);
    return merged;
  });
  ipcMain.handle(IPC.HUB_PLUGIN_PANE_TOKEN_REVOKE, async (_event, token: string) => {
    try {
      await fetch(`${HUB_HTTP_URL}/plugins/pane-token/revoke`, {
        method: 'POST',
        headers: hubAuthHeaders(),
        body: JSON.stringify({ token }),
      });
    } catch { /* best-effort; the hub also sweeps pane tokens on plugin unload */ }
  });
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
  ipcMain.handle(IPC.HUB_SET_PLUGIN_ENABLED, async (_event, args: { id: string; enabled: boolean }) => {
    try {
      const res = await fetch(`${HUB_HTTP_URL}/plugins/setEnabled`, {
        method: 'POST',
        headers: hubAuthHeaders(),
        body: JSON.stringify(args),
      });
      const body = await res.json() as any;
      if (!res.ok) return { ok: false, error: body?.error || `HTTP ${res.status}` };
      return { ok: true, plugin: body };
    } catch (err) {
      return { ok: false, error: String((err as Error)?.message ?? err) };
    }
  });

  // Model choices for the spawn dialog. Dynamic by design: the aliases always
  // resolve to the latest model of each family (so they track Claude Code
  // updates with zero maintenance), and `seen` carries concrete ids observed
  // in real transcripts — past sessions plus anything persisted in config.
  ipcMain.handle(IPC.CLAUDE_LIST_MODELS, () => listClaudeModels());

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

  // The dedicated supervisor home (~/.workspacer), created on demand. The
  // renderer resolves this before spawning a fleet supervisor so its card cwd
  // matches where the session actually opens.
  ipcMain.handle(IPC.APP_SUPERVISOR_HOME, () => {
    return ensureSupervisorHome();
  });

  // Files (editor pane). Errors (missing / too big / binary) reject the invoke,
  // which the EditorPane surfaces to the user.
  ipcMain.handle(IPC.FILE_READ, (_event, filePath: string) => readTextFile(filePath));
  ipcMain.handle(IPC.FILE_WRITE, (_event, filePath: string, contents: string) =>
    writeTextFile(filePath, contents),
  );
  ipcMain.handle(IPC.FILE_LIST_DIR, (_event, dirPath: string) => listDir(dirPath));

  // Watch a single file for external changes. Changes are pushed back via the
  // FILE_CHANGED channel (and the hub bus) by the sink installed above.
  ipcMain.handle(IPC.FILE_WATCH, (_event, filePath: string) => { startWatch(filePath); });
  ipcMain.handle(IPC.FILE_UNWATCH, (_event, filePath: string) => { stopWatch(filePath); });

  // Project-wide search (editor search sidebar), backed by ripgrep.
  ipcMain.handle(IPC.SEARCH_PROJECT, (_event, opts: Parameters<typeof searchProject>[0]) =>
    searchProject(opts));

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
  ipcMain.handle(IPC.CLAUDE_PROFILES_ADD, (_event, name: string, configDir: string, extraArgs: string[], mcpItemIds?: string[]) =>
    claudeProfiles.addProfile(name, configDir, extraArgs, mcpItemIds ?? []));
  ipcMain.handle(IPC.CLAUDE_PROFILES_UPDATE, (_event, id: string, updates: ProfileUpdate) =>
    claudeProfiles.updateProfile(id, updates));
  ipcMain.handle(IPC.CLAUDE_PROFILES_REMOVE, (_event, id: string) =>
    claudeProfiles.removeProfile(id));

}
