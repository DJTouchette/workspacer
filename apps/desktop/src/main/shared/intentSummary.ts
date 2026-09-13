import type {
  IntentDirection,
  IntentExecution,
  IntentLiveSession,
  IntentSessionRef,
  IntentWorkspace,
} from './intentWorkspace';
import { intentCriteria, type IntentEvidenceResponse } from './intentEvidence';
import type { IntentSourceResponse } from './intentSources';
import type { IntentArtifactResponse } from './intentArtifacts';

export interface IntentSessionAttention {
  target: IntentSessionRef;
  approval?: string;
  questions: string[];
}
/** A projection of existing live pending slots, never a separate approval lifecycle. */
export function intentSessionAttention(
  refs: readonly IntentSessionRef[],
  sessions: readonly IntentLiveSession[],
): IntentSessionAttention[] {
  const found = new Set<string>();
  const attention: IntentSessionAttention[] = [];
  for (const target of refs) {
    const key = JSON.stringify([target.hub, target.sessionId]);
    if (found.has(key)) continue;
    found.add(key);
    const live = sessions.find(
      (session) => session.sessionId === target.sessionId && (session.hub || '') === target.hub,
    );
    if (!live || live.hubOffline || ['ended', 'stopped'].includes(live.status || '')) continue;
    const questions = (live.pendingQuestions ?? [])
      .map((question) => question.question)
      .filter(Boolean);
    if (live.pendingApproval || questions.length)
      attention.push({
        target,
        ...(live.pendingApproval
          ? { approval: live.pendingApproval.toolName || 'Tool request' }
          : {}),
        questions,
      });
  }
  return attention;
}
export interface IntentSummaryInput {
  executions?: IntentExecution[];
  evidence?: Extract<IntentEvidenceResponse, { action: 'evidence' }>;
  directions?: IntentDirection[];
  sources?: Extract<IntentSourceResponse, { action: 'sources' }>;
  artifacts?: Extract<IntentArtifactResponse, { action: 'artifacts' }>;
}
export function summarizeIntent(workspace: IntentWorkspace, input: IntentSummaryInput) {
  const criteria = intentCriteria(workspace.revision, workspace.successCriteria);
  const evidence =
    input.evidence?.evidence.filter((record) => record.intentRevision === workspace.revision) ?? [];
  const coverage = criteria.map((criterion) => {
    const records = evidence.filter((record) => record.criterion.id === criterion.id);
    return {
      criterion,
      records: records.length,
      verified: records.filter((record) => record.assessment === 'user-verified').length,
      reported: records.filter((record) => record.assessment === 'reported').length,
      unresolved: records.filter((record) => record.assessment === 'unresolved').length,
    };
  });
  const review = input.evidence?.reviews.find(
    (record) => record.intentRevision === workspace.revision,
  );
  const groups =
    input.artifacts?.groups.filter((group) => group.intentRevision === workspace.revision) ?? [];
  const alternatives = groups.map((group) => {
    const selection = input.artifacts?.selections.find(
      (record) => record.groupId === group.id && record.intentRevision === workspace.revision,
    );
    const selected =
      selection &&
      group.alternatives.find((alternative) => alternative.id === selection.alternativeId);
    return { group, selection, selected };
  });
  const directions = input.directions ?? [];
  return {
    coverage,
    verifiedCriteria: coverage.filter((criterion) => criterion.verified > 0).length,
    uncoveredCriteria: coverage.filter((criterion) => criterion.records === 0).length,
    unresolvedCriteria: coverage.filter((criterion) => criterion.unresolved > 0).length,
    review,
    alternatives,
    unselectedAlternatives: alternatives.filter((group) => !group.selected).length,
    unsentDirections: directions.filter(
      (direction) => !direction.supersededBy && direction.attempts.length === 0,
    ).length,
    uncertainDirections: directions.filter((direction) =>
      direction.attempts.some((attempt) => attempt.status === 'unknown'),
    ).length,
    uncertainPublications:
      input.sources?.comments.filter((comment) =>
        comment.attempts.some((attempt) => attempt.status === 'unknown'),
      ).length ?? 0,
    driftingSources:
      input.sources?.sources.filter(
        (source) => source.candidate && source.candidate.digest !== source.accepted.digest,
      ).length ?? 0,
    unconfirmedExecutions: input.executions?.filter((run) => !run.session).length ?? 0,
  };
}
