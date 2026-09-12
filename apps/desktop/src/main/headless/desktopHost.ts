import { createWorkflowTelemetry } from '../services/workflowTelemetryCore';
import { listPickerEntries, readFileBytes } from './files';
import { hostCall } from './hostBridge';
import { prepareLaunchIntegrationCore } from '../services/launchIntegrationCore';
import { readCodexProvider } from '../services/codexRouting';
/** Shared desktop services hosted by the brain's worker user, without Electron.
 * The private stdio envelope is built by the brain; browser params cannot supply
 * roots or live snapshots. Each public method has a fixed action and argument
 * schema, and filesystem operations use the desktop's canonical path guards.
 */
import * as path from 'node:path';
import * as fs from 'node:fs';
import * as os from 'node:os';
import { randomUUID } from 'node:crypto';
import { configService } from '../services/configService';
import { assertPathAllowed, canonicalRoot, containsCanonical } from '../lib/pathConfinement';
import { worktreeInfo, createWorktree, removeAgentWorktree, defaultWorktreeRoot } from '../services/worktreeService';
import { MODEL_RATES, readModelRateOverrides, writeModelRateOverrides, type ModelRateOverrides } from '../services/modelUsage';
import { windowFor } from '../shared/modelContextWindows';
import { claudeProfiles } from '../services/claudeProfiles';
import { createAccountConfigDir } from '../services/claudeAccountSetup';
import { profileAccount, profileSignedIn } from '../lib/profileAccounts';
import { toolsStatus } from '../services/toolCheck';
import { fleetReviewStore, type ReviewAllocation } from '../services/fleetReviewStore';
import { dispatchHistoryStore } from '../services/dispatchHistoryStore';
import { readHtmlCardDiff } from '../services/gitService';
import type { ClaudeSessionSnapshot } from '../shared/ipcTypes';
import type { TaskEditRequest, TaskOpenRequest } from '../shared/dispatchHistory';
import { createFleetWorkflowRuntime } from '../services/fleetWorkflowCore';
import { createFleetWorkflowService } from '../services/fleetWorkflowServiceCore';
import { dispatchTemplateParams, validateDispatchTemplateParams, renderDispatchTemplate } from '../lib/dispatchTemplate';
import { managerReplacementState } from '../services/managerReplacementState';
import { managerRequests } from '../services/managerRequestService';
import { buildManagerKickoff } from '../shared/managerDoctrine';
import { loadBoard, applyBoardMove, setBoardRecentSessions, type BoardMoveRequest } from '../services/briefBoardCore';
import type { WorkflowTemplate, WorkflowRequest } from '../shared/fleetWorkflow';
import { buildResultContract, checkResultSchema, readStructuredResult, extractResultBlock } from '../shared/structuredResult';
import { readWorkerEscalation } from '../shared/workerEscalation';
import { generateAgentTitle, type TitleRequest } from '../services/agentTitler';
import { configureCompletionDaemonURL, completeReadinessPing } from '../services/directCompletion';
import { ProviderReadinessService } from '../services/providerReadiness';
import { checkAllProviders } from '../services/agentProviders';
import { resolveCodexReadinessBinary } from '../services/codexReadinessBinary';
import { resolveTransport } from '../lib/spawnTransport';
import { workflowWatcher } from '../services/workflowWatcher';
import { ensureReplacementReady, replacementRequest, rememberManager, routeReplacementMessage, finishReplacementMessage } from './managerReplacement';
import { headlessAnalytics } from './analytics';
import { listUIFonts, readUIAsset, installUIFont, installProjectIcon } from './uiAssets';

export interface HostContext {
  workspaceRoots: string[];
  setupRoots: string[];
  snapshots: Array<ClaudeSessionSnapshot & { isWakeTarget?: boolean; startedAt?: number; resultSchema?: Record<string, unknown> }>;
  templates?: Array<{ id: string; body: string; kind?: string; scope?: string; resultSchema?: Record<string, unknown> }>;
  recent?: Array<{ cwd: string }>;
  daemonURL?: string;
  spawnCallId?: string;
  analyticsSnapshots?: Array<Record<string, unknown>>;
}
type Params = Record<string, unknown>;
type Admission = Parameters<typeof dispatchHistoryStore.accept>[0];
type PendingAdmission = { input: Omit<Admission, 'sessionId'>; review?: ReviewAllocation; resume?: boolean; finish: (value: unknown) => void; cancel: (error: Error) => void; settled: Promise<unknown> };
const admissions = new Map<string, PendingAdmission>();
const resultCommits = new Map<string,{expires:number;commit:()=>unknown}>();
const observed = new Map<string, string>();
const lifecycle = new Map<string, string>();
let currentContext: HostContext = { workspaceRoots: [], setupRoots: [], snapshots: [] };
let templates: WorkflowTemplate[] = [];
let dispatchTemplates: WorkflowTemplate[] = [];
let nativeSnapshot: ((id: string) => HostContext['snapshots'][number] | undefined) | undefined;
const currentSnapshot = (id: string) => nativeSnapshot ? nativeSnapshot(id) : currentContext.snapshots.find((s) => s.sessionId === id);
let workflow: ReturnType<typeof createFleetWorkflowRuntime>;
let workflows: ReturnType<typeof createFleetWorkflowService>;
/** Native hosting reuses its already-running controllers and ownership store. */
export function configureNativeDesktopRuntime(runtime: typeof workflow, service: typeof workflows, snapshot: NonNullable<typeof nativeSnapshot>): void {
  workflow = runtime; workflows = service; nativeSnapshot = snapshot;
}
function ensureRuntime(): void {
  if (workflow) return;
  workflow = createFleetWorkflowRuntime(currentSnapshot);
  workflows = createFleetWorkflowService(workflow, currentSnapshot, () => templates);
  setBoardRecentSessions(() => currentContext.recent ?? currentContext.snapshots.map((s) => ({ cwd: s.cwd })));
}
const readiness = new ProviderReadinessService({
  context: (selected) => {
    const cfg = configService.getConfig();
    const provider = selected ?? cfg.agents?.managerProvider ?? 'claude';
    const local = !!currentContext.daemonURL && ['127.0.0.1', 'localhost', '[::1]'].includes(new URL(currentContext.daemonURL).hostname) && (provider !== 'claude' || resolveTransport('claude', undefined, cfg) === 'stream');
    const configured = local ? checkAllProviders(cfg.agents?.binaries).find((r) => r.provider === provider)?.resolvedPath ?? null : null;
    const bin = provider === 'codex' && configured ? resolveCodexReadinessBinary(configured) ?? configured : configured;
    return { provider, bin, local, enabled: cfg.agents?.checkProviderOnStartup !== false, key: JSON.stringify([local,currentContext.daemonURL,provider,bin,cfg.agents,cfg.claude,cfg.codex]) };
  },
  ping: completeReadinessPing,
});
let readinessStarted = false;
let emit: (event: string, data: unknown) => void = () => {};
export function setDesktopEventSink(sink: typeof emit): void { emit = sink; }
const workflowTelemetry=createWorkflowTelemetry((event)=>emit('workflow.event',event));
const workflowWatching = new Map<string, string>();
const artifactID = /^[A-Za-z0-9_-]{1,128}$/;
function watchWorkflow(sessionId: string): string | undefined {
  const session = currentSnapshot(sessionId);
  if (!session || !artifactID.test(sessionId)) return;
  const known = workflowWatching.get(sessionId);
  if (known) return known;
  const roots = [process.env.CLAUDE_CONFIG_DIR || path.join(os.homedir(), '.claude'), ...claudeProfiles.getProfiles().filter((p) => !p.provider || p.provider === 'claude').map((p) => p.configDir.replace(/^~/, os.homedir())).filter(Boolean)].map((root) => path.join(root, 'projects'));
  const row = session as unknown as Record<string, unknown>;
  const candidates = [row.transcriptPath, row.transcript_path].filter((p): p is string => typeof p === 'string' && p.endsWith('.jsonl'));
  for (const root of roots) {
    try { for (const dir of fs.readdirSync(root, { withFileTypes: true })) if (dir.isDirectory()) candidates.push(path.join(root, dir.name, `${sessionId}.jsonl`)); } catch { /* This profile may not have any Claude transcripts. */ }
  }
  for (const candidate of candidates) {
    try {
      const checked = assertPathAllowed('desktop.workflowAgentConversation', candidate, roots);
      if (!fs.statSync(checked).isFile()) continue;
      workflowWatcher.attach(sessionId, checked, (update) => {emit('workflow.update',{sessionId,...update});workflowTelemetry.publishWorkflowRuns({sessionId,cwd:session.cwd},update.runs);});
      workflowWatching.set(sessionId, checked);
      workflowWatcher.refresh(sessionId);
      return checked;
    } catch { /* Try another authorized transcript root. */ }
  }
}
function text(params: Params, key: string, optional = false): string {
  const value = params[key];
  if (optional && value == null) return '';
  if (typeof value !== 'string' || (!optional && !value) || value.length > 64 * 1024) throw new Error(`Invalid ${key}`);
  return value;
}
function configuredWorktreeRoot(): string {
  return (configService.getConfig().agents as { worktreeRoot?: string })?.worktreeRoot?.trim() || defaultWorktreeRoot();
}
function liveOwner(context: HostContext, id: string) {
  const owner = context.snapshots.find((s) => s.sessionId === id && s.status !== 'ended' && !s.hub);
  if (!owner) throw new Error('Owning session is no longer available on this server');
  return owner;
}

export async function desktopHostCall(method: string, params: Params, context: HostContext): Promise<unknown> {
  currentContext = context;
  ensureRuntime();
  if (context.daemonURL) configureCompletionDaemonURL(context.daemonURL);
  if (context.templates) {
    const convert = (t: NonNullable<HostContext['templates']>[number]):WorkflowTemplate => ({id:t.id,body:t.body,resultSchema:t.resultSchema??{},params:dispatchTemplateParams(t.body)});
    templates=context.templates.filter(t=>t.kind==='dispatch'&&t.scope==='global').map(convert);
    dispatchTemplates=context.templates.filter(t=>t.kind==='dispatch').map(convert);
  }
  const snapshot = (id: string) => context.snapshots.find((s) => s.sessionId === id);
  switch (method) {
    case 'internal.prepareIntegration': {
      if (!context.spawnCallId) throw new Error('Launch integration requires an active owner-authorized bus spawn');
      const pluginId=text(params,'id');
      return prepareLaunchIntegrationCore(pluginId, params.launchContext as Parameters<typeof prepareLaunchIntegrationCore>[1], params.base as Parameters<typeof prepareLaunchIntegrationCore>[2], {
        list:()=>hostCall('launch.prepare',{callId:context.spawnCallId,pluginId,op:'describe'}),
        call:(_method,launchContext)=>hostCall('launch.prepare',{callId:context.spawnCallId,pluginId,op:'prepare',context:launchContext}),
        routing:readCodexProvider,
      });
    }
    case 'desktop.managerReplacement': return replacementRequest(params.request as Parameters<typeof replacementRequest>[0], params.bindings);
    case 'internal.rememberManager': rememberManager(text(params,'sessionId'),params.options as Parameters<typeof rememberManager>[1]); return {ok:true};
    case 'internal.routeMessage': await ensureReplacementReady(); return routeReplacementMessage(text(params,'sessionId'),text(params,'text'),params.sourceRequest as Parameters<typeof routeReplacementMessage>[2]);
    case 'internal.messageResult': finishReplacementMessage(text(params,'id'),text(params,'status')); return {ok:true};
    case 'internal.assertReplacementAvailable': managerReplacementState.assertAvailable(text(params,'source')); managerReplacementState.assertAvailable(text(params,'successor')); return {ok:true};
    case 'internal.analyticsSummary':
    case 'internal.analyticsRecent':
      if (!context.analyticsSnapshots) throw new Error('Analytics source is unavailable');
      return headlessAnalytics(method === 'internal.analyticsSummary' ? 'analytics.summary' : 'analytics.recent', params, context.analyticsSnapshots);
    case 'desktop.filePickerList': return listPickerEntries(params.path);
    case 'desktop.readFileBytes': return readFileBytes(params.path,context.workspaceRoots);
    case 'ui.fonts': return listUIFonts();
    case 'ui.asset': return readUIAsset(params.kind, params.file);
    case 'desktop.installUiFont': return installUIFont(params.name, params.dataBase64);
    case 'desktop.downloadProjectIcon': return installProjectIcon(params.url);
    // Private lifecycle messages have NO public hub capability. The Go spawn
    // wrapper supplies the accepted daemon ID, never a browser's claim.
    case 'internal.prepareSpawn': {
      const submitted = params.spawn as Params;
      managerReplacementState.assertResume(submitted.resumeSessionId as string | undefined);
      let prepared!: (value: unknown) => void;
      let rejected!: (error: unknown) => void;
      const ready = new Promise((resolve, reject) => { prepared = resolve; rejected = reject; });
      let finish!: (value: unknown) => void;
      let cancel!: (error: Error) => void;
      const completion = new Promise((resolve, reject) => { finish = resolve; cancel = reject; });
      const run = managerReplacementState.admitted(
        [submitted.dispatchOwnerSessionId as string | undefined, submitted.parentSessionId as string | undefined],
        () => workflow.workflowSpawn(async (wire) => {
          const p = wire as Params;
          const owner = typeof p.parentSessionId === 'string' && p.dispatchOwnerSessionId === p.parentSessionId ? currentSnapshot(p.parentSessionId) : undefined;
          const projectCwd = text(p, 'cwd');
          const input: Omit<Admission, 'sessionId'> = {
            owner, projectCwd, executionCwd: projectCwd,
            title: typeof p.label === 'string' ? p.label : undefined,
            trackTask: p.trackTask as boolean | undefined,
            taskId: p.taskId as string | undefined,
            workflowStepId: p.workflowStepId as string | undefined,
            stage: p.stage as Admission['stage'],
            afterDispatchId: p.afterDispatchId as string | undefined,
            retrySourceSessionId: p.retrySourceSessionId as string | undefined,
            requestedProvider: p.provider as string | undefined,
            provider: p.provider as string | undefined,
            requestedModel: p.model as string | undefined,
            role: p.role as string | undefined,
          };
          dispatchHistoryStore.validate(input);
          if (admissions.size >= 128) throw new Error('Too many pending dispatches');
          let template: WorkflowTemplate | undefined;
          if (p.workflowStepId && p.taskId) template = workflow.pinnedWorkflowTemplate(String(p.taskId), String(p.workflowStepId));
          else if (p.template) {
            if (typeof p.message === 'string' && p.message.trim()) throw new Error('Pass template or message, not both');
            template = dispatchTemplates.find((t) => t.id === p.template);
            if (!template) throw new Error(`Dispatch template unavailable: ${String(p.template)}`);
          } else if (p.templateParams && Object.keys(p.templateParams).length) throw new Error('templateParams requires a template');
          if (template) validateDispatchTemplateParams(template.body, (p.templateParams ?? {}) as Record<string, string>);
          const schema = (p.workflowStepId ? template?.resultSchema : p.resultSchema ?? template?.resultSchema) as Record<string, unknown> | undefined;
          if (schema !== undefined) { const error = checkResultSchema(schema); if (error) throw new Error(error); }
          let review: ReviewAllocation | undefined;
          if (p.worktree === true) {
            const source = assertPathAllowed('desktop.worktreeCreate', projectCwd, context.setupRoots);
            const result = await createWorktree({ repoCwd: source, name: typeof p.label === 'string' ? p.label : undefined, rootOverride: configuredWorktreeRoot(), config: configService.getConfig() });
            input.worktree = { requested: true, allocated: result.ok, fallback: !result.ok, branch: result.branch, error: result.error };
            if (result.ok && result.path) { input.executionCwd = result.path; review = result.reviewAllocation; }
            if (p.workflowStepId && !result.ok) throw new Error('Workflow ship step requires successful worktree allocation');
          }
          const patch: Params = { cwd: input.executionCwd };
          if (template) {
            patch.message = renderDispatchTemplate(template.body, (p.templateParams ?? {}) as Record<string, string>, { cwd: input.executionCwd, projectCwd });
            patch.resultSchema = p.workflowStepId ? template.resultSchema : p.resultSchema ?? template.resultSchema;
          }
          if (p.toolScope) patch.toolScope = p.toolScope;
          const token = randomUUID();
          // Yield once so run is initialized even on a non-worktree spawn.
          await Promise.resolve();
          admissions.set(token, { input, review, resume: !!p.resumeSessionId, finish, cancel, settled: run });
          prepared({ token, cwd: input.executionCwd, worktree: input.worktree, patch, schema, contract: schema ? buildResultContract(schema) : '' });
          return completion;
        })(submitted),
      );
      void run.catch(rejected);
      return ready;
    }
    case 'internal.acceptSpawn': {
      const token = text(params, 'token');
      const admission = admissions.get(token);
      if (!admission) throw new Error('Dispatch admission expired or already consumed');
      admissions.delete(token);
      const sessionId = text(params, 'sessionId');
      try {
        const accepted = admission.resume ? undefined : dispatchHistoryStore.accept({ ...admission.input, owner: admission.input.owner ? currentSnapshot(admission.input.owner.sessionId) : undefined, sessionId });
        if (admission.review && admission.input.owner?.isWakeTarget) fleetReviewStore.register(admission.input.owner.sessionId, sessionId, admission.review);
        managerReplacementState.rememberChild({sessionId,cwd:admission.input.executionCwd,label:admission.input.title,parentSessionId:admission.input.owner?.sessionId,provider:admission.input.provider});
        admission.finish(accepted ?? {});
        await admission.settled;
        return accepted ?? {};
      } catch (error) {
        admission.cancel(error instanceof Error ? error : new Error('Dispatch admission failed'));
        await admission.settled.catch(() => {});
        throw error;
      }
    }
    case 'internal.cancelSpawn': {
      const token = text(params, 'token');
      const admission = admissions.get(token);
      admissions.delete(token);
      admission?.cancel(new Error('Spawn failed before acknowledgement'));
      await admission?.settled.catch(() => {});
      return { ok: true };
    }
    case 'desktop.fleetWorkflowRequest':
      return workflows.fleetWorkflowRequest(params.request as WorkflowRequest);
    case 'internal.workflowRequest':
      return workflows.fleetWorkflowRequest(params as unknown as WorkflowRequest, text(params, 'callerSessionId', true) || undefined);
    case 'desktop.managerRequestPrepare':
      return managerRequests().prepare(text(params, 'sessionId'), text(params, 'text'), params.bootstrap === true);
    case 'internal.beginDelivery': {
      const delivery = managerRequests().beginDelivery(managerReplacementState.wakeTarget(text(params, 'sessionId')), text(params, 'requestId'));
      if (!delivery) return null;
      return { ...delivery, text: delivery.bootstrap ? buildManagerKickoff(delivery.text, !!configService.getConfig().agents?.fleetFullAccess) : delivery.text };
    }
    case 'internal.finishDelivery':
      managerRequests().finishDelivery(text(params, 'requestId'), text(params, 'deliveryId'), params.status as 'accepted' | 'rejected' | 'unknown');
      return { ok: true };
    case 'internal.requestReceipt': {
      const request = managerRequests().request(managerReplacementState.wakeTarget(text(params, 'sessionId')), text(params, 'requestId'));
      return { ok: request.delivery === 'accepted' || request.delivery === 'pending', requestId: request.requestId, delivery: request.delivery, mode: request.delivery };
    }
    case 'internal.finishWorker': {
      const sessionId = text(params, 'sessionId');
      const session = currentSnapshot(sessionId);
      const task = dispatchHistoryStore.list().find((t) => t.attempts.some((a) => a.sessionId === sessionId));
      if (!session || !task) return {};
      const reply = text(params, 'reply', true);
      const escalation = readWorkerEscalation(reply);
      const schema = workflow.workflowResultSchema(sessionId) ?? session.resultSchema;
      const result = schema && !escalation?.json ? readStructuredResult(reply, schema) : {};
      const evidenceId = await fleetReviewStore.capture(task.ownerSessionId, sessionId, session.status === 'ended' ? 'session-ended' : 'turn-ended');
      const current = currentSnapshot(sessionId);
      if (!current || (current.status !== 'ended' && current.ambientState !== 'idle')) return {};
      const token = randomUUID();
      for (const [id, pending] of resultCommits) if (pending.expires < Date.now()) resultCommits.delete(id);
      if (resultCommits.size >= 128) throw new Error('Too many pending result validations');
      const fingerprint = (s: typeof current | undefined) => JSON.stringify([s?.status,s?.ambientState,s?.lastActivity,s?.cwd,s?.liveCwd]);
      const expected = fingerprint(current);
      resultCommits.set(token,{expires:Date.now()+60_000,commit:()=>{
        if (fingerprint(currentSnapshot(sessionId)) !== expected) throw new Error('Worker changed during result capture');
        dispatchHistoryStore.validated(sessionId, escalation?.json ? 'escalated' : result.json ? 'valid' : result.error ? 'invalid' : 'absent', evidenceId, result.json ? JSON.parse(extractResultBlock(reply)!) : escalation?.json);
        return {result:result.json,resultError:result.error,reviewEvidenceId:evidenceId,instructions:workflow.workflowWakeInstructions([sessionId])};
      }});
      return {token};
    }
    case 'internal.commitWorkerResult': {
      const token=text(params,'token'),pending=resultCommits.get(token);resultCommits.delete(token);
      if(!pending||pending.expires<Date.now())throw new Error('Worker result validation expired');
      return pending.commit();
    }
    case 'desktop.loadBriefBoard': return loadBoard();
    case 'desktop.moveBriefCard': return applyBoardMove(params.request as unknown as BoardMoveRequest);
    case 'desktop.saveConfig': {
      if (!params.partial || typeof params.partial !== 'object' || Array.isArray(params.partial)) throw new Error('Configuration patch must be an object');
      return configService.saveConfig(params.partial as Parameters<typeof configService.saveConfig>[0]);
    }
    case 'desktop.agentSuggestTitle': return generateAgentTitle(params.request as unknown as TitleRequest);
    case 'desktop.providerReadiness': {
      if (!readinessStarted) { readinessStarted = true; configService.onChange(() => readiness.invalidate()); readiness.start(); }
      const provider = text(params, 'provider');
      return params.check === true ? readiness.check(provider) : readiness.read(provider);
    }
    case 'desktop.workflowAgentTranscript':
    case 'desktop.workflowAgentConversation': {
      const sessionId = text(params, 'sessionId'), runId = text(params, 'runId'), agentId = text(params, 'agentId');
      if (![sessionId, runId, agentId].every((id) => artifactID.test(id))) throw new Error('Invalid workflow identity');
      const transcript = watchWorkflow(sessionId);
      if (!transcript) return null;
      const root = transcript.replace(/\.jsonl$/i, '');
      const target = path.join(root, 'subagents', 'workflows', `wf_${runId}`, `agent-${agentId.replace(/^agent-/, '')}.jsonl`);
      try { assertPathAllowed(method, target, [root]); } catch { return null; }
      workflowWatcher.refresh(sessionId);
      return method === 'desktop.workflowAgentTranscript' ? workflowWatcher.readAgentTranscript(sessionId, runId, agentId) : workflowWatcher.readAgentConversation(sessionId, runId, agentId);
    }
    case 'internal.observe': {
      if (!nativeSnapshot) await ensureReplacementReady();
      for (const s of context.snapshots) {
        if (watchWorkflow(s.sessionId) && s.status !== 'ended' && s.ambientState !== 'idle') workflowWatcher.poke(s.sessionId);
      }
      for (const id of workflowWatching.keys()) if (!context.snapshots.some((s) => s.sessionId === id)) { workflowWatcher.detach(id); workflowWatching.delete(id); workflowTelemetry.forgetSession(id); }
      const tracked = new Map(dispatchHistoryStore.list().flatMap((t) => t.attempts.map((a) => [a.sessionId, t] as const)));
      for (const s of context.snapshots) {
        const task = tracked.get(s.sessionId);
        if (!task) continue;
        const digest = JSON.stringify(s);
        if (observed.get(s.sessionId) === digest) continue;
        observed.set(s.sessionId, digest);
        dispatchHistoryStore.observe(s as Parameters<typeof dispatchHistoryStore.observe>[0]);
        const state = s.status === 'ended' ? 'ended' : s.ambientState;
        lifecycle.set(s.sessionId, state ?? 'unknown');
      }
      for (const id of observed.keys()) if (!tracked.has(id)) { observed.delete(id); lifecycle.delete(id); }
      return { ok: true };
    }
    case 'desktop.worktreeInfo':
      return worktreeInfo(assertPathAllowed(method, text(params, 'cwd'), context.setupRoots));
    case 'desktop.worktreeCreate': {
      const repoCwd = assertPathAllowed(method, text(params, 'repoCwd'), context.setupRoots);
      const root = configuredWorktreeRoot();
      const requested = text(params, 'rootOverride', true);
      if (requested && path.resolve(requested) !== path.resolve(root)) throw new Error('Worktree destination must match the server configuration');
      return createWorktree({ repoCwd, name: text(params, 'name', true), rootOverride: root, config: configService.getConfig() });
    }
    case 'desktop.worktreeRemove': {
      const root = canonicalRoot(configuredWorktreeRoot());
      const cwd = canonicalRoot(text(params, 'cwd'));
      if (!root || !cwd || cwd === root || !containsCanonical(root, cwd)) return { ok: false, skipped: true };
      if (context.snapshots.some((s) => s.status !== 'ended' && (s.liveCwd || s.cwd) === cwd)) return { ok: false, skipped: true, error: 'Worktree still has a live agent' };
      return removeAgentWorktree({ cwd, rootOverride: root });
    }
    case 'desktop.pricingGetRates':
      return { defaults: Object.fromEntries(Object.entries(MODEL_RATES).map(([prefix, rate]) => [prefix, { ...rate, contextLimit: windowFor(prefix) }])), overrides: readModelRateOverrides() };
    case 'desktop.pricingSaveOverrides': {
      const overrides = params.overrides;
      if (!overrides || typeof overrides !== 'object' || Array.isArray(overrides)) throw new Error('Rate overrides must be an object');
      for (const row of Object.values(overrides)) {
        if (!row || typeof row !== 'object' || ['input', 'output'].some((key) => typeof row[key] !== 'number' || !Number.isFinite(row[key]) || row[key] < 0)) throw new Error('Rates must have nonnegative finite input and output values');
        for (const key of ['cached_input', 'context_limit']) if (row[key] !== undefined && (typeof row[key] !== 'number' || !Number.isFinite(row[key]) || row[key] < 0)) throw new Error(`Invalid ${key}`);
      }
      writeModelRateOverrides(overrides as ModelRateOverrides);
      return { ok: true };
    }
    case 'desktop.claudeProfilesAccounts':
      return Object.fromEntries(claudeProfiles.getProfiles().map((p) => [p.id, profileAccount(p)]));
    case 'desktop.claudeProfilesLoginStatus':
      return Object.fromEntries(claudeProfiles.getProfiles().map((p) => [p.id, profileSignedIn(p)]));
    case 'desktop.claudeProfilesAddAccount': {
      const name = text(params, 'name', true).trim() || 'Account';
      const { dir, shared, warnings } = createAccountConfigDir(name);
      return { profile: claudeProfiles.addProfile(name, dir, [], []), shared, warnings };
    }
    case 'desktop.claudeProfilesAdd':
      return claudeProfiles.addProfile(text(params, 'name'), text(params, 'configDir', true), (params.extraArgs ?? []) as string[], (params.mcpItemIds ?? []) as string[], params.init as Parameters<typeof claudeProfiles.addProfile>[4]);
    case 'desktop.claudeProfilesUpdate': {
      const result = claudeProfiles.updateProfile(text(params, 'id'), params.updates as Parameters<typeof claudeProfiles.updateProfile>[1]);
      if (!result) throw new Error('Profile not found');
      return result;
    }
    case 'desktop.claudeProfilesRemove': claudeProfiles.removeProfile(text(params, 'id')); return null;
    case 'desktop.toolsStatus': return toolsStatus(true);
    case 'desktop.fleetReviewRead': return fleetReviewStore.read(params.request);
    case 'desktop.fleetReviewForget': return fleetReviewStore.forget(params.request);
    case 'desktop.taskInspectorEdit':
      return dispatchHistoryStore.editByHostUser(params.request as TaskEditRequest, snapshot, (id) => workflow.workflowBusy.has(id));
    case 'desktop.taskInspectorOpen':
      return { ok: true, ...dispatchHistoryStore.openTarget(params.request as TaskOpenRequest) };
    case 'desktop.dispatchHistoryRead': {
      const owners = context.snapshots.filter((s) => s.isWakeTarget && s.status !== 'ended' && !s.hub).sort((a,b) => (b.startedAt ?? 0) - (a.startedAt ?? 0));
      return { available: true, currentOwnerSessionId: owners[0]?.sessionId, tasks: dispatchHistoryStore.listForHostUser(snapshot), requests: owners.flatMap((s) => dispatchHistoryStore.listRequests(s.sessionId).map((r) => ({ ownerSessionId: r.ownerSessionId, requestId: r.requestId, delivery: r.delivery, resolved: !!r.intents }))) };
    }
    case 'desktop.htmlCardReadDiff': {
      const owner = liveOwner(context, text(params, 'ownerId'));
      const cwd = owner.liveCwd || owner.cwd;
      const result = await readHtmlCardDiff(text(params, 'target'), cwd);
      const current = currentSnapshot(owner.sessionId);
      return !current || current.status === 'ended' || current.hub || (current.liveCwd || current.cwd) !== cwd
        ? { ok: false, error: 'Owning session changed while reading the diff' } : result;
    }
    default: throw new Error(`Unknown desktop service: ${method}`);
  }
}
