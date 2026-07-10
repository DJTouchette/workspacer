import { ipcMain, BrowserWindow, dialog, shell } from 'electron';
import * as os from 'os';
import * as fs from 'fs';
import { pathToFileURL } from 'url';
import { configService } from './services/configService';
import { libraryService } from './services/libraryService';
import { sessionService } from './services/sessionService';
import {
  peekLegacyPluginSettings,
  clearLegacyPluginSettings,
} from './services/pluginSettingsMigration';
import { sessionHistory } from './services/sessionHistory';
import { layoutService } from './services/layoutService';
import { claudeSessionStore } from './services/claudeSessionStore';
import { listClaudeModels } from './services/claudeModels';
import { workflowWatcher } from './services/workflowWatcher';
import { agentNotifier } from './services/agentNotifier';
import { claudemonSessionClient } from './services/claudemonSessionClient';
import { agentHandoffBrief } from './services/agentHandoff';
import { resolveAgentBinary, checkAllProviders } from './services/agentProviders';
import { spawnManagedAgent } from './services/managedSpawn';
import { spawnClaudeAgent } from './services/claudeSpawn';
import { logsDir } from './services/logFile';
import { ensureSupervisorHome } from './services/supervisorSkill';
import { importChromeCookies, importChromeCookiesViaCDP } from './services/chromeCookieImport';
import { claudeProfiles } from './services/claudeProfiles';
import { listClaudeSessionsForDir } from './services/claudeSessionList';
import { readTextFile, writeTextFile, listDir } from './services/fileService';
import { startWatch, stopWatch, setEmitSink } from './services/fileWatchService';
import { searchProject } from './services/searchService';
import * as git from './services/gitService';
import {
  HUB_HTTP_URL,
  HUB_PORT,
  getHubToken,
  getRemoteShareInfo,
  setRemoteShare,
} from './services/hubDaemon';
import { getTailscaleInfo, setTailscaleServe } from './services/tailscaleServe';
import { publishToHub, isHubConnected, callHub } from './services/hubClient';
import { IPC } from './shared/ipcChannels';
import type {
  ClaudeSessionSnapshot,
  AppConfig,
  AppConfigPartial,
  SessionData,
  LayoutInput,
  ProfileUpdate,
} from './shared/ipcTypes';

function detectDefaultShell(): string {
  if (process.platform === 'win32') {
    const gitBash = 'C:\\Program Files\\Git\\bin\\bash.exe';
    try {
      fs.accessSync(gitBash);
      return gitBash;
    } catch {}
    try {
      require('child_process').execSync('where pwsh.exe', { stdio: 'ignore' });
      return 'pwsh.exe';
    } catch {}
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
  ipcMain.handle(
    IPC.LIBRARY_REMOVE,
    (
      _event,
      scope: 'global' | 'project' | 'claude',
      id: string,
      cwd?: string,
      kind?: 'prompt' | 'skill' | 'agent',
    ) => libraryService.remove(scope, id, cwd, kind),
  );

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
  ipcMain.handle(
    IPC.TERMINAL_CREATE,
    async (_event, shell: string, cwd?: string, cols?: number, rows?: number) => {
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
    },
  );

  ipcMain.handle(IPC.TERMINAL_RESIZE, (_event, id: string, cols: number, rows: number) =>
    claudemonSessionClient.resize(id, cols, rows),
  );
  ipcMain.handle(IPC.TERMINAL_CLOSE, (_event, id: string) => claudemonSessionClient.close(id));

  // ── Claude sessions (delegated to claudemon) ──
  ipcMain.handle(
    IPC.CLAUDE_SPAWN,
    async (
      _event,
      opts: {
        cwd?: string;
        provider?: 'claude' | 'codex' | 'opencode' | 'pi';
        /** Claude only: 'pty' (classic TUI) or 'stream' (headless stream-json,
         *  managed adapter). Omitted = the config default (claude.transport). */
        transport?: 'pty' | 'stream';
        profileId?: string;
        model?: string;
        effort?: string;
        permissionMode?: string;
        skipPermissions?: boolean;
        resumeSessionId?: string;
        cols?: number;
        rows?: number;
        supervisor?: boolean;
        mcpFacade?: boolean;
        label?: string;
        parentSessionId?: string;
        mcpItemIds?: string[];
      },
    ) => {
      // Provider selects the coding-agent backend. OpenCode and Codex are Tier-2
      // managed: claudemon drives their machine interface (`opencode serve` HTTP+SSE
      // / `codex app-server` JSON-RPC) and translates events into the shared session
      // model, so they light up the GUI / Fleet Deck like a Claude session — no PTY.
      const provider = opts.provider ?? 'claude';
      if (provider !== 'claude') {
        // Managed (Tier-2) backend — driven by claudemon's adapter, not a PTY.
        // Shared with the `agents.spawn` hub capability so the two transports
        // can't diverge (see managedSpawn.ts).
        return spawnManagedAgent({
          provider,
          cwd: opts.cwd,
          model: opts.model,
          effort: opts.effort,
          skipPermissions: opts.skipPermissions || opts.permissionMode === 'yolo',
          resumeSessionId: opts.resumeSessionId,
          supervisor: opts.supervisor,
          mcpFacade: opts.mcpFacade,
          label: opts.label,
          parentSessionId: opts.parentSessionId,
          cols: opts.cols,
          rows: opts.rows,
        });
      }
      // Claude on the 'stream' transport is also managed — claudemon's
      // claude_stream adapter runs headless stream-json (no PTY). Same shared
      // dispatch as the other managed providers so the IPC and hub-bus spawn
      // paths can't drift (standing project rule; see managedSpawn.ts).
      const transport = opts.transport ?? configService.getConfig().claude?.transport ?? 'pty';
      if (transport === 'stream') {
        return spawnManagedAgent({
          provider: 'claude',
          transport: 'stream',
          cwd: opts.cwd,
          profileId: opts.profileId,
          model: opts.model,
          permissionMode: opts.permissionMode,
          skipPermissions: opts.skipPermissions,
          resumeSessionId: opts.resumeSessionId,
          supervisor: opts.supervisor,
          mcpFacade: opts.mcpFacade,
          label: opts.label,
          parentSessionId: opts.parentSessionId,
          mcpItemIds: opts.mcpItemIds,
        });
      }
      // Claude (Tier-1) PTY spawn. Shared with the `agents.spawn` hub capability
      // via spawnClaudeAgent so the two transports can't drift (see claudeSpawn.ts).
      return spawnClaudeAgent({
        cwd: opts.cwd,
        profileId: opts.profileId,
        model: opts.model,
        permissionMode: opts.permissionMode,
        skipPermissions: opts.skipPermissions,
        resumeSessionId: opts.resumeSessionId,
        supervisor: opts.supervisor,
        mcpFacade: opts.mcpFacade,
        label: opts.label,
        parentSessionId: opts.parentSessionId,
        cols: opts.cols,
        rows: opts.rows,
        mcpItemIds: opts.mcpItemIds,
      });
    },
  );

  // ── Hub (control-plane / event bus) ──
  ipcMain.handle(IPC.HUB_LIST_PLUGINS, async () => {
    try {
      const res = await fetch(`${HUB_HTTP_URL}/plugins`);
      if (!res.ok) return [];
      const plugins = (await res.json()) as Array<{ id: string; [k: string]: unknown }>;
      // Merge each plugin's per-plugin bus token (served only on the token-guarded
      // /plugins/tokens, never on public /plugins) so the renderer can inject it
      // into that plugin's webview URL. Best-effort: no tokens → webviews can't
      // call capabilities, but the list still renders.
      try {
        const tokRes = await fetch(`${HUB_HTTP_URL}/plugins/tokens`, { headers: hubAuthHeaders() });
        if (tokRes.ok) {
          const tokens = (await tokRes.json()) as Record<string, string>;
          for (const p of plugins) {
            if (tokens[p.id]) (p as { busToken?: string }).busToken = tokens[p.id];
          }
        }
      } catch {
        /* tokens unavailable — degrade to no webview capability calls */
      }
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
  ipcMain.handle(
    IPC.HUB_PUBLISH,
    (_event, ev: { type: string; source?: string; data?: unknown }) => {
      publishToHub(ev);
    },
  );
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
  ipcMain.handle(IPC.HUB_SET_REMOTE_SHARE, (_event, enabled: boolean) => setRemoteShare(!!enabled));
  ipcMain.handle(IPC.TAILSCALE_GET_INFO, () => getTailscaleInfo(HUB_PORT));
  ipcMain.handle(IPC.TAILSCALE_SET_SERVE, (_event, enable: boolean) =>
    setTailscaleServe(HUB_PORT, !!enable),
  );
  ipcMain.handle(IPC.LOGS_OPEN_FOLDER, async () => {
    const dir = logsDir();
    try {
      await fs.promises.mkdir(dir, { recursive: true });
    } catch {
      /* best effort */
    }
    const err = await shell.openPath(dir);
    return { ok: !err, error: err || undefined };
  });
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
      const body = (await res.json()) as { token?: string };
      return body?.token ?? null;
    } catch {
      return null;
    }
  });
  // Plugin settings live on the hub (single source of truth, shared with
  // web/remote): GET returns the manifest defaults with the user's overlay
  // merged on top, POST validates + persists a partial and returns the merged
  // result. Both go through the token-guarded /plugins/settings route.
  const hubGetPluginSettings = async (pluginId: string): Promise<Record<string, unknown>> => {
    try {
      const res = await fetch(
        `${HUB_HTTP_URL}/plugins/settings?pluginId=${encodeURIComponent(pluginId)}`,
        {
          headers: hubAuthHeaders(),
        },
      );
      if (!res.ok) return {};
      const body = (await res.json()) as { values?: Record<string, unknown> };
      return body?.values ?? {};
    } catch {
      return {}; // hub down / plugin not loaded — caller falls back to defaults
    }
  };
  const hubSetPluginSettings = async (
    pluginId: string,
    values: Record<string, unknown>,
  ): Promise<Record<string, unknown> | null> => {
    try {
      const res = await fetch(`${HUB_HTTP_URL}/plugins/settings`, {
        method: 'POST',
        headers: hubAuthHeaders(),
        body: JSON.stringify({ pluginId, values }),
      });
      if (!res.ok) return null;
      const body = (await res.json()) as { values?: Record<string, unknown> };
      return body?.values ?? {};
    } catch {
      return null;
    }
  };
  ipcMain.handle(IPC.HUB_PLUGIN_SETTINGS_GET, async (_event, pluginId: string) => {
    // A read means this plugin is loaded in the hub, so it's the safe moment to
    // migrate any legacy locally-stored overlay: push it, then drop the local
    // copy only once the hub accepted it (a failed push retries on a later read).
    const legacy = peekLegacyPluginSettings(pluginId);
    if (legacy) {
      const migrated = await hubSetPluginSettings(pluginId, legacy);
      if (migrated !== null) clearLegacyPluginSettings(pluginId);
    }
    return hubGetPluginSettings(pluginId);
  });
  ipcMain.handle(
    IPC.HUB_PLUGIN_SETTINGS_SET,
    async (_event, pluginId: string, values: Record<string, unknown>) => {
      const merged = await hubSetPluginSettings(pluginId, values);
      if (merged === null) return {};
      // Tell any open pane of this plugin to re-apply live (the bridge listens).
      // Remote-origin writes reach the renderer via the plugin.settings.changed
      // bus event (bridged in hubClient); this is the fast path for local writes.
      mainWindow.webContents.send(IPC.HUB_PLUGIN_SETTINGS_CHANGED, pluginId, merged);
      return merged;
    },
  );
  ipcMain.handle(IPC.HUB_PLUGIN_PANE_TOKEN_REVOKE, async (_event, token: string) => {
    try {
      await fetch(`${HUB_HTTP_URL}/plugins/pane-token/revoke`, {
        method: 'POST',
        headers: hubAuthHeaders(),
        body: JSON.stringify({ token }),
      });
    } catch {
      /* best-effort; the hub also sweeps pane tokens on plugin unload */
    }
  });
  ipcMain.handle(IPC.HUB_INSTALL_PLUGIN, async (_event, url: string) => {
    try {
      const res = await fetch(`${HUB_HTTP_URL}/plugins/install`, {
        method: 'POST',
        headers: hubAuthHeaders(),
        body: JSON.stringify({ url }),
      });
      const body = (await res.json()) as any;
      if (!res.ok) return { ok: false, error: body?.error || `HTTP ${res.status}` };
      return { ok: true, plugin: body };
    } catch (err) {
      return { ok: false, error: String((err as Error)?.message ?? err) };
    }
  });
  // Inspect a GitHub plugin before installing — returns its manifest so the
  // install dialog can show what it is and what it requires up front. No install.
  ipcMain.handle(IPC.HUB_INSPECT_PLUGIN, async (_event, url: string) => {
    try {
      const res = await fetch(`${HUB_HTTP_URL}/plugins/inspect`, {
        method: 'POST',
        headers: hubAuthHeaders(),
        body: JSON.stringify({ url }),
      });
      const body = (await res.json()) as any;
      if (!res.ok) return { ok: false, error: body?.error || `HTTP ${res.status}` };
      return { ok: true, plugin: body };
    } catch (err) {
      return { ok: false, error: String((err as Error)?.message ?? err) };
    }
  });
  // Read-only catalog of bundled example plugins the user can add. Returns the
  // manifests as-is; the renderer derives each one's runtime requirement and
  // cross-references the installed list to show an "Added" state.
  ipcMain.handle(IPC.HUB_LIST_EXAMPLES, async () => {
    try {
      const res = await fetch(`${HUB_HTTP_URL}/plugins/examples`);
      if (!res.ok) return [];
      return (await res.json()) as Array<{ id: string; [k: string]: unknown }>;
    } catch {
      return [];
    }
  });
  // Add one bundled example by manifest id (the hub copies it from the examples
  // dir into the writable plugins dir and supervises it — no network).
  ipcMain.handle(IPC.HUB_INSTALL_EXAMPLE, async (_event, id: string) => {
    try {
      const res = await fetch(`${HUB_HTTP_URL}/plugins/examples/install`, {
        method: 'POST',
        headers: hubAuthHeaders(),
        body: JSON.stringify({ id }),
      });
      const body = (await res.json()) as any;
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
  ipcMain.handle(
    IPC.HUB_SET_PLUGIN_ENABLED,
    async (_event, args: { id: string; enabled: boolean }) => {
      try {
        const res = await fetch(`${HUB_HTTP_URL}/plugins/setEnabled`, {
          method: 'POST',
          headers: hubAuthHeaders(),
          body: JSON.stringify(args),
        });
        const body = (await res.json()) as any;
        if (!res.ok) return { ok: false, error: body?.error || `HTTP ${res.status}` };
        return { ok: true, plugin: body };
      } catch (err) {
        return { ok: false, error: String((err as Error)?.message ?? err) };
      }
    },
  );

  // Model choices for the spawn dialog. Dynamic by design: the aliases always
  // resolve to the latest model of each family (so they track Claude Code
  // updates with zero maintenance), and `seen` carries concrete ids observed
  // in real transcripts — past sessions plus anything persisted in config.
  ipcMain.handle(IPC.CLAUDE_LIST_MODELS, () => listClaudeModels());

  // Full transcript of one subagent, for the timeline / watch-pane drill-in.
  // runId names a workflow run; null means a plain Agent-tool subagent.
  // Resolved from the on-disk dir by the watcher; null if it's no longer around.
  ipcMain.handle(
    IPC.WORKFLOW_AGENT_TRANSCRIPT,
    (_event, sessionId: string, runId: string | null, agentId: string) =>
      workflowWatcher.readAgentTranscript(sessionId, runId, agentId),
  );

  // Rich variant for the watch pane's GUI view: ConversationTurn-shaped turns
  // with real tool-call objects, renderable by the main conversation components.
  ipcMain.handle(
    IPC.WORKFLOW_AGENT_CONVERSATION,
    (_event, sessionId: string, runId: string | null, agentId: string) =>
      workflowWatcher.readAgentConversation(sessionId, runId, agentId),
  );

  // Live model catalog for a managed provider (codex/opencode/pi). We resolve
  // the launcher binary the same way spawning does, then query the provider's
  // own CLI/server via claudemon so the picker matches the installed version.
  ipcMain.handle(
    IPC.PROVIDER_LIST_MODELS,
    (_event, provider: 'codex' | 'opencode' | 'pi', cwd?: string) => {
      const customBin = configService.getConfig().agents?.binaries?.[provider] ?? '';
      return claudemonSessionClient.listProviderModels(
        provider,
        cwd,
        resolveAgentBinary(provider, customBin),
      );
    },
  );

  // Detection status for all providers — returns path + found flag, using the
  // user-configured binary overrides from config when present.
  ipcMain.handle(IPC.PROVIDER_CHECK_ALL, () => {
    const binaries = configService.getConfig().agents?.binaries ?? {};
    return checkAllProviders(binaries);
  });

  ipcMain.handle(IPC.CLAUDE_MESSAGE, (_event, sessionId: string, text: string) =>
    claudemonSessionClient.message(sessionId, text),
  );
  // Live permission-mode switch (no restart). On success, reflect the
  // daemon-confirmed mode into the snapshot store immediately — the switch
  // itself fires no hook, so telemetry would otherwise lag until the next one.
  ipcMain.handle(
    IPC.CLAUDE_SET_PERMISSION_MODE,
    async (_event, sessionId: string, mode: string) => {
      const result = await claudemonSessionClient.setPermissionMode(sessionId, mode);
      if (result.ok && result.mode) claudeSessionStore.notePermissionMode(sessionId, result.mode);
      return result;
    },
  );
  // Live model switch for managed providers (no restart). Confirmation flows
  // back through the status line (codex broadcasts thread/settings/updated),
  // so no store note is needed here.
  ipcMain.handle(
    IPC.CLAUDE_SET_MODEL,
    (_event, sessionId: string, model?: string, effort?: string) =>
      claudemonSessionClient.setModel(sessionId, model, effort),
  );
  // Cross-provider handoff: daemon distills the session's conversation into a
  // brief under ~/.workspacer/handoffs/; the renderer spawns the successor and
  // points its first message at the file.
  ipcMain.handle(IPC.CLAUDE_HANDOFF_BRIEF, (_event, sessionId: string) =>
    claudemonSessionClient.handoffBrief(sessionId),
  );
  // Agent-authored brief: the source agent writes the file itself (it's the
  // only thing holding the session in context); falls back to the mechanical
  // brief if it doesn't deliver. Resolves only once a brief file exists.
  ipcMain.handle(IPC.CLAUDE_HANDOFF_AGENT_BRIEF, (_event, sessionId: string) =>
    agentHandoffBrief(sessionId),
  );
  ipcMain.handle(
    IPC.CLAUDE_APPROVE,
    (_event, sessionId: string, decision: 'yes' | 'no' | 'always', reason?: string) =>
      claudemonSessionClient.approve(sessionId, decision, reason),
  );
  ipcMain.handle(
    IPC.CLAUDE_ANSWER,
    async (
      _event,
      sessionId: string,
      payload: { option?: number; text?: string; answers?: string[] },
    ) => {
      const res = await claudemonSessionClient.answer(sessionId, payload);
      claudeSessionStore.clearPendingQuestions(sessionId);
      return res;
    },
  );
  ipcMain.handle(IPC.CLAUDE_RESIZE, (_event, sessionId: string, cols: number, rows: number) =>
    claudemonSessionClient.resize(sessionId, cols, rows),
  );
  ipcMain.handle(IPC.CLAUDE_SIGNAL, (_event, sessionId: string, signal: string) =>
    claudemonSessionClient.signal(sessionId, signal),
  );
  ipcMain.handle(IPC.CLAUDE_CLOSE, (_event, sessionId: string) =>
    claudemonSessionClient.close(sessionId),
  );
  ipcMain.handle(IPC.CLAUDE_ATTACH, (_event, paneId: string, sessionId: string) =>
    claudemonSessionClient.attach(paneId, sessionId, IPC.CLAUDE_PORT),
  );
  ipcMain.handle(IPC.CLAUDE_DETACH, (_event, paneId: string) =>
    claudemonSessionClient.detach(paneId),
  );
  ipcMain.handle(IPC.CLAUDE_GATE, (_event, sessionId: string, on: boolean) =>
    claudemonSessionClient.setGate(sessionId, on),
  );

  ipcMain.handle(
    IPC.CLAUDE_SESSION_GET,
    (_event, sessionId: string): ClaudeSessionSnapshot | null =>
      claudeSessionStore.getSnapshot(sessionId),
  );

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
  ipcMain.handle(
    IPC.CHROME_COOKIES_IMPORT,
    async (
      _e,
      opts?: { domainFilter?: string[]; method?: 'cdp' | 'direct'; browser?: 'chrome' | 'edge' },
    ) => {
      const method = opts?.method ?? 'cdp';
      try {
        if (method === 'cdp') {
          return await importChromeCookiesViaCDP({
            domainFilter: opts?.domainFilter,
            browser: opts?.browser ?? 'chrome',
          });
        }
        return await importChromeCookies({ domainFilter: opts?.domainFilter });
      } catch (err: any) {
        return { imported: 0, skipped: 0, errors: [err?.message ?? String(err)] };
      }
    },
  );

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
  ipcMain.handle(IPC.ANALYTICS_SUMMARY, (_event, provider?: string, since?: string) =>
    sessionHistory.summary(provider, since),
  );
  ipcMain.handle(
    IPC.ANALYTICS_RECENT,
    (_event, limit?: number, provider?: string, since?: string) =>
      sessionHistory.recent(limit, provider, since),
  );

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

  // Open a file with the OS default handler (browser for .html) via a file://
  // URL, and reveal a file in the OS file manager. Both validate existence
  // first — shell.openExternal on a missing path fails silently on some
  // platforms, which reads as a dead click.
  ipcMain.handle(IPC.FILE_OPEN_EXTERNAL, async (_event, filePath: string) => {
    try {
      await fs.promises.access(filePath);
    } catch {
      return { ok: false, error: `File not found: ${filePath}` };
    }
    try {
      await shell.openExternal(pathToFileURL(filePath).toString());
      return { ok: true };
    } catch (err) {
      return { ok: false, error: err instanceof Error ? err.message : String(err) };
    }
  });
  // Open an http(s) URL in the OS default browser (e.g. a Tailscale opt-in link
  // from the Remote Share dialog). Scheme-checked so this can't be coaxed into
  // launching file:// or arbitrary custom-protocol handlers.
  ipcMain.handle(IPC.SHELL_OPEN_EXTERNAL, async (_event, url: string) => {
    let parsed: URL;
    try {
      parsed = new URL(url);
    } catch {
      return { ok: false, error: 'Invalid URL' };
    }
    if (parsed.protocol !== 'http:' && parsed.protocol !== 'https:') {
      return { ok: false, error: `Refusing to open ${parsed.protocol} URL` };
    }
    try {
      await shell.openExternal(parsed.toString());
      return { ok: true };
    } catch (err) {
      return { ok: false, error: err instanceof Error ? err.message : String(err) };
    }
  });
  ipcMain.handle(IPC.FILE_SHOW_IN_FOLDER, async (_event, filePath: string) => {
    try {
      await fs.promises.access(filePath);
    } catch {
      return { ok: false, error: `File not found: ${filePath}` };
    }
    shell.showItemInFolder(filePath);
    return { ok: true };
  });

  // Watch a single file for external changes. Changes are pushed back via the
  // FILE_CHANGED channel (and the hub bus) by the sink installed above.
  ipcMain.handle(IPC.FILE_WATCH, (_event, filePath: string) => {
    startWatch(filePath);
  });
  ipcMain.handle(IPC.FILE_UNWATCH, (_event, filePath: string) => {
    stopWatch(filePath);
  });

  // Project-wide search (editor search sidebar), backed by ripgrep.
  ipcMain.handle(IPC.SEARCH_PROJECT, (_event, opts: Parameters<typeof searchProject>[0]) =>
    searchProject(opts),
  );

  // ── Git (review pane) ── shells out to `git`; same backend as the git.*
  // hub capabilities, so the desktop reaches it whether it's on IPC or the bus.
  ipcMain.handle(IPC.GIT_STATUS, (_event, cwd: string) => git.status(cwd));
  ipcMain.handle(IPC.GIT_LOG, (_event, cwd: string, limit?: number) => git.log(cwd, limit));
  ipcMain.handle(
    IPC.GIT_DIFF,
    (_event, cwd: string, path?: string, staged?: boolean, untracked?: boolean) =>
      git.diff(cwd, path, staged, untracked),
  );
  ipcMain.handle(IPC.GIT_NUMSTAT, (_event, cwd: string, staged?: boolean) =>
    git.numstat(cwd, staged),
  );
  ipcMain.handle(IPC.GIT_STAGE, (_event, cwd: string, path?: string) => git.stage(cwd, path));
  ipcMain.handle(IPC.GIT_UNSTAGE, (_event, cwd: string, path?: string) => git.unstage(cwd, path));
  ipcMain.handle(IPC.GIT_COMMIT, (_event, cwd: string, message: string) =>
    git.commit(cwd, message),
  );
  ipcMain.handle(IPC.GIT_PUSH, (_event, cwd: string) => git.push(cwd));

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
    listClaudeSessionsForDir(cwd),
  );

  ipcMain.handle(IPC.CLAUDE_PROFILES_LIST, () => claudeProfiles.getProfiles());
  ipcMain.handle(
    IPC.CLAUDE_PROFILES_ADD,
    (_event, name: string, configDir: string, extraArgs: string[], mcpItemIds?: string[]) =>
      claudeProfiles.addProfile(name, configDir, extraArgs, mcpItemIds ?? []),
  );
  ipcMain.handle(IPC.CLAUDE_PROFILES_UPDATE, (_event, id: string, updates: ProfileUpdate) =>
    claudeProfiles.updateProfile(id, updates),
  );
  ipcMain.handle(IPC.CLAUDE_PROFILES_REMOVE, (_event, id: string) =>
    claudeProfiles.removeProfile(id),
  );
}
