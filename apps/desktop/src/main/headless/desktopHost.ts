import { createWorkflowTelemetry } from '../services/workflowTelemetryCore';
import { listPickerEntries, readFileBytes } from './files';
/** Shared desktop services hosted inside Electron. Host-owned context supplies
 * roots and live snapshots; browser parameters cannot override that context.
 */
import * as path from 'node:path';
import * as fs from 'node:fs';
import * as os from 'node:os';
import { configService } from '../services/configService';
import {
  assertPathAllowed,
  assertPathContained,
  canonicalRoot,
  containsCanonical,
} from '../lib/pathConfinement';
import {
  worktreeInfo,
  createWorktree,
  removeAgentWorktree,
  defaultWorktreeRoot,
} from '../services/worktreeService';
import {
  MODEL_RATES,
  readModelRateOverrides,
  writeModelRateOverrides,
  type ModelRateOverrides,
} from '../services/modelUsage';
import { windowFor } from '../shared/modelContextWindows';
import { claudeProfiles } from '../services/claudeProfiles';
import { createAccountConfigDir } from '../services/claudeAccountSetup';
import { profileAccount, profileSignedIn } from '../lib/profileAccounts';
import { toolsStatus } from '../services/toolCheck';
import { fleetReviewStore } from '../services/fleetReviewStore';
import { dispatchHistoryStore } from '../services/dispatchHistoryStore';
import { readHtmlCardDiff } from '../services/gitService';
import type { ClaudeSessionSnapshot } from '../shared/ipcTypes';
import type { TaskEditRequest, TaskOpenRequest } from '../shared/dispatchHistory';
import { createFleetWorkflowRuntime } from '../services/fleetWorkflowCore';
import { createFleetWorkflowService } from '../services/fleetWorkflowServiceCore';
import { dispatchTemplateParams } from '../lib/dispatchTemplate';
import { managerRequests } from '../services/managerRequestService';
import {
  loadBoard,
  applyBoardMove,
  setBoardRecentSessions,
  type BoardMoveRequest,
} from '../services/briefBoardCore';
import type { WorkflowTemplate, WorkflowRequest } from '../shared/fleetWorkflow';
import { generateAgentTitle, type TitleRequest } from '../services/agentTitler';
import { configureCompletionDaemonURL, completeReadinessPing } from '../services/directCompletion';
import { ProviderReadinessService } from '../services/providerReadiness';
import { checkAllProviders } from '../services/agentProviders';
import { resolveCodexReadinessBinary } from '../services/codexReadinessBinary';
import { resolveTransport } from '../lib/spawnTransport';
import { workflowWatcher } from '../services/workflowWatcher';
import { listUIFonts, readUIAsset, installUIFont, installProjectIcon } from './uiAssets';

export interface HostContext {
  workspaceRoots: string[];
  setupRoots: string[];
  snapshots: Array<
    ClaudeSessionSnapshot & {
      isWakeTarget?: boolean;
      startedAt?: number;
      resultSchema?: Record<string, unknown>;
    }
  >;
  templates?: Array<{
    id: string;
    body: string;
    kind?: string;
    scope?: string;
    resultSchema?: Record<string, unknown>;
  }>;
  recent?: Array<{ cwd: string }>;
  daemonURL?: string;
}
type Params = Record<string, unknown>;
let currentContext: HostContext = { workspaceRoots: [], setupRoots: [], snapshots: [] };
let templates: WorkflowTemplate[] = [];
let nativeSnapshot: ((id: string) => HostContext['snapshots'][number] | undefined) | undefined;
const currentSnapshot = (id: string) =>
  nativeSnapshot ? nativeSnapshot(id) : currentContext.snapshots.find((s) => s.sessionId === id);
let workflow: ReturnType<typeof createFleetWorkflowRuntime>;
let workflows: ReturnType<typeof createFleetWorkflowService>;
/** Native hosting reuses its already-running controllers and ownership store. */
export function configureNativeDesktopRuntime(
  runtime: typeof workflow,
  service: typeof workflows,
  snapshot: NonNullable<typeof nativeSnapshot>,
): void {
  workflow = runtime;
  workflows = service;
  nativeSnapshot = snapshot;
}
function ensureRuntime(): void {
  if (workflow) return;
  workflow = createFleetWorkflowRuntime(currentSnapshot);
  workflows = createFleetWorkflowService(workflow, currentSnapshot, () => templates);
  setBoardRecentSessions(
    () => currentContext.recent ?? currentContext.snapshots.map((s) => ({ cwd: s.cwd })),
  );
}
const readiness = new ProviderReadinessService({
  context: (selected) => {
    const cfg = configService.getConfig();
    const provider = selected ?? cfg.agents?.managerProvider ?? 'claude';
    const local =
      !!currentContext.daemonURL &&
      ['127.0.0.1', 'localhost', '[::1]'].includes(new URL(currentContext.daemonURL).hostname) &&
      (provider !== 'claude' || resolveTransport('claude', undefined, cfg) === 'stream');
    const configured = local
      ? (checkAllProviders(cfg.agents?.binaries).find((r) => r.provider === provider)
          ?.resolvedPath ?? null)
      : null;
    const bin =
      provider === 'codex' && configured
        ? (resolveCodexReadinessBinary(configured) ?? configured)
        : configured;
    return {
      provider,
      bin,
      local,
      enabled: cfg.agents?.checkProviderOnStartup !== false,
      key: JSON.stringify([
        local,
        currentContext.daemonURL,
        provider,
        bin,
        cfg.agents,
        cfg.claude,
        cfg.codex,
      ]),
    };
  },
  ping: completeReadinessPing,
});
let readinessStarted = false;
const emit: (event: string, data: unknown) => void = () => {};
const workflowTelemetry = createWorkflowTelemetry((event) => emit('workflow.event', event));
const workflowWatching = new Map<string, string>();
const artifactID = /^[A-Za-z0-9_-]{1,128}$/;
function watchWorkflow(sessionId: string): string | undefined {
  const session = currentSnapshot(sessionId);
  if (!session || !artifactID.test(sessionId)) return;
  const known = workflowWatching.get(sessionId);
  if (known) return known;
  const roots = [
    process.env.CLAUDE_CONFIG_DIR || path.join(os.homedir(), '.claude'),
    ...claudeProfiles
      .getProfiles()
      .filter((p) => !p.provider || p.provider === 'claude')
      .map((p) => p.configDir.replace(/^~/, os.homedir()))
      .filter(Boolean),
  ].map((root) => path.join(root, 'projects'));
  const row = session as unknown as Record<string, unknown>;
  const candidates = [row.transcriptPath, row.transcript_path].filter(
    (p): p is string => typeof p === 'string' && p.endsWith('.jsonl'),
  );
  for (const root of roots) {
    try {
      for (const dir of fs.readdirSync(root, { withFileTypes: true }))
        if (dir.isDirectory()) candidates.push(path.join(root, dir.name, `${sessionId}.jsonl`));
    } catch {
      /* This profile may not have any Claude transcripts. */
    }
  }
  for (const candidate of candidates) {
    try {
      const checked = assertPathContained('desktop.workflowAgentConversation', candidate, roots);
      if (!fs.statSync(checked).isFile()) continue;
      workflowWatcher.attach(sessionId, checked, (update) => {
        emit('workflow.update', { sessionId, ...update });
        workflowTelemetry.publishWorkflowRuns({ sessionId, cwd: session.cwd }, update.runs);
      });
      workflowWatching.set(sessionId, checked);
      workflowWatcher.refresh(sessionId);
      return checked;
    } catch {
      /* Try another authorized transcript root. */
    }
  }
}
function text(params: Params, key: string, optional = false): string {
  const value = params[key];
  if (optional && value == null) return '';
  if (typeof value !== 'string' || (!optional && !value) || value.length > 64 * 1024)
    throw new Error(`Invalid ${key}`);
  return value;
}
function configuredWorktreeRoot(): string {
  return (
    (configService.getConfig().agents as { worktreeRoot?: string })?.worktreeRoot?.trim() ||
    defaultWorktreeRoot()
  );
}
function liveOwner(context: HostContext, id: string) {
  const owner = context.snapshots.find((s) => s.sessionId === id && s.status !== 'ended' && !s.hub);
  if (!owner) throw new Error('Owning session is no longer available on this server');
  return owner;
}

export async function desktopHostCall(
  method: string,
  params: Params,
  context: HostContext,
): Promise<unknown> {
  currentContext = context;
  ensureRuntime();
  if (context.daemonURL) configureCompletionDaemonURL(context.daemonURL);
  if (context.templates) {
    const convert = (t: NonNullable<HostContext['templates']>[number]): WorkflowTemplate => ({
      id: t.id,
      body: t.body,
      resultSchema: t.resultSchema ?? {},
      params: dispatchTemplateParams(t.body),
    });
    templates = context.templates
      .filter((t) => t.kind === 'dispatch' && t.scope === 'global')
      .map(convert);
  }
  const snapshot = (id: string) => context.snapshots.find((s) => s.sessionId === id);
  switch (method) {
    case 'desktop.filePickerList':
      return listPickerEntries(params.path);
    case 'desktop.readFileBytes':
      return readFileBytes(params.path, context.workspaceRoots);
    case 'ui.fonts':
      return listUIFonts();
    case 'ui.asset':
      return readUIAsset(params.kind, params.file);
    case 'desktop.installUiFont':
      return installUIFont(params.name, params.dataBase64);
    case 'desktop.downloadProjectIcon':
      return installProjectIcon(params.url);
    case 'desktop.fleetWorkflowRequest':
      return workflows.fleetWorkflowRequest(params.request as WorkflowRequest);
    case 'desktop.managerRequestPrepare':
      return managerRequests().prepare(
        text(params, 'sessionId'),
        text(params, 'text'),
        params.bootstrap === true,
      );
    case 'desktop.loadBriefBoard':
      return loadBoard();
    case 'desktop.moveBriefCard':
      return applyBoardMove(params.request as unknown as BoardMoveRequest);
    case 'desktop.saveConfig': {
      if (!params.partial || typeof params.partial !== 'object' || Array.isArray(params.partial))
        throw new Error('Configuration patch must be an object');
      return configService.saveConfig(
        params.partial as Parameters<typeof configService.saveConfig>[0],
      );
    }
    case 'desktop.agentSuggestTitle':
      return generateAgentTitle(params.request as unknown as TitleRequest);
    case 'desktop.providerReadiness': {
      if (!readinessStarted) {
        readinessStarted = true;
        configService.onChange(() => readiness.invalidate());
        readiness.start();
      }
      const provider = text(params, 'provider');
      return params.check === true ? readiness.check(provider) : readiness.read(provider);
    }
    case 'desktop.workflowAgentTranscript':
    case 'desktop.workflowAgentConversation': {
      const sessionId = text(params, 'sessionId'),
        runId = text(params, 'runId'),
        agentId = text(params, 'agentId');
      if (![sessionId, runId, agentId].every((id) => artifactID.test(id)))
        throw new Error('Invalid workflow identity');
      const transcript = watchWorkflow(sessionId);
      if (!transcript) return null;
      const root = transcript.replace(/\.jsonl$/i, '');
      const target = path.join(
        root,
        'subagents',
        'workflows',
        `wf_${runId}`,
        `agent-${agentId.replace(/^agent-/, '')}.jsonl`,
      );
      try {
        assertPathContained(method, target, [root]);
      } catch {
        return null;
      }
      workflowWatcher.refresh(sessionId);
      return method === 'desktop.workflowAgentTranscript'
        ? workflowWatcher.readAgentTranscript(sessionId, runId, agentId)
        : workflowWatcher.readAgentConversation(sessionId, runId, agentId);
    }
    case 'desktop.worktreeInfo':
      return worktreeInfo(assertPathAllowed(method, text(params, 'cwd'), context.setupRoots));
    case 'desktop.worktreeCreate': {
      const repoCwd = assertPathAllowed(method, text(params, 'repoCwd'), context.setupRoots);
      const root = configuredWorktreeRoot();
      const requested = text(params, 'rootOverride', true);
      if (requested && path.resolve(requested) !== path.resolve(root))
        throw new Error('Worktree destination must match the server configuration');
      return createWorktree({
        repoCwd,
        name: text(params, 'name', true),
        rootOverride: root,
        config: configService.getConfig(),
      });
    }
    case 'desktop.worktreeRemove': {
      const root = canonicalRoot(configuredWorktreeRoot());
      const cwd = canonicalRoot(text(params, 'cwd'));
      if (!root || !cwd || cwd === root || !containsCanonical(root, cwd))
        return { ok: false, skipped: true };
      if (context.snapshots.some((s) => s.status !== 'ended' && (s.liveCwd || s.cwd) === cwd))
        return { ok: false, skipped: true, error: 'Worktree still has a live agent' };
      return removeAgentWorktree({ cwd, rootOverride: root });
    }
    case 'desktop.pricingGetRates':
      return {
        defaults: Object.fromEntries(
          Object.entries(MODEL_RATES).map(([prefix, rate]) => [
            prefix,
            { ...rate, contextLimit: windowFor(prefix) },
          ]),
        ),
        overrides: readModelRateOverrides(),
      };
    case 'desktop.pricingSaveOverrides': {
      const overrides = params.overrides;
      if (!overrides || typeof overrides !== 'object' || Array.isArray(overrides))
        throw new Error('Rate overrides must be an object');
      for (const row of Object.values(overrides)) {
        if (
          !row ||
          typeof row !== 'object' ||
          ['input', 'output'].some(
            (key) => typeof row[key] !== 'number' || !Number.isFinite(row[key]) || row[key] < 0,
          )
        )
          throw new Error('Rates must have nonnegative finite input and output values');
        for (const key of ['cached_input', 'context_limit'])
          if (
            row[key] !== undefined &&
            (typeof row[key] !== 'number' || !Number.isFinite(row[key]) || row[key] < 0)
          )
            throw new Error(`Invalid ${key}`);
      }
      writeModelRateOverrides(overrides as ModelRateOverrides);
      return { ok: true };
    }
    case 'desktop.claudeProfilesAccounts':
      return Object.fromEntries(claudeProfiles.getProfiles().map((p) => [p.id, profileAccount(p)]));
    case 'desktop.claudeProfilesLoginStatus':
      return Object.fromEntries(
        claudeProfiles.getProfiles().map((p) => [p.id, profileSignedIn(p)]),
      );
    case 'desktop.claudeProfilesAddAccount': {
      const name = text(params, 'name', true).trim() || 'Account';
      const { dir, shared, warnings } = createAccountConfigDir(name);
      return { profile: claudeProfiles.addProfile(name, dir, [], []), shared, warnings };
    }
    case 'desktop.claudeProfilesAdd':
      return claudeProfiles.addProfile(
        text(params, 'name'),
        text(params, 'configDir', true),
        (params.extraArgs ?? []) as string[],
        (params.mcpItemIds ?? []) as string[],
        params.init as Parameters<typeof claudeProfiles.addProfile>[4],
      );
    case 'desktop.claudeProfilesUpdate': {
      const result = claudeProfiles.updateProfile(
        text(params, 'id'),
        params.updates as Parameters<typeof claudeProfiles.updateProfile>[1],
      );
      if (!result) throw new Error('Profile not found');
      return result;
    }
    case 'desktop.claudeProfilesRemove':
      claudeProfiles.removeProfile(text(params, 'id'));
      return null;
    case 'desktop.toolsStatus':
      return toolsStatus(true);
    case 'desktop.fleetReviewRead':
      return fleetReviewStore.read(params.request);
    case 'desktop.fleetReviewForget':
      return fleetReviewStore.forget(params.request);
    case 'desktop.taskInspectorEdit':
      return dispatchHistoryStore.editByHostUser(
        params.request as TaskEditRequest,
        snapshot,
        (id) => workflow.workflowBusy.has(id),
      );
    case 'desktop.taskInspectorOpen':
      return { ok: true, ...dispatchHistoryStore.openTarget(params.request as TaskOpenRequest) };
    case 'desktop.dispatchHistoryRead': {
      const owners = context.snapshots
        .filter((s) => s.isWakeTarget && s.status !== 'ended' && !s.hub)
        .sort((a, b) => (b.startedAt ?? 0) - (a.startedAt ?? 0));
      return {
        available: true,
        currentOwnerSessionId: owners[0]?.sessionId,
        ...dispatchHistoryStore.readForHostUser(
          snapshot,
          owners.map((s) => s.sessionId),
        ),
      };
    }
    case 'desktop.htmlCardReadDiff': {
      const owner = liveOwner(context, text(params, 'ownerId'));
      const cwd = owner.liveCwd || owner.cwd;
      const result = await readHtmlCardDiff(text(params, 'target'), cwd);
      const current = currentSnapshot(owner.sessionId);
      return !current ||
        current.status === 'ended' ||
        current.hub ||
        (current.liveCwd || current.cwd) !== cwd
        ? { ok: false, error: 'Owning session changed while reading the diff' }
        : result;
    }
    default:
      throw new Error(`Unknown desktop service: ${method}`);
  }
}
