import { ipcMain, BrowserWindow, dialog } from 'electron';
import * as os from 'os';
import * as fs from 'fs';
import { configService } from './services/configService';
import { sessionService } from './services/sessionService';
import { claudeSessionStore } from './services/claudeSessionStore';
import { claudemonSessionClient } from './services/claudemonSessionClient';
import { buildClaudeArgv } from './services/claudeResolver';
import { importChromeCookies, importChromeCookiesViaCDP } from './services/chromeCookieImport';
import { trackerService } from './services/tracker/trackerService';
import { issueCache } from './services/db';
import { devopsService } from './services/devops/devopsService';
import { claudeProfiles } from './services/claudeProfiles';
import { listClaudeSessionsForDir } from './services/claudeSessionList';

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

  // ── Generic terminal (non-Claude shells) — routed through claudemon ──
  ipcMain.handle('terminal:create', async (_event, shell: string, cwd?: string, cols?: number, rows?: number) => {
    try {
      const resolvedShell = shell || detectDefaultShell();
      const resolvedCwd = cwd && fs.existsSync(cwd) ? cwd : os.homedir();
      return claudemonSessionClient.spawn({
        argv: [resolvedShell],
        cwd: resolvedCwd,
        cols,
        rows,
        portChannel: 'terminal:port',
      });
    } catch (err: any) {
      console.error('[IPC] terminal:create failed:', err?.message);
      throw err;
    }
  });

  ipcMain.handle('terminal:resize', (_event, id: string, cols: number, rows: number) =>
    claudemonSessionClient.resize(id, cols, rows));
  ipcMain.handle('terminal:close', (_event, id: string) =>
    claudemonSessionClient.close(id));

  // ── Claude sessions (delegated to claudemon) ──
  ipcMain.handle('claude:spawn', async (_event, opts: { cwd?: string; profileId?: string; resumeSessionId?: string; cols?: number; rows?: number }) => {
    const profile = opts.profileId ? claudeProfiles.getProfile(opts.profileId) : undefined;
    const env: Record<string, string> = {};
    if (profile?.configDir) {
      env.CLAUDE_CONFIG_DIR = profile.configDir.replace(/^~/, os.homedir());
    }
    const argv = buildClaudeArgv({
      extraArgs: profile?.extraArgs,
      resumeSessionId: opts.resumeSessionId,
    });
    const cwd = opts.cwd ?? process.env.HOME ?? os.homedir();
    return claudemonSessionClient.spawn({ argv, cwd, cols: opts.cols, rows: opts.rows, env });
  });

  ipcMain.handle('claude:message', (_event, sessionId: string, text: string) =>
    claudemonSessionClient.message(sessionId, text));
  ipcMain.handle('claude:approve', (_event, sessionId: string, decision: 'yes' | 'no' | 'always', reason?: string) =>
    claudemonSessionClient.approve(sessionId, decision, reason));
  ipcMain.handle('claude:answer', (_event, sessionId: string, payload: { option?: number; text?: string; answers?: string[] }) =>
    claudemonSessionClient.answer(sessionId, payload));
  ipcMain.handle('claude:resize', (_event, sessionId: string, cols: number, rows: number) =>
    claudemonSessionClient.resize(sessionId, cols, rows));
  ipcMain.handle('claude:signal', (_event, sessionId: string, signal: string) =>
    claudemonSessionClient.signal(sessionId, signal));
  ipcMain.handle('claude:close', (_event, sessionId: string) =>
    claudemonSessionClient.close(sessionId));
  ipcMain.handle('claude:attach', (_event, paneId: string, sessionId: string) =>
    claudemonSessionClient.attach(paneId, sessionId, 'claude:port'));
  ipcMain.handle('claude:detach', (_event, paneId: string) =>
    claudemonSessionClient.detach(paneId));
  ipcMain.handle('claude:gate', (_event, sessionId: string, on: boolean) =>
    claudemonSessionClient.setGate(sessionId, on));

  ipcMain.handle('claude-session:get', (_event, sessionId: string) =>
    claudeSessionStore.getSnapshot(sessionId));

  ipcMain.handle('claude-session:getAll', () => {
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
  ipcMain.handle('chrome-cookies:import', async (_e, opts?: { domainFilter?: string[]; method?: 'cdp' | 'direct'; browser?: 'chrome' | 'edge' }) => {
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
  ipcMain.handle('config:get', () => {
    return configService.getConfig();
  });

  ipcMain.handle('config:reload', () => {
    return configService.reloadConfig();
  });

  ipcMain.handle('config:getPath', () => {
    return configService.getConfigPath();
  });

  ipcMain.handle('config:save', (_event, partial: unknown) => {
    return configService.saveConfig(partial as any);
  });

  // Session handlers
  ipcMain.handle('session:list', () => {
    return sessionService.listSessions();
  });

  ipcMain.handle('session:load', (_event, filename: string) => {
    return sessionService.loadSession(filename);
  });

  ipcMain.handle('session:save', (_event, data: any) => {
    // Enrich terminal panes with CWD within tabs
    const ptyMapping = data.ptyMapping || {};
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

  ipcMain.handle('session:delete', (_event, filename: string) => {
    sessionService.deleteSession(filename);
  });

  // App info
  ipcMain.handle('app:getCwd', () => {
    return process.cwd();
  });

  // Dialog
  ipcMain.handle('dialog:pickFolder', async (_event, defaultPath?: string) => {
    const result = await dialog.showOpenDialog(mainWindow, {
      title: 'Choose working directory for Claude',
      defaultPath: defaultPath || process.cwd(),
      properties: ['openDirectory'],
    });
    if (result.canceled || result.filePaths.length === 0) return null;
    return result.filePaths[0];
  });

  ipcMain.handle('dialog:pickFiles', async (_event, defaultPath?: string) => {
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

  ipcMain.handle('claude-sessions:listForDir', (_event, cwd: string) =>
    listClaudeSessionsForDir(cwd));

  ipcMain.handle('claude-profiles:list', () => claudeProfiles.getProfiles());
  ipcMain.handle('claude-profiles:add', (_event, name: string, configDir: string, extraArgs: string[]) =>
    claudeProfiles.addProfile(name, configDir, extraArgs));
  ipcMain.handle('claude-profiles:update', (_event, id: string, updates: any) =>
    claudeProfiles.updateProfile(id, updates));
  ipcMain.handle('claude-profiles:remove', (_event, id: string) =>
    claudeProfiles.removeProfile(id));

  // ── Issue Tracker ──

  ipcMain.handle('tracker:getProviders', () => trackerService.getProviderList());

  ipcMain.handle('tracker:getAccounts', () => trackerService.getAccounts());

  ipcMain.handle('tracker:addAccount', async (_event, provider: string, label: string, config: Record<string, string>, token: string) =>
    trackerService.addAccount(provider, label, config, token),
  );

  ipcMain.handle('tracker:updateAccount', async (_event, accountId: string, updates: any) =>
    trackerService.updateAccount(accountId, updates),
  );

  ipcMain.handle('tracker:removeAccount', (_event, accountId: string) => {
    trackerService.removeAccount(accountId);
  });

  ipcMain.handle('tracker:listProjects', async (_event, accountId: string) =>
    trackerService.listProjects(accountId),
  );

  ipcMain.handle('tracker:listIssues', async (_event, accountId: string, options: any) =>
    trackerService.listIssues(accountId, options),
  );

  ipcMain.handle('tracker:getIssue', async (_event, accountId: string, issueKey: string) =>
    trackerService.getIssue(accountId, issueKey),
  );

  ipcMain.handle('tracker:searchIssues', async (_event, accountId: string, query: string) =>
    trackerService.searchIssues(accountId, query),
  );

  ipcMain.handle('tracker:resolveIssueKey', async (_event, issueKey: string) =>
    trackerService.resolveIssueKey(issueKey),
  );

  ipcMain.handle('tracker:getTransitions', async (_event, accountId: string, issueKey: string) =>
    trackerService.getTransitions(accountId, issueKey),
  );

  ipcMain.handle('tracker:transitionIssue', async (_event, accountId: string, issueKey: string, transitionId: string) =>
    trackerService.transitionIssue(accountId, issueKey, transitionId),
  );

  // ── Cached queries (SQLite) ──

  ipcMain.handle('cache:getIssueLinks', (_event, issueKey: string) =>
    issueCache.getIssueLinks(issueKey),
  );

  ipcMain.handle('cache:getChildIssues', (_event, parentKey: string) =>
    issueCache.getChildIssues(parentKey),
  );

  ipcMain.handle('cache:searchIssues', (_event, query: string) =>
    issueCache.searchIssues(query),
  );

  ipcMain.handle('cache:recentPipelines', (_event, limit?: number) =>
    issueCache.getRecentPipelines(limit),
  );

  ipcMain.handle('cache:recentPRs', (_event, limit?: number) =>
    issueCache.getRecentPullRequests(limit),
  );

  // ── DevOps (Git + CI/CD) ──

  ipcMain.handle('devops:getProviders', () => devopsService.getProviderList());
  ipcMain.handle('devops:getAccounts', () => devopsService.getAccounts());
  ipcMain.handle('devops:addAccount', async (_event, provider: string, label: string, config: Record<string, string>, token: string) =>
    devopsService.addAccount(provider, label, config, token));
  ipcMain.handle('devops:removeAccount', (_event, accountId: string) => devopsService.removeAccount(accountId));
  ipcMain.handle('devops:listRepos', async (_event, accountId: string) => devopsService.listRepos(accountId));
  ipcMain.handle('devops:listPRs', async (_event, accountId: string, options?: any) => devopsService.listPullRequests(accountId, options));
  ipcMain.handle('devops:listPipelines', async (_event, accountId: string, options?: any) => devopsService.listPipelines(accountId, options));
}
