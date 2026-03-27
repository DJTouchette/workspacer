import { ipcMain, BrowserWindow, dialog } from 'electron';
import { terminalService } from './services/terminalService';
import { configService } from './services/configService';
import { sessionService } from './services/sessionService';
import { claudeSessionStore } from './services/claudeSessionStore';
import { trackerService } from './services/tracker/trackerService';
import { issueCache } from './services/db';
import { backgroundSync } from './services/tracker/backgroundSync';
import { devopsService } from './services/devops/devopsService';

export function registerIpcHandlers(mainWindow: BrowserWindow): void {
  // Wire terminal service to use this window for push events
  terminalService.setMainWindow(mainWindow);

  // Terminal handlers
  ipcMain.handle('terminal:create', (_event, shell: string, cwd?: string, cols?: number, rows?: number) => {
    try {
      return terminalService.createTerminal(shell, cwd, cols, rows);
    } catch (err: any) {
      console.error('[IPC] terminal:create failed:', err?.message);
      throw err;
    }
  });

  // Claude terminal — spawns claude CLI with headless mirroring
  ipcMain.handle('terminal:createClaude', (_event, cwd?: string) => {
    try {
      return terminalService.createClaudeTerminal(cwd);
    } catch (err: any) {
      console.error('[IPC] terminal:createClaude failed:', err?.message);
      throw err;
    }
  });

  // Claude session state queries
  ipcMain.handle('claude-session:getByPty', (_event, ptyId: string) => {
    return claudeSessionStore.getSnapshotByPty(ptyId);
  });

  ipcMain.handle('claude-session:getAll', () => {
    return claudeSessionStore.getAllSnapshots();
  });

  // terminal:write removed — writes go through MessagePort now

  ipcMain.handle('terminal:resize', (_event, id: string, cols: number, rows: number) => {
    terminalService.resizeTerminal(id, cols, rows);
  });

  ipcMain.handle('terminal:close', (_event, id: string) => {
    terminalService.closeTerminal(id);
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

  ipcMain.handle('cache:syncNow', async () => {
    await backgroundSync.syncAll();
  });

  ipcMain.handle('cache:watchRepo', (_event, repoPath: string) => {
    backgroundSync.watchRepo(repoPath);
  });

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
