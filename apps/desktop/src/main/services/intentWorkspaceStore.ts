import { resolveIntegration, type IntentIntegrationResponse } from '../shared/intentIntegrations';
import type { IntentSource } from '../shared/intentSources';
import { IntentCompletionStore, INTENT_COMPLETION_SCHEMA } from './intentCompletionStore';
import { intentCompletionIdle, boundIntentReport } from '../shared/intentCompletion';
import { INTENT_INTEGRATION_SCHEMA } from './intentIntegrationStore';
import { INTENT_SOURCE_SYNC_SCHEMA } from './intentSourceSyncStore';
import {
  IntentAutomationStore,
  INTENT_AUTOMATION_SCHEMA,
  type IntentAutomationEffects,
} from './intentAutomationStore';
import { randomUUID } from 'node:crypto';
import fs from 'node:fs';
import path from 'node:path';
import type { DatabaseSync } from 'node:sqlite';
import { isMainThread } from 'node:worker_threads';
import { isIntentFileAction } from './intentFileWorkerProtocol';
import { getConfigDir } from './configService';
import { IntentSteeringStore, type IntentDirectionDelivery } from './intentSteeringStore';
import { IntentEvidenceStore, INTENT_EVIDENCE_SCHEMA } from './intentEvidenceStore';
import { IntentSourceStore, INTENT_SOURCE_SCHEMA } from './intentSourceStore';
import { SOURCE_ACTIONS } from '../shared/intentSources';
import { IntentProjectStore, INTENT_PROJECT_SCHEMA } from './intentProjectStore';
import { IntentKnowledgeStore, INTENT_KNOWLEDGE_SCHEMA } from './intentKnowledgeStore';
import { KNOWLEDGE_ACTIONS } from '../shared/intentKnowledge';
import { IntentArtifactStore, INTENT_ARTIFACT_SCHEMA } from './intentArtifactStore';
import {
  IntentControlStore,
  INTENT_CONTROL_SCHEMA,
  type IntentControlDelivery,
} from './intentControlStore';
import {
  INTENT_STATUSES,
  buildIntentContext,
  intentObservation,
  type IntentExecution,
  type IntentLiveSession,
  type IntentObservation,
  type IntentSessionRef,
  type IntentWorkLink,
  type IntentFields,
  type IntentWorkspace,
  type IntentWorkspaceResponse,
} from '../shared/intentWorkspace';

function object(value: unknown): Record<string, unknown> {
  if (!value || typeof value !== 'object' || Array.isArray(value))
    throw new Error('Invalid workspace request');
  return value as Record<string, unknown>;
}

function text(value: unknown, name: string, max: number, required = false): string {
  if (typeof value !== 'string' || value.length > max || value.includes('\0'))
    throw new Error(`Invalid ${name}`);
  const result = value.trim();
  if (required && !result) throw new Error(`${name} is required`);
  return result;
}

function fields(value: unknown): IntentFields {
  const raw = object(value);
  if (!INTENT_STATUSES.includes(raw.status as IntentFields['status']))
    throw new Error('Invalid workspace status');
  const sourceUrl = text(raw.sourceUrl, 'source link', 4096);
  if (sourceUrl) {
    let url: URL;
    try {
      url = new URL(sourceUrl);
    } catch {
      throw new Error('Source link must be an HTTP or HTTPS URL');
    }
    if (!['https:', 'http:'].includes(url.protocol) || url.username || url.password)
      throw new Error('Source link must be an HTTP or HTTPS URL without credentials');
  }
  return {
    title: text(raw.title, 'Title', 240, true),
    outcome: text(raw.outcome, 'outcome', 32_000),
    constraints: text(raw.constraints, 'constraints', 32_000),
    successCriteria: text(raw.successCriteria, 'success criteria', 32_000),
    sourceUrl,
    status: raw.status as IntentFields['status'],
  };
}

/** One schema and implementation for native Electron and the headless host.
 * Current state and its revision are committed together; stale clients must reload.
 * No foreign path from a request is opened: projectRoot is an identity only.
 */
export const INTENT_WORKSPACE_SCHEMA_VERSION = 10;
export class IntentWorkspaceStore {
  readonly completions: IntentCompletionStore;
  readonly automation: IntentAutomationStore;
  readonly steering: IntentSteeringStore;
  readonly evidence: IntentEvidenceStore;
  readonly controls: IntentControlStore;
  readonly sources: IntentSourceStore;
  readonly projects: IntentProjectStore;
  readonly knowledge: IntentKnowledgeStore;
  readonly artifacts: IntentArtifactStore;
  private linked: Map<string, IntentExecution[]> | undefined;
  private dataVersion = -1;
  private pending = new Map<string, IntentObservation>();
  captureWarning: string | undefined;
  private reportFailures = new Map<string, string>();

  reportCaptureResult(sessionId: string, error?: string): void {
    if (error) this.reportFailures.set(sessionId, error);
    else this.reportFailures.delete(sessionId);
  }

  private get reportCaptureWarning(): string | undefined {
    return this.reportFailures.size
      ? `Background agent report capture failed: ${[...new Set(this.reportFailures.values())].join('; ')}`
      : undefined;
  }

  private linkedExecutions(): Map<string, IntentExecution[]> {
    const version = Number(this.db.prepare('PRAGMA data_version').get()?.data_version);
    if (!this.linked || version !== this.dataVersion) {
      const linked = new Map<string, IntentExecution[]>();
      for (const row of this.db.prepare('SELECT snapshot FROM intent_executions').all()) {
        const execution = JSON.parse(String(row.snapshot)) as IntentExecution;
        if (!execution.session) continue;
        const key = sessionKey(execution.session);
        linked.set(key, [...(linked.get(key) ?? []), execution]);
      }
      this.linked = linked;
      this.dataVersion = version;
    }
    return this.linked;
  }

  trackedSessions(): IntentSessionRef[] {
    return [...this.linkedExecutions().values()].map((rows) => rows[0].session!);
  }

  /** Shared by lifecycle capture and reads. Only linked identities are eligible.
   * Busy reports checkpoint at most every five seconds; state/cwd changes and
   * late idle/stopped reports commit immediately. Failed samples remain queued.
   */
  capture(observations: readonly CapturedIntentSession[]): void {
    try {
      const linked = this.linkedExecutions();
      for (const sample of observations) {
        const key = sessionKey(sample);
        if (linked.has(key)) {
          const queued = this.pending.get(key);
          if (!queued || queued.observedAt <= sample.observation.observedAt)
            this.pending.set(key, {
              ...sample.observation,
              summary: sample.observation.summary || queued?.summary || '',
            });
        }
      }
      const updates: IntentExecution[] = [];
      const consumed: string[] = [];
      for (const [key, incoming] of this.pending) {
        let deferred = false;
        for (const execution of linked.get(key) ?? []) {
          const prior = execution.lastObservation;
          // Sparse snapshots (or a temporarily unavailable report source) must
          // not erase text already captured from the host.
          const observation = {
            ...incoming,
            summary: incoming.summary || prior?.summary || '',
            completionIdle:
              incoming.completionIdle ??
              (incoming.state === prior?.state ? prior.completionIdle : false),
          };
          if (prior && prior.observedAt > observation.observedAt) continue;
          if (
            prior &&
            prior.state === observation.state &&
            prior.summary === observation.summary &&
            prior.cwd === observation.cwd &&
            prior.completionIdle === observation.completionIdle
          )
            continue;
          const busy = ['thinking', 'streaming', 'background'].includes(observation.state);
          if (
            prior &&
            busy &&
            prior.state === observation.state &&
            prior.cwd === observation.cwd &&
            Date.parse(observation.observedAt) - Date.parse(prior.observedAt) < 5000
          ) {
            deferred = true;
            continue;
          }
          updates.push({
            ...execution,
            lastObservation: observation,
            updatedAt: observation.observedAt,
          });
        }
        if (!deferred) consumed.push(key);
      }
      if (updates.length)
        this.transaction(() => {
          const write = this.db.prepare('UPDATE intent_executions SET snapshot=? WHERE id=?');
          for (const execution of updates) {
            // Rebase under the write lock; another host connection may have
            // updated the execution after the cached index was read.
            const row = this.db
              .prepare('SELECT snapshot FROM intent_executions WHERE id=?')
              .get(execution.id);
            if (!row) continue;
            const current = JSON.parse(String(row.snapshot)) as IntentExecution;
            if (
              current.lastObservation &&
              current.lastObservation.observedAt > execution.lastObservation!.observedAt
            )
              continue;
            write.run(
              JSON.stringify({
                ...current,
                lastObservation: execution.lastObservation,
                updatedAt: execution.updatedAt,
              }),
              execution.id,
            );
          }
        });
      for (const key of consumed) this.pending.delete(key);
      this.transaction(() => {
        this.completions.observe(observations);
        this.automation.observe(observations);
      });
      this.captureWarning = undefined;
    } catch (error) {
      this.captureWarning = `Background result capture failed: ${error instanceof Error ? error.message : String(error)}`;
      throw error;
    }
  }

  constructor(private readonly db: DatabaseSync) {
    db.exec('PRAGMA foreign_keys=ON; PRAGMA busy_timeout=5000; PRAGMA journal_mode=WAL;');
    const version = Number(db.prepare('PRAGMA user_version').get()?.user_version ?? 0);
    if (version > INTENT_WORKSPACE_SCHEMA_VERSION)
      throw new Error('This workspace database requires a newer Workspacer version');
    if (version < 1)
      this.transaction(() => {
        db.exec(`
        CREATE TABLE IF NOT EXISTS intent_workspaces (
          id TEXT PRIMARY KEY, project_root TEXT NOT NULL, revision INTEGER NOT NULL,
          updated_at TEXT NOT NULL, snapshot TEXT NOT NULL
        );
        CREATE INDEX IF NOT EXISTS intent_workspace_project ON intent_workspaces(project_root, updated_at);
        CREATE TABLE IF NOT EXISTS intent_revisions (
          workspace_id TEXT NOT NULL REFERENCES intent_workspaces(id),
          revision INTEGER NOT NULL, at TEXT NOT NULL, reason TEXT NOT NULL,
          snapshot TEXT NOT NULL, PRIMARY KEY(workspace_id, revision)
        );
        PRAGMA user_version=1;
      `);
      });
    if (version < 2)
      this.transaction(() => {
        db.exec(`
        CREATE TABLE IF NOT EXISTS intent_executions (
          id TEXT PRIMARY KEY, workspace_id TEXT NOT NULL, intent_revision INTEGER NOT NULL,
          snapshot TEXT NOT NULL,
          FOREIGN KEY(workspace_id, intent_revision) REFERENCES intent_revisions(workspace_id, revision)
        );
        CREATE INDEX IF NOT EXISTS intent_execution_workspace ON intent_executions(workspace_id);
        CREATE TABLE IF NOT EXISTS intent_work_links (
          id TEXT PRIMARY KEY, workspace_id TEXT NOT NULL REFERENCES intent_workspaces(id),
          kind TEXT NOT NULL, target TEXT NOT NULL, created_at TEXT NOT NULL,
          UNIQUE(workspace_id, kind, target)
        );
        PRAGMA user_version=2;
      `);
      });
    if (version < 3)
      this.transaction(() => {
        db.exec(`
        CREATE TABLE IF NOT EXISTS intent_directions (
          id TEXT PRIMARY KEY, workspace_id TEXT NOT NULL,
          execution_id TEXT NOT NULL REFERENCES intent_executions(id),
          intent_revision INTEGER NOT NULL, snapshot TEXT NOT NULL,
          FOREIGN KEY(workspace_id, intent_revision) REFERENCES intent_revisions(workspace_id, revision)
        );
        CREATE INDEX IF NOT EXISTS intent_direction_workspace ON intent_directions(workspace_id);
        PRAGMA user_version=3;
      `);
      });
    this.steering = new IntentSteeringStore(db, (id) => this.contextPacket(id));
    if (version < 4)
      this.transaction(() =>
        db.exec(INTENT_EVIDENCE_SCHEMA + INTENT_CONTROL_SCHEMA + 'PRAGMA user_version=4;'),
      );
    this.evidence = new IntentEvidenceStore(db, undefined, (review) => {
      const proposal = this.completions.validateReview(review);
      this.automation.review(review.workspaceId, review.decision, review.reason, proposal);
      if (review.decision === 'changes-requested')
        review.directionId = this.automation.get(review.workspaceId)?.id;
    });
    this.controls = new IntentControlStore(db, (id) => this.contextPacket(id));
    this.projects = new IntentProjectStore(db);
    if (version < 5)
      this.transaction(() => {
        db.exec(
          INTENT_SOURCE_SCHEMA +
            INTENT_PROJECT_SCHEMA +
            INTENT_KNOWLEDGE_SCHEMA +
            INTENT_ARTIFACT_SCHEMA,
        );
        this.projects.backfill();
        db.exec('PRAGMA user_version=5;');
      });
    if (version < 6)
      this.transaction(() => db.exec(INTENT_AUTOMATION_SCHEMA + 'PRAGMA user_version=6;'));
    this.automation = new IntentAutomationStore(
      db,
      (input) => this.request(input),
      (input, sessions, deliver) => this.steering.send(input, sessions, deliver),
      (workspace, run) => this.evidence.reportRun(workspace, run),
    );
    if (version < 7)
      this.transaction(() => db.exec(INTENT_SOURCE_SYNC_SCHEMA + 'PRAGMA user_version=7;'));
    if (version < 8)
      this.transaction(() => db.exec(INTENT_INTEGRATION_SCHEMA + 'PRAGMA user_version=8;'));
    if (version < 9)
      this.transaction(() => db.exec(INTENT_COMPLETION_SCHEMA + 'PRAGMA user_version=9;'));
    if (version < 10)
      this.transaction(() =>
        db.exec(
          `CREATE TABLE IF NOT EXISTS intent_jira_imports (id TEXT PRIMARY KEY, fingerprint TEXT NOT NULL, response TEXT NOT NULL); PRAGMA user_version=10;`,
        ),
      );
    this.completions = new IntentCompletionStore(db, (workspace) =>
      this.automation.status(workspace, 'review', 'Agent reported work ready for review'),
    );
    this.sources = new IntentSourceStore(db);
    this.knowledge = new IntentKnowledgeStore(db);
    this.artifacts = new IntentArtifactStore(db);
  }

  /** Snapshot provenance travels with future requests. Earlier packets remain
   * immutable; compiling this context never fetches a provider or changes a file. */
  contextPacket(workspaceId: string): string {
    const sources = this.sources.contextPacket(workspaceId);
    const knowledge = this.knowledge.contextPacket(workspaceId);
    const row = this.db
      .prepare('SELECT revision FROM intent_workspaces WHERE id=?')
      .get(workspaceId);
    const revision = Number(row?.revision);
    const evidence = this.db
      .prepare(
        'SELECT snapshot FROM intent_evidence WHERE workspace_id=? AND intent_revision=? ORDER BY rowid DESC LIMIT 8',
      )
      .all(workspaceId, revision)
      .map((row) => {
        const item = JSON.parse(
          String(row.snapshot),
        ) as import('../shared/intentEvidence').IntentEvidence;
        return {
          id: item.id,
          criterion: item.criterion.text.slice(0, 500),
          assessment: item.assessment,
          noteExcerpt: item.note.slice(0, 500),
          reference: item.reference,
          ...(item.git
            ? {
                gitArtifact: item.git.artifactId,
                sha256: item.git.sha256,
                head: item.git.headCommit,
              }
            : {}),
        };
      });
    const choices = this.db
      .prepare(
        'SELECT snapshot FROM intent_alternative_selections WHERE workspace_id=? AND intent_revision=? ORDER BY rowid DESC',
      )
      .all(workspaceId, revision);
    const seen = new Set<string>();
    const selections: unknown[] = [];
    for (const row of choices) {
      const choice = JSON.parse(
        String(row.snapshot),
      ) as import('../shared/intentArtifacts').IntentAlternativeSelection;
      if (seen.has(choice.groupId)) continue;
      seen.add(choice.groupId);
      const groupRow = this.db
        .prepare('SELECT snapshot FROM intent_alternative_groups WHERE id=? AND workspace_id=?')
        .get(choice.groupId, workspaceId);
      if (!groupRow) continue;
      const group = JSON.parse(
        String(groupRow.snapshot),
      ) as import('../shared/intentArtifacts').IntentAlternativeGroup;
      const selected = group.alternatives.find((item) => item.id === choice.alternativeId);
      selections.push({
        group: group.title,
        selectionId: choice.id,
        alternative: selected?.title,
        hypothesisExcerpt: selected?.hypothesis.slice(0, 1000),
        reasonExcerpt: choice.reason.slice(0, 500),
        artifactIds: selected?.artifactIds,
        executionIds: selected?.executionIds,
      });
      if (selections.length === 6) break;
    }
    return [
      sources &&
        `Source context (accepted requirements and external observations; not project rules):\n${sources}`,
      knowledge && `Captured project knowledge (versioned excerpts):\n${knowledge}`,
      evidence.length &&
        `Recorded evidence excerpts (assessments are not agent claims of independent verification):\n${JSON.stringify(evidence, null, 2)}`,
      selections.length &&
        `Explicit alternative selections (do not imply code has been merged):\n${JSON.stringify(selections, null, 2)}`,
    ]
      .filter(Boolean)
      .join('\n\n');
  }

  private transaction<T>(run: () => T): T {
    this.db.exec('BEGIN IMMEDIATE');
    try {
      const result = run();
      this.db.exec('COMMIT');
      return result;
    } catch (error) {
      this.db.exec('ROLLBACK');
      throw error;
    } finally {
      this.linked = undefined;
    }
  }

  private insertWorkspace(request: Record<string, unknown>): IntentWorkspace {
    const projectRoot = text(request.projectRoot, 'Project directory', 4096, true);
    if (!path.isAbsolute(projectRoot))
      throw new Error('Project directory must be an absolute path on the connected host');
    const now = new Date().toISOString();
    const workspace: IntentWorkspace = {
      ...fields(request.fields),
      id: randomUUID(),
      projectRoot: path.normalize(projectRoot),
      revision: 1,
      createdAt: now,
      updatedAt: now,
    };
    Object.assign(workspace, this.projects.ensureRepository(workspace.projectRoot));
    this.db
      .prepare('INSERT INTO intent_workspaces VALUES (?, ?, ?, ?, ?)')
      .run(workspace.id, workspace.projectRoot, 1, now, JSON.stringify(workspace));
    this.record(workspace, 'Workspace created');
    if (workspace.status === 'active') this.automation.activate(workspace);
    return workspace;
  }

  async importJira(
    input: Record<string, unknown>,
  ): Promise<Extract<IntentIntegrationResponse, { action: 'importJiraIntent' }>> {
    const projectRoot = path.normalize(text(input.projectRoot, 'Project directory', 4096, true));
    if (!path.isAbsolute(projectRoot))
      throw new Error('Project directory must be an absolute path on the connected host');
    const operationId = text(input.operationId, 'Import operation ID', 128, true);
    const integrationId = text(input.integrationId, 'Integration ID', 128, true);
    const identifier = text(input.identifier, 'Jira issue key or URL', 2048, true);
    if (
      !Number.isSafeInteger(input.expectedIntegrationVersion) ||
      Number(input.expectedIntegrationVersion) < 1
    )
      throw new Error('Invalid integration version');
    const fingerprint = JSON.stringify({
      projectRoot,
      integrationId,
      identifier,
      version: input.expectedIntegrationVersion,
    });
    const replay = () => {
      const row = this.db
        .prepare('SELECT fingerprint,response FROM intent_jira_imports WHERE id=?')
        .get(operationId);
      if (!row) return null;
      if (row.fingerprint !== fingerprint)
        throw new Error('Import operation ID was already used for another ticket');
      return JSON.parse(String(row.response)) as Extract<
        IntentIntegrationResponse,
        { action: 'importJiraIntent' }
      >;
    };
    const prior = replay();
    if (prior) return prior;
    const resolve = () => {
      const integration = this.sources.integrations
        .listProject(projectRoot)
        .find((item) => item.id === integrationId);
      if (!integration || integration.provider !== 'jira' || !integration.enabled)
        throw new Error('Choose an enabled Jira connection for this project');
      if (integration.version !== input.expectedIntegrationVersion)
        throw new Error('Jira connection changed. Reload connections before importing.');
      let key = identifier;
      if (/^https?:/i.test(key)) {
        const url = new URL(key);
        if (
          url.origin !== integration.baseUrl ||
          url.username ||
          url.password ||
          url.search ||
          url.hash ||
          !/^\/browse\/[^/]+$/.test(url.pathname)
        )
          throw new Error('Use a Jira issue URL from the selected connection');
        key = url.pathname.slice('/browse/'.length);
      }
      const connection = resolveIntegration(integration, { objectType: 'issue', identifier: key });
      return { integration, connection };
    };
    const { integration, connection } = resolve();
    const read = await this.sources.sync.import(connection);
    if (!read.snapshot) throw new Error('Jira did not return ticket requirements');
    return this.transaction(() => {
      const raced = replay();
      if (raced) return raced;
      resolve(); // Configuration may change while Jira is being read.
      const workspace = this.insertWorkspace({
        projectRoot,
        fields: {
          title: read.snapshot!.title.slice(0, 240) || read.nativeId,
          outcome: read.snapshot!.content.slice(0, 32000),
          constraints: '',
          successCriteria: '',
          sourceUrl: connection.url,
          status: 'draft',
        },
      });
      const source: IntentSource = {
        ...connection,
        integration,
        id: randomUUID(),
        workspaceId: workspace.id,
        nativeId: read.nativeId,
        version: 1,
        accepted: read.snapshot!,
        candidate: null,
        history: [],
        createdAt: new Date().toISOString(),
      };
      this.db
        .prepare('INSERT INTO intent_sources VALUES(?,?,?)')
        .run(source.id, workspace.id, JSON.stringify(source));
      source.external = this.sources.sync.record(source.id, read);
      const response = { action: 'importJiraIntent' as const, workspace, source };
      this.db
        .prepare('INSERT INTO intent_jira_imports VALUES(?,?,?)')
        .run(operationId, fingerprint, JSON.stringify(response));
      return response;
    });
  }

  request(input: unknown, sessions: readonly IntentLiveSession[] = []): IntentWorkspaceResponse {
    const request = object(input);
    if (request.action === 'jiraIntegrations') {
      const projectRoot = text(request.projectRoot, 'Project directory', 4096, true);
      if (!path.isAbsolute(projectRoot)) throw new Error('Project directory must be absolute');
      return {
        action: 'jiraIntegrations',
        integrations: this.sources.integrations
          .listProject(path.normalize(projectRoot))
          .filter((item) => item.provider === 'jira' && item.enabled),
      };
    }
    if (request.action === 'completionProposals')
      return this.completions.view(text(request.id, 'workspace ID', 128, true));
    if (
      [
        'automation',
        'activateIntent',
        'pauseIntent',
        'answerIntent',
        'restartIntent',
        'resumeInspectedIntent',
      ].includes(String(request.action))
    )
      return this.transaction(() => this.automation.request(request, sessions));
    if (
      [
        'projects',
        'renameIntentProject',
        'addIntentRepository',
        'relocateIntentRepository',
      ].includes(String(request.action))
    )
      return this.projects.request(request);
    if ((KNOWLEDGE_ACTIONS as readonly string[]).includes(String(request.action)))
      return this.knowledge.request(request);
    if (
      [
        'artifacts',
        'readArtifact',
        'addArtifact',
        'annotateArtifact',
        'createDemonstration',
        'createAlternativeGroup',
        'selectAlternative',
      ].includes(String(request.action))
    )
      return this.artifacts.request(request);
    if (
      request.action === 'directions' ||
      request.action === 'prepareDirection' ||
      request.action === 'reconcileDirection'
    )
      return this.steering.request(request);
    if (
      ['evidence', 'addEvidence', 'readEvidence', 'recordReview'].includes(String(request.action))
    ) {
      if (
        request.action === 'recordReview' &&
        request.proposalId &&
        request.decision === 'accept'
      ) {
        // Owner-host current lifecycle, not renderer-supplied eligibility. Replays
        // are handled by the evidence operation key before any further side effect.
        const replay = this.db
          .prepare('SELECT id FROM intent_reviews WHERE id=?')
          .get(String(request.reviewId));
        if (!replay) {
          const view = this.completions.view(String(request.id));
          const proposal = view.proposals.find((p) => p.id === request.proposalId);
          const live =
            proposal &&
            sessions.find(
              (s) =>
                s.sessionId === proposal.session.sessionId &&
                (s.hub || '') === proposal.session.hub,
            );
          if (
            !live ||
            this.reportFailures.has(live.sessionId) ||
            !intentCompletionIdle(live, sessions)
          )
            throw new Error(
              'The execution is unavailable or no longer idle. Open the session before approving.',
            );
          this.capture(captureIntentSessions(sessions));
        }
      }
      return this.evidence.request(request);
    }
    if (['controls', 'prepareControl', 'reconcileControl'].includes(String(request.action)))
      return this.controls.request(request);
    if (request.action === 'executions') {
      // A read retries queued failures, but still returns retained records with
      // a visible warning if the disk remains unwritable.
      try {
        this.capture(captureIntentSessions(sessions));
      } catch {
        /* captureWarning carries the error. */
      }
    }
    if (
      [
        'executions',
        'prepareExecution',
        'linkExecution',
        'markExecutionUnknown',
        'attachSession',
        'addWorkLink',
      ].includes(String(request.action))
    ) {
      return this.executionRequest(request, sessions);
    }
    if (request.action === 'list') {
      return {
        action: 'list',
        projects: (
          this.projects.request({ action: 'projects' }) as Extract<
            import('../shared/intentProject').IntentProjectResponse,
            { action: 'projects' }
          >
        ).projects,
        executionIndex: Object.fromEntries(
          this.db
            .prepare('SELECT workspace_id, snapshot FROM intent_executions')
            .all()
            .reduce((index, row) => {
              const execution = JSON.parse(String(row.snapshot)) as IntentExecution;
              if (execution.session)
                index.set(String(row.workspace_id), [
                  ...(index.get(String(row.workspace_id)) || []),
                  execution.session,
                ]);
              return index;
            }, new Map<string, IntentSessionRef[]>()),
        ),
        workspaces: this.db
          .prepare('SELECT snapshot FROM intent_workspaces ORDER BY updated_at DESC, id')
          .all()
          .map((row) => JSON.parse(String(row.snapshot)) as IntentWorkspace),
      };
    }
    if (request.action === 'create') {
      const workspace = this.transaction(() => this.insertWorkspace(request));
      return { action: 'create', workspace };
    }
    if (request.action !== 'history' && request.action !== 'update')
      throw new Error('Unknown workspace action');
    const id = text(request.id, 'workspace ID', 128, true);
    if (request.action === 'history') {
      if (!this.db.prepare('SELECT id FROM intent_workspaces WHERE id=?').get(id))
        throw new Error('Workspace no longer exists');
      return {
        action: 'history',
        statusEvents: this.db
          .prepare(
            'SELECT at, status, reason FROM intent_status_events WHERE workspace_id=? ORDER BY rowid DESC',
          )
          .all(id) as { at: string; status: IntentFields['status']; reason: string }[],
        revisions: this.db
          .prepare('SELECT * FROM intent_revisions WHERE workspace_id=? ORDER BY revision DESC')
          .all(id)
          .map((row) => ({
            revision: Number(row.revision),
            at: String(row.at),
            reason: String(row.reason),
            snapshot: JSON.parse(String(row.snapshot)) as IntentWorkspace,
          })),
      };
    }
    const next = fields(request.fields);
    const reason = text(request.reason, 'revision reason', 4000);
    if (!Number.isSafeInteger(request.expectedRevision) || Number(request.expectedRevision) < 1)
      throw new Error('Invalid expected revision');
    return this.transaction(() => {
      const row = this.db
        .prepare('SELECT snapshot, revision FROM intent_workspaces WHERE id=?')
        .get(id);
      if (!row) throw new Error('Workspace no longer exists');
      if (row.revision !== request.expectedRevision)
        throw new Error(
          'This workspace changed elsewhere. Your draft is preserved; reload the latest revision before saving.',
        );
      const previous = JSON.parse(String(row.snapshot)) as IntentWorkspace;
      if (
        request.expectedUpdatedAt !== undefined &&
        request.expectedUpdatedAt !== previous.updatedAt
      )
        throw new Error('This work item changed. Refresh before saving.');
      const contentChanged = (
        ['title', 'outcome', 'constraints', 'successCriteria', 'sourceUrl'] as const
      ).some((key) => previous[key] !== next[key]);
      const workspace: IntentWorkspace = {
        ...previous,
        ...next,
        revision: previous.revision + (contentChanged ? 1 : 0),
        updatedAt: new Date(Math.max(Date.now(), Date.parse(previous.updatedAt) + 1)).toISOString(),
      };
      this.db
        .prepare('UPDATE intent_workspaces SET revision=?, updated_at=?, snapshot=? WHERE id=?')
        .run(workspace.revision, workspace.updatedAt, JSON.stringify(workspace), id);
      if (contentChanged) this.record(workspace, reason || 'Intent updated');
      if (previous.status !== workspace.status)
        this.automation.status(workspace, workspace.status, reason || 'Status updated');
      if (workspace.status === 'active' && (previous.status !== 'active' || contentChanged))
        this.automation.activate(
          workspace,
          contentChanged
            ? 'The user updated the intent. Read the current requirements and adjust your work.'
            : '',
        );
      else if (workspace.status !== 'active' && previous.status === 'active')
        this.automation.pause(workspace, 'Work left Active status');
      return { action: 'update', workspace };
    });
  }

  private executionRequest(
    request: Record<string, unknown>,
    sessions: readonly IntentLiveSession[],
  ): IntentWorkspaceResponse {
    const id = text(request.id, 'workspace ID', 128, true);
    return this.transaction(() => {
      const row = this.db.prepare('SELECT snapshot FROM intent_workspaces WHERE id=?').get(id);
      if (!row) throw new Error('Workspace no longer exists');
      const workspace = JSON.parse(String(row.snapshot)) as IntentWorkspace;
      const readExecutions = () =>
        this.db
          .prepare(
            'SELECT snapshot FROM intent_executions WHERE workspace_id=? ORDER BY rowid DESC',
          )
          .all(id)
          .map((row) => JSON.parse(String(row.snapshot)) as IntentExecution);
      const persist = (execution: IntentExecution) =>
        this.db
          .prepare('UPDATE intent_executions SET snapshot=? WHERE id=?')
          .run(JSON.stringify(execution), execution.id);
      const observe = (execution: IntentExecution) => {
        const live =
          execution.session &&
          sessions.find(
            (item) =>
              item.sessionId === execution.session!.sessionId &&
              (item.hub || '') === execution.session!.hub,
          );
        if (live && !live.hubOffline) {
          const observation = intentObservation(live, new Date().toISOString());
          const prior = execution.lastObservation;
          if (
            !prior ||
            prior.state !== observation.state ||
            prior.summary !== observation.summary ||
            prior.cwd !== observation.cwd
          ) {
            execution.lastObservation = observation;
            execution.updatedAt = observation.observedAt;
            persist(execution);
          }
        }
        return execution;
      };
      if (request.action === 'executions') {
        const links = this.db
          .prepare('SELECT * FROM intent_work_links WHERE workspace_id=? ORDER BY rowid DESC')
          .all(id)
          .map((row) => ({
            id: String(row.id),
            workspaceId: id,
            kind: row.kind as IntentWorkLink['kind'],
            target: String(row.target),
            createdAt: String(row.created_at),
          }));
        const captureWarning = [this.captureWarning, this.reportCaptureWarning]
          .filter(Boolean)
          .join(' ');
        return {
          action: 'executions',
          executions: readExecutions(),
          links,
          ...(captureWarning ? { captureWarning } : {}),
        };
      }
      if (request.action === 'addWorkLink') {
        if (request.kind !== 'branch' && request.kind !== 'pull-request')
          throw new Error('Invalid work link kind');
        const target = text(request.target, 'Link target', 4096, true);
        if (request.kind === 'pull-request') fields({ ...workspace, sourceUrl: target });
        const existing = this.db
          .prepare(
            'SELECT id, created_at FROM intent_work_links WHERE workspace_id=? AND kind=? AND target=?',
          )
          .get(id, request.kind, target);
        const link: IntentWorkLink = {
          id: existing ? String(existing.id) : randomUUID(),
          workspaceId: id,
          kind: request.kind,
          target,
          createdAt: existing ? String(existing.created_at) : new Date().toISOString(),
        };
        if (!existing)
          this.db
            .prepare('INSERT INTO intent_work_links VALUES (?, ?, ?, ?, ?)')
            .run(link.id, id, link.kind, target, link.createdAt);
        return { action: 'addWorkLink', link };
      }
      if (request.action === 'prepareExecution' || request.action === 'attachSession') {
        // Idempotent launch claims return their existing record even if the intent
        // changed afterwards. The client must never dispatch again for created=false.
        const executionId =
          request.action === 'prepareExecution'
            ? text(request.executionId, 'execution ID', 128, true)
            : randomUUID();
        if (request.action === 'prepareExecution') {
          const existing = this.db
            .prepare('SELECT workspace_id, snapshot FROM intent_executions WHERE id=?')
            .get(executionId);
          if (existing) {
            if (existing.workspace_id !== id)
              throw new Error('Execution belongs to another workspace');
            return {
              action: 'prepareExecution',
              execution: JSON.parse(String(existing.snapshot)) as IntentExecution,
              created: false,
            };
          }
        }
        if (request.expectedRevision !== workspace.revision)
          throw new Error(
            'Intent changed elsewhere. Reload the saved revision before starting or linking work.',
          );
        const session =
          request.action === 'attachSession' ? this.sessionRef(request.session) : null;
        if (session) {
          const existing = readExecutions().find(
            (item) =>
              item.session?.sessionId === session.sessionId && item.session.hub === session.hub,
          );
          if (existing) return { action: 'attachSession', execution: observe(existing) };
        }
        const task =
          request.action === 'prepareExecution'
            ? text(request.task, 'Requested work', 32_000, true)
            : '';
        const now = new Date().toISOString();
        const execution: IntentExecution = {
          id: executionId,
          workspaceId: id,
          intentRevision: workspace.revision,
          kind: session ? 'attached' : 'launch',
          ...(session ? {} : { completionContract: 1 as const }),
          state: session ? 'linked' : 'launching',
          task,
          contextPacket: session ? null : buildIntentContext(workspace, executionId, task),
          session,
          lastObservation: null,
          createdAt: now,
          updatedAt: now,
        };
        if (execution.contextPacket) {
          const context = this.contextPacket(id);
          if (context) execution.contextPacket += `\n\nRecorded workspace context:\n${context}`;
          if (Buffer.byteLength(execution.contextPacket, 'utf8') > 240 * 1024)
            throw new Error(
              'Saved intent and workspace context exceed the launch packet limit. Shorten the intent or captured context.',
            );
        }
        this.db
          .prepare('INSERT INTO intent_executions VALUES (?, ?, ?, ?)')
          .run(executionId, id, workspace.revision, JSON.stringify(execution));
        if (request.action === 'attachSession')
          return { action: 'attachSession', execution: observe(execution) };
        return { action: 'prepareExecution', execution, created: true };
      }
      const executionId = text(request.executionId, 'execution ID', 128, true);
      const executionRow = this.db
        .prepare('SELECT snapshot FROM intent_executions WHERE id=? AND workspace_id=?')
        .get(executionId, id);
      if (!executionRow) throw new Error('Execution does not belong to this workspace');
      const execution = JSON.parse(String(executionRow.snapshot)) as IntentExecution;
      if (request.action === 'linkExecution') {
        const session = this.sessionRef(request.session);
        if (
          execution.session &&
          (execution.session.sessionId !== session.sessionId ||
            execution.session.hub !== session.hub)
        )
          throw new Error('Execution is already linked to another session');
        execution.session = session;
        execution.state = 'linked';
      } else if (request.action === 'markExecutionUnknown') {
        // A delayed error must not undo a successful link from another client.
        if (!execution.session) execution.state = 'unknown';
      } else throw new Error('Unknown execution action');
      execution.updatedAt = new Date().toISOString();
      persist(execution);
      if (request.action === 'linkExecution') this.automation.linked(execution);
      return { action: request.action, execution: observe(execution) };
    });
  }

  private sessionRef(input: unknown): IntentSessionRef {
    const raw = object(input);
    return {
      sessionId: text(raw.sessionId, 'Session ID', 256, true),
      hub: text(raw.hub, 'Hub ID', 256),
      label: text(raw.label, 'Agent name', 240, true),
      provider: text(raw.provider, 'Provider', 64, true),
      cwd: text(raw.cwd, 'Working directory', 4096, true),
    };
  }

  private record(workspace: IntentWorkspace, reason: string): void {
    this.db
      .prepare('INSERT INTO intent_revisions VALUES (?, ?, ?, ?, ?)')
      .run(
        workspace.id,
        workspace.revision,
        workspace.updatedAt,
        reason,
        JSON.stringify(workspace),
      );
  }
}

export interface CapturedIntentSession {
  sessionId: string;
  hub?: string;
  observation: IntentObservation;
  completionIdle?: boolean;
  finalReport?: { text: string; truncated: boolean; interrupted: boolean; redacted?: boolean };
}
const sessionKey = (session: { sessionId: string; hub?: string }) =>
  JSON.stringify([session.hub || '', session.sessionId]);
export function captureIntentSessions(
  sessions: readonly IntentLiveSession[],
  lifecycleSessions: readonly IntentLiveSession[] = sessions,
): CapturedIntentSession[] {
  const now = new Date().toISOString();
  return sessions
    .filter((session) => !session.hubOffline)
    .map((session) => ({
      sessionId: session.sessionId,
      hub: session.hub,
      observation: {
        ...intentObservation(session, now),
        completionIdle: !intentCompletionIdle(session, lifecycleSessions)
          ? false
          : session.conversation === undefined
            ? undefined
            : !captureFinalIntentReport(session.conversation).interrupted,
      },
      completionIdle: intentCompletionIdle(session, lifecycleSessions),
      ...(session.conversation !== undefined
        ? { finalReport: captureFinalIntentReport(session.conversation) }
        : {}),
    }));
}
export function captureFinalIntentReport(
  conversation: NonNullable<IntentLiveSession['conversation']>,
): NonNullable<CapturedIntentSession['finalReport']> {
  // Never reach backwards across a newer user message to reuse a previous outcome.
  const last = [...conversation]
    .reverse()
    .find((turn) => turn.role === 'assistant' || turn.role === 'user');
  const text = last?.role === 'assistant' ? last.content : '';
  const bounded = boundIntentReport(text);
  return {
    text: bounded.report,
    truncated: bounded.truncated,
    redacted: bounded.redacted,
    interrupted: conversation
      .slice(-2)
      .some((turn) => /\[Request interrupted by user\]/i.test(turn.content)),
  };
}
let store: Promise<IntentWorkspaceStore> | undefined;
let nextExistenceCheck = 0;
export async function intentWorkspaceStoreIfUsed(
  create = false,
): Promise<IntentWorkspaceStore | undefined> {
  if (!store && !create) {
    if (Date.now() < nextExistenceCheck) return undefined;
    nextExistenceCheck = Date.now() + 5000;
    if (!fs.existsSync(path.join(getConfigDir(), 'intent-workspaces.sqlite'))) return undefined;
  }
  if (!store)
    store = (async () => {
      const { DatabaseSync } = await import('node:sqlite');
      fs.mkdirSync(getConfigDir(), { recursive: true });
      const db = new DatabaseSync(path.join(getConfigDir(), 'intent-workspaces.sqlite'));
      try {
        return new IntentWorkspaceStore(db);
      } catch (error) {
        db.close();
        throw error;
      }
    })().catch((error) => {
      store = undefined;
      throw error;
    });
  return store;
}

export async function captureIntentWorkspaceSessions(
  sessions: readonly IntentLiveSession[],
  lifecycleSessions: readonly IntentLiveSession[] = sessions,
): Promise<void> {
  if (!store && Date.now() < nextExistenceCheck) return;
  // Detach before the asynchronous lazy open: native session rows mutate in place.
  const observations = captureIntentSessions(sessions, lifecycleSessions);
  const active = await intentWorkspaceStoreIfUsed();
  active?.capture(observations);
}

export async function intentWorkspaceRequest(
  input: unknown,
  sessions: readonly IntentLiveSession[] = [],
  deliver?: IntentDirectionDelivery,
  deliverControl?: IntentControlDelivery,
  effects?: IntentAutomationEffects,
): Promise<IntentWorkspaceResponse> {
  const request = object(input);
  const active = (await intentWorkspaceStoreIfUsed(true))!;
  if (process.platform === 'win32' && isMainThread && isIntentFileAction(request)) {
    const { runIntentFileRequest } = await import('./intentFileWorkerBroker');
    return runIntentFileRequest(
      path.join(getConfigDir(), 'intent-workspaces.sqlite'),
      request,
      sessions,
    );
  }
  if (request.action === 'importJiraIntent') return active.importJira(request);
  if ((SOURCE_ACTIONS as readonly string[]).includes(String(request.action)))
    return active.sources.request(request);
  if (request.action === 'captureEvidence') return active.evidence.capture(request, sessions);
  if (request.action === 'sendControl') {
    if (!deliverControl) throw new Error('This host does not support execution controls');
    return active.controls.send(request, sessions, deliverControl);
  }
  if (request.action === 'sendDirection') {
    if (!deliver) throw new Error('This host does not support direction delivery');
    return active.steering.send(request, sessions, deliver);
  }
  const result = active.request(input, sessions);
  if (effects) await active.automation.tick(sessions, effects);
  return result;
}
