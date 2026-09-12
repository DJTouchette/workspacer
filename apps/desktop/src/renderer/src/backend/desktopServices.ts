import type { ElectronAPI } from '../types/electron';
import { requestOpenInEditor } from '../lib/editorBus';

type Services = Pick<ElectronAPI, 'sessionGrantReconcile' |
  'worktreeInfo' | 'worktreeCreate' | 'worktreeRemove' | 'pricingGetRates' |
  'pricingSaveOverrides' | 'claudeProfilesAccounts' | 'claudeProfilesLoginStatus' |
  'claudeProfilesAddAccount' | 'toolsStatus' | 'fleetReviewRead' | 'fleetReviewForget' |
  'taskInspectorEdit' | 'taskInspectorOpen' | 'dispatchHistoryRead' | 'htmlCardReadDiff' |
  'loadBriefBoard' | 'moveBriefCard' | 'fleetWorkflowRequest' | 'managerRequestPrepare' |
  'claudeProfilesAdd' | 'claudeProfilesUpdate' | 'claudeProfilesRemove' | 'saveConfig' |
  'agentSuggestTitle' | 'providerReadiness' | 'agentRuntimeStatus' | 'keepWarmHeartbeats'>;

/** The selected server executes the same services as native IPC. */
export function desktopServices(call: <T>(method: string, params: unknown, timeout?: number) => Promise<T>): Services {
  return {
    sessionGrantReconcile: (sessionId,role) => call('desktop.sessionGrantReconcile', {sessionId,role}),
    agentSuggestTitle: (request) => call('desktop.agentSuggestTitle', { request }, 30_000),
    providerReadiness: (provider, check) => call('desktop.providerReadiness', { provider, check }, 60_000),
    agentRuntimeStatus: () => call('desktop.agentRuntimeStatus', {}),
    keepWarmHeartbeats: (limit) => call('desktop.keepWarmHeartbeats', { limit }),
    saveConfig: (partial) => call('desktop.saveConfig', { partial }),
    claudeProfilesAdd: (name, configDir, extraArgs, mcpItemIds, init) => call('desktop.claudeProfilesAdd', { name, configDir, extraArgs, mcpItemIds, init }),
    claudeProfilesUpdate: (id, updates) => call('desktop.claudeProfilesUpdate', { id, updates }),
    claudeProfilesRemove: (id) => call('desktop.claudeProfilesRemove', { id }),
    loadBriefBoard: () => call('desktop.loadBriefBoard', {}),
    moveBriefCard: (request) => call('desktop.moveBriefCard', { request }),
    fleetWorkflowRequest: (request) => call('desktop.fleetWorkflowRequest', { request }),
    managerRequestPrepare: (sessionId, text, bootstrap) => call('desktop.managerRequestPrepare', { sessionId, text, bootstrap }),
    worktreeInfo: (cwd) => call('desktop.worktreeInfo', { cwd }),
    worktreeCreate: (opts) => call('desktop.worktreeCreate', opts, 6 * 60_000),
    worktreeRemove: (cwd) => call('desktop.worktreeRemove', { cwd }),
    pricingGetRates: () => call('desktop.pricingGetRates', {}),
    pricingSaveOverrides: (overrides) => call('desktop.pricingSaveOverrides', { overrides }),
    claudeProfilesAccounts: () => call('desktop.claudeProfilesAccounts', {}),
    claudeProfilesLoginStatus: () => call('desktop.claudeProfilesLoginStatus', {}),
    claudeProfilesAddAccount: (name) => call('desktop.claudeProfilesAddAccount', { name }),
    toolsStatus: () => call('desktop.toolsStatus', {}),
    fleetReviewRead: (request) => call('desktop.fleetReviewRead', { request }),
    fleetReviewForget: (request) => call('desktop.fleetReviewForget', { request }),
    taskInspectorEdit: (request) => call('desktop.taskInspectorEdit', { request }),
    taskInspectorOpen: async (request) => {
      try {
        const result = await call<{ ok: boolean; kind?: 'url' | 'worktree'; target?: string; error?: string }>('desktop.taskInspectorOpen', { request });
        if (!result.ok || !result.target) return { ok: false, error: result.error ?? 'Target unavailable' };
        if (result.kind === 'url') window.open(result.target, '_blank', 'noopener,noreferrer');
        else requestOpenInEditor({ directory: result.target });
        return { ok: true };
      } catch (error) { return { ok: false, error: error instanceof Error ? error.message : 'Could not open target' }; }
    },
    dispatchHistoryRead: () => call('desktop.dispatchHistoryRead', {}),
    htmlCardReadDiff: (target, ownerId) => call('desktop.htmlCardReadDiff', { target, ownerId }),
  };
}
