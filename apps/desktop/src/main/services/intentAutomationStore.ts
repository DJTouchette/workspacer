import type { IntentDirectionDelivery } from './intentSteeringStore';
import type { DatabaseSync } from 'node:sqlite';
import { randomUUID } from 'node:crypto';
import { buildManagerKickoff } from '../shared/managerDoctrine';
import {
  intentRunInstructions,
  readIntentRunReport,
  type IntentRun,
} from '../shared/intentAutomation';
import type {
  IntentLiveSession,
  IntentWorkspace,
  IntentWorkspaceResponse,
  IntentSessionRef,
} from '../shared/intentWorkspace';
import type { CapturedIntentSession } from './intentWorkspaceStore';

export interface IntentAutomationEffects {
  spawn: (cwd: string, label: string, message: string) => Promise<IntentSessionRef>;
  send: IntentDirectionDelivery;
  interrupt: (session: IntentSessionRef) => Promise<{ status: string; detail: string }>;
}
export const INTENT_AUTOMATION_SCHEMA = `
CREATE TABLE IF NOT EXISTS intent_runs (workspace_id TEXT PRIMARY KEY REFERENCES intent_workspaces(id), snapshot TEXT NOT NULL);
CREATE TABLE IF NOT EXISTS intent_run_history (id TEXT PRIMARY KEY, workspace_id TEXT NOT NULL REFERENCES intent_workspaces(id), snapshot TEXT NOT NULL);
CREATE TABLE IF NOT EXISTS intent_status_events (id TEXT PRIMARY KEY, workspace_id TEXT NOT NULL REFERENCES intent_workspaces(id), at TEXT NOT NULL, status TEXT NOT NULL, reason TEXT NOT NULL);
`;
export class IntentAutomationStore {
  constructor(
    private db: DatabaseSync,
    private requestWorkspace: (input: unknown) => IntentWorkspaceResponse,
    private sendDirection: (
      input: Record<string, unknown>,
      sessions: readonly IntentLiveSession[],
      deliver: IntentDirectionDelivery,
    ) => Promise<IntentWorkspaceResponse>,
    private reportEvidence: (workspace: IntentWorkspace, run: IntentRun) => void,
  ) {}
  get(id: string): IntentRun | null {
    const row = this.db.prepare('SELECT snapshot FROM intent_runs WHERE workspace_id=?').get(id);
    return row ? JSON.parse(String(row.snapshot)) : null;
  }
  private save(run: IntentRun) {
    const prior = this.get(run.workspaceId);
    if (
      prior &&
      JSON.stringify({ ...prior, updatedAt: '' }) === JSON.stringify({ ...run, updatedAt: '' })
    )
      return;
    if (prior && prior.id !== run.id)
      this.db
        .prepare('INSERT OR IGNORE INTO intent_run_history VALUES (?, ?, ?)')
        .run(prior.id, prior.workspaceId, JSON.stringify(prior));
    run.updatedAt = new Date().toISOString();
    this.db
      .prepare(
        'INSERT INTO intent_runs VALUES (?, ?) ON CONFLICT(workspace_id) DO UPDATE SET snapshot=excluded.snapshot',
      )
      .run(run.workspaceId, JSON.stringify(run));
  }
  private claim(run: IntentRun, operation: NonNullable<IntentRun['operation']>): boolean {
    const expected = JSON.stringify(run);
    run.operation = operation;
    if (operation !== 'interrupt') run.state = 'starting';
    run.updatedAt = new Date().toISOString();
    return (
      this.db
        .prepare('UPDATE intent_runs SET snapshot=? WHERE workspace_id=? AND snapshot=?')
        .run(JSON.stringify(run), run.workspaceId, expected).changes === 1
    );
  }
  private workspace(id: string): IntentWorkspace {
    const row = this.db.prepare('SELECT snapshot FROM intent_workspaces WHERE id=?').get(id);
    if (!row) throw new Error('Workspace no longer exists');
    return JSON.parse(String(row.snapshot));
  }
  status(workspace: IntentWorkspace, status: IntentWorkspace['status'], reason: string) {
    workspace.status = status;
    workspace.updatedAt = new Date(
      Math.max(Date.now(), Date.parse(workspace.updatedAt) + 1),
    ).toISOString();
    this.db
      .prepare('UPDATE intent_workspaces SET snapshot=?, updated_at=? WHERE id=?')
      .run(JSON.stringify(workspace), workspace.updatedAt, workspace.id);
    this.db
      .prepare('INSERT INTO intent_status_events VALUES (?, ?, ?, ?, ?)')
      .run(randomUUID(), workspace.id, workspace.updatedAt, status, reason);
  }
  /** Existing preview Active records are not activated by migration or reads. */
  activate(workspace: IntentWorkspace, message = '', minutes = 60): IntentRun {
    if (!workspace.outcome.trim() || !workspace.successCriteria.trim())
      throw new Error('Add an outcome and success criteria before activating work.');
    if (!Number.isInteger(minutes) || minutes < 1 || minutes > 480)
      throw new Error('Work limit must be between 1 and 480 minutes.');
    const prior = this.get(workspace.id);
    if (prior && ['starting', 'uncertain'].includes(prior.state))
      throw new Error('Inspect the unconfirmed execution before activating again.');
    if (
      prior &&
      prior.intentRevision === workspace.revision &&
      ['queued', 'working', 'waiting'].includes(prior.state) &&
      !message
    )
      return prior;
    if (prior?.operation)
      throw new Error(
        'An execution request is still unconfirmed. Inspect the agent before continuing.',
      );
    const run: IntentRun = {
      id: randomUUID(),
      workspaceId: workspace.id,
      intentRevision: workspace.revision,
      executionId: prior?.session ? prior.executionId : randomUUID(),
      session: prior?.session ?? null,
      state: 'queued',
      message,
      report: '',
      deadline:
        prior && ['working', 'queued', 'waiting'].includes(prior.state)
          ? prior.deadline
          : new Date(Date.now() + minutes * 60_000).toISOString(),
      updatedAt: '',
    };
    if (prior?.session) {
      const row = this.db
        .prepare('SELECT snapshot FROM intent_executions WHERE id=?')
        .get(prior.executionId);
      run.baselineSummary = row
        ? JSON.parse(String(row.snapshot)).lastObservation?.summary
        : undefined;
    }
    if (!prior) {
      const linked = this.db
        .prepare(
          'SELECT snapshot FROM intent_executions WHERE workspace_id=? ORDER BY rowid DESC LIMIT 32',
        )
        .all(workspace.id)
        .map((row) => JSON.parse(String(row.snapshot)))
        .filter((execution) => execution.session)
        .map((execution) => execution.session);
      if (linked.length)
        run.message += `\nExisting linked sessions: ${JSON.stringify(linked)}. Inspect their work before dispatching; do not duplicate existing efforts.`;
    }
    this.status(workspace, 'active', 'Intent activated');
    this.save(run);
    return run;
  }
  pause(workspace: IntentWorkspace, reason = 'Paused by you') {
    const run = this.get(workspace.id);
    if (!run) return null;
    run.state = 'paused';
    run.report = reason;
    run.needsInterrupt = !!run.session;
    this.save(run);
    return run;
  }
  linked(execution: { workspaceId: string; id: string; session: IntentSessionRef | null }) {
    const run = this.get(execution.workspaceId);
    if (!run || run.executionId !== execution.id || !execution.session) return;
    run.session = execution.session;
    if (run.state === 'uncertain' && run.operation === 'spawn') {
      run.state = 'paused';
      delete run.operation;
      run.report = 'Launch linked. Inspect the conversation, then resume when ready.';
    }
    this.save(run);
  }
  review(id: string, decision: string, reason: string) {
    const workspace = this.workspace(id);
    if (!this.get(id)) return;
    if (decision === 'accept') {
      const run = this.get(id)!;
      if (!['review', 'paused', 'complete'].includes(run.state) || run.operation)
        throw new Error('Wait for the manager review or pause work before accepting.');
      this.status(workspace, 'complete', reason);
      run.state = 'complete';
      this.save(run);
    } else this.activate(workspace, `The user requested changes: ${reason}`);
  }
  request(input: Record<string, unknown>, sessions: readonly IntentLiveSession[] = []) {
    if (typeof input.id !== 'string') throw new Error('Invalid workspace ID');
    const workspace = this.workspace(input.id),
      run = this.get(workspace.id);
    if (input.action === 'automation') return { action: 'automation' as const, run };
    if (input.action !== 'activateIntent' && (!run || input.runId !== run.id))
      throw new Error('Work changed. Refresh before controlling it.');
    if (input.action === 'pauseIntent')
      return { action: 'pauseIntent' as const, run: this.pause(workspace) };
    if (input.expectedRevision !== workspace.revision)
      throw new Error('Intent changed. Save or refresh before activating.');
    if (input.action === 'resumeInspectedIntent') {
      if (!run?.session || !['uncertain', 'paused'].includes(run.state))
        throw new Error('Link and inspect the existing agent first.');
      if (typeof input.text !== 'string' || !input.text.trim() || input.text.length > 8000)
        throw new Error('Describe the result of your inspection and the next direction.');
      this.db
        .prepare('INSERT OR IGNORE INTO intent_run_history VALUES (?, ?, ?)')
        .run(run.id, run.workspaceId, JSON.stringify(run));
      run.state = 'paused';
      delete run.operation;
      run.needsInterrupt = false;
      this.save(run);
      return {
        action: 'resumeInspectedIntent' as const,
        run: this.activate(
          workspace,
          `The user inspected the earlier unconfirmed request. Do not replay it. Follow this new direction: ${input.text.trim()}`,
        ),
      };
    }
    if (input.action === 'restartIntent') {
      if (!run || run.state !== 'paused' || run.operation || !run.session)
        throw new Error('Inspect the existing run before replacing it.');
      const live = sessions.find(
        (s) => s.sessionId === run.session!.sessionId && (s.hub || '') === run.session!.hub,
      );
      const row = this.db
        .prepare('SELECT snapshot FROM intent_executions WHERE id=?')
        .get(run.executionId);
      const stopped = row && JSON.parse(String(row.snapshot)).lastObservation?.state === 'stopped';
      if ((live && !['ended', 'stopped'].includes(live.status || '')) || (!live && !stopped))
        throw new Error(
          'The previous manager has not been confirmed stopped. Open Execution and inspect it first.',
        );
      const predecessor = run.session;
      run.session = null;
      this.save(run);
      return {
        action: 'restartIntent' as const,
        run: this.activate(
          workspace,
          `The previous manager session ${predecessor.sessionId} ended. Inspect its execution history and reconcile only its workers before dispatching; do not duplicate existing work.`,
          input.minutes === undefined ? 60 : Number(input.minutes),
        ),
      };
    }
    if (input.action === 'answerIntent') {
      const live =
        run?.session &&
        sessions.find(
          (s) => s.sessionId === run.session!.sessionId && (s.hub || '') === run.session!.hub,
        );
      if (live && (live.pendingApproval || live.pendingQuestions?.length))
        throw new Error('Answer the pending question or approval in the agent conversation first.');
      if (typeof input.text !== 'string' || !input.text.trim() || input.text.length > 8000)
        throw new Error('Enter an answer of at most 8000 characters.');
      if (!run || !['waiting', 'paused', 'review'].includes(run.state))
        throw new Error('This run is not waiting for an answer.');
      return {
        action: 'answerIntent' as const,
        run: this.activate(workspace, `The user says: ${input.text.trim()}`),
      };
    }
    return {
      action: 'activateIntent' as const,
      run: this.activate(workspace, '', input.minutes === undefined ? 60 : Number(input.minutes)),
    };
  }
  observe(samples: readonly CapturedIntentSession[]) {
    for (const row of this.db.prepare('SELECT snapshot FROM intent_runs').all()) {
      const run = JSON.parse(String(row.snapshot)) as IntentRun;
      if (!run.session || !['working', 'waiting'].includes(run.state)) continue;
      const sample = samples.find(
        (s) => s.sessionId === run.session!.sessionId && (s.hub || '') === run.session!.hub,
      );
      if (!sample) continue;
      const workspace = this.workspace(run.workspaceId);
      if (workspace.revision !== run.intentRevision) continue;
      const observation = sample.observation;
      const report = ['idle', 'stopped'].includes(observation.state)
        ? readIntentRunReport(observation.summary, run)
        : undefined;
      if (report) {
        run.state = report.state;
        run.report = report.summary;
        if (report.state === 'review') {
          this.reportEvidence(workspace, run);
          this.status(workspace, 'review', 'Agent reported work ready for review');
        }
      } else if (observation.state === 'stopped') {
        run.state = 'paused';
        run.report = 'The agent ended. Inspect its execution before starting replacement work.';
      } else if (['blocked', 'waiting-input', 'approval'].includes(observation.state)) {
        run.state = 'waiting';
        run.report = observation.summary;
      } else if (['thinking', 'streaming', 'background'].includes(observation.state)) {
        run.state = 'working';
        run.report = '';
      }
      this.save(run);
    }
  }
  /** Claim before I/O. Crashed or ambiguous effects never replay on restart. */
  async tick(sessions: readonly IntentLiveSession[], effects: IntentAutomationEffects) {
    for (const row of this.db.prepare('SELECT snapshot FROM intent_runs').all()) {
      let run = this.get((JSON.parse(String(row.snapshot)) as IntentRun).workspaceId)!;
      const workspace = this.workspace(run.workspaceId);
      if (
        run.intentRevision !== workspace.revision &&
        ['working', 'queued', 'waiting'].includes(run.state)
      )
        run = this.pause(
          workspace,
          'The saved requirements or project root changed. Review the new intent before resuming.',
        )!;
      if (
        ['working', 'queued', 'waiting'].includes(run.state) &&
        Date.now() >= Date.parse(run.deadline)
      )
        run = this.pause(workspace, 'Time limit reached. Review progress and resume when ready.')!;
      if (run.operation) {
        if (run.state === 'starting' && Date.now() - Date.parse(run.updatedAt) > 120_000) {
          run.state = 'uncertain';
          run.report =
            'The execution request did not receive a durable acknowledgment. Inspect and link the existing agent; this request will not replay.';
          this.save(run);
        }
        continue;
      }
      if (run.needsInterrupt && run.session) {
        if (!this.claim(run, 'interrupt')) continue;
        try {
          const notice = await effects.send(
            run.session,
            'The user has paused this intent. Stop your outstanding workers and wait for an explicit user resume. Worker wakes are not authorization to continue.',
          );
          const result = await effects.interrupt(run.session);
          if (notice.status !== 'accepted')
            result.detail +=
              ' Pause instruction delivery is unconfirmed; inspect the conversation.';
          const latest = this.get(run.workspaceId)!;
          if (latest.id !== run.id) continue;
          latest.needsInterrupt = false;
          if (result.status === 'accepted') delete latest.operation;
          else latest.report += ` ${result.detail}`;
          this.save(latest);
        } catch {
          run.report += ' Interrupt is unconfirmed; inspect the session.';
          this.save(run);
        }
        continue;
      }
      // A manager sometimes ends a turn without the routing report. Continue
      // at most three times, only after a new idle reply and no outstanding
      // descendants. Never poll an LLM while delegated workers are running.
      if (
        run.state === 'working' &&
        run.session &&
        Date.now() - Date.parse(run.updatedAt) > 10_000
      ) {
        const manager = sessions.find(
          (s) => s.sessionId === run.session!.sessionId && (s.hub || '') === run.session!.hub,
        );
        const children = new Set([run.session.sessionId]);
        for (let changed = true; changed;) {
          changed = false;
          for (const child of sessions)
            if (
              (child.hub || '') === run.session.hub &&
              child.parentSessionId &&
              children.has(child.parentSessionId) &&
              !children.has(child.sessionId)
            ) {
              children.add(child.sessionId);
              changed = true;
            }
        }
        const outstanding = sessions.some(
          (s) =>
            s.sessionId !== run.session!.sessionId &&
            (s.hub || '') === run.session!.hub &&
            children.has(s.sessionId) &&
            !['ended', 'stopped'].includes(s.status || '') &&
            (s.ambientState !== 'idle' || s.pendingApproval || s.pendingQuestions?.length),
        );
        const execution = this.db
          .prepare('SELECT snapshot FROM intent_executions WHERE id=?')
          .get(run.executionId);
        const summary =
          execution && JSON.parse(String(execution.snapshot)).lastObservation?.summary;
        if (
          manager?.ambientState === 'idle' &&
          !manager.pendingApproval &&
          !manager.pendingQuestions?.length &&
          !outstanding &&
          summary &&
          summary !== run.baselineSummary
        ) {
          if ((run.automaticContinuations || 0) >= 3) {
            run.state = 'waiting';
            run.report =
              'The manager stopped without a question or review report after three continuations. Inspect its conversation and provide direction.';
          } else {
            run.id = randomUUID();
            run.state = 'queued';
            run.automaticContinuations = (run.automaticContinuations || 0) + 1;
            run.message =
              'Continue authorized work if success criteria remain incomplete. If ready for review or you need the user, provide the intent-report described above. Do not end merely to offer to continue.';
          }
          run.baselineSummary = summary;
          this.save(run);
        }
      }
      if (run.state !== 'queued' || workspace.status !== 'active') continue;
      if (run.session) {
        const live = sessions.find(
          (s) => s.sessionId === run.session!.sessionId && (s.hub || '') === run.session!.hub,
        );
        if (!live || live.hubOffline || ['ended', 'stopped'].includes(live.status || '')) {
          run.state = 'paused';
          run.report = 'The linked agent is unavailable. Inspect Execution before resuming.';
          this.save(run);
          continue;
        }
      }
      if (!this.claim(run, run.session ? 'send' : 'spawn')) continue;
      try {
        let packet: string;
        if (!run.session) {
          const result = this.requestWorkspace({
            action: 'prepareExecution',
            id: workspace.id,
            expectedRevision: workspace.revision,
            executionId: run.executionId,
            task: buildManagerKickoff(intentRunInstructions(run)),
          });
          if (result.action !== 'prepareExecution')
            throw new Error('Could not prepare intent execution');
          if (!result.created) {
            run.state = 'uncertain';
            run.report = 'Launch already claimed. Inspect and link its existing agent.';
            this.save(run);
            continue;
          }
          packet = result.execution.contextPacket!;
        } else {
          const direction = this.requestWorkspace({
            action: 'prepareDirection',
            id: workspace.id,
            expectedRevision: workspace.revision,
            directionId: run.id,
            executionId: run.executionId,
            text: intentRunInstructions(run),
          });
          if (direction.action !== 'prepareDirection')
            throw new Error('Could not prepare continuation');
          packet = direction.direction.packet;
        }

        if (!run.session) {
          run.session = await effects.spawn(
            workspace.projectRoot,
            `Intent: ${workspace.title}`,
            packet,
          );
          this.requestWorkspace({
            action: 'linkExecution',
            id: workspace.id,
            executionId: run.executionId,
            session: run.session,
          });
        } else {
          const sent = await this.sendDirection(
            { action: 'sendDirection', id: workspace.id, directionId: run.id, attemptId: run.id },
            sessions,
            effects.send,
          );
          const receipt =
            sent.action === 'sendDirection' ? sent.direction.attempts.at(-1) : undefined;
          if (receipt?.status === 'failed') {
            const latest = this.get(run.workspaceId)!;
            if (latest.id === run.id) {
              latest.state = 'paused';
              latest.report = receipt.detail;
              delete latest.operation;
              this.save(latest);
            }
            continue;
          }
          if (receipt?.status !== 'accepted')
            throw new Error(receipt?.detail || 'No confirmed continuation receipt');
        }
        const latest = this.get(run.workspaceId)!;
        if (latest.id !== run.id) continue;
        latest.session = run.session;
        delete latest.operation;
        if (latest.state === 'starting' || latest.state === 'uncertain') {
          latest.state = 'working';
          latest.report = '';
        }
        if (latest.state === 'paused') latest.needsInterrupt = true;
        this.save(latest);
      } catch (error) {
        const latest = this.get(run.workspaceId)!;
        if (latest.id !== run.id) continue;
        latest.session = run.session;
        latest.state = 'uncertain';
        latest.report = `Execution request is unconfirmed. Inspect the linked agent before retrying. ${String(error)}`;
        this.save(latest);
      }
    }
  }
}
