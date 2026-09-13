import type { IntentLiveSession, IntentWorkspaceResponse } from '../shared/intentWorkspace';

/** Only host-owner file operations cross this boundary. In particular there is
 * no direction/control delivery, arbitrary service dispatch, or source publish. */
export const INTENT_FILE_ACTIONS = new Set([
  'addArtifact',
  'readArtifact',
  'annotateArtifact',
  'createDemonstration',
  'captureEvidence',
  'readEvidence',
  'knowledge',
  'captureKnowledge',
  'prepareKnowledgePromotion',
  'publishKnowledgePromotion',
  'reconcileKnowledgePromotion',
]);
export const INTENT_FILE_DEADLINE_MS = 150_000;
export interface IntentFileWorkerRequest {
  token: number;
  request: Record<string, unknown>;
  sessions: IntentLiveSession[];
  expiresAt: number;
}
export type IntentFileWorkerResponse =
  { token: number; result: IntentWorkspaceResponse } | { token: number; error: string };
export function isIntentFileAction(input: Record<string, unknown>): boolean {
  return INTENT_FILE_ACTIONS.has(String(input.action));
}
const FILE_REQUEST_FIELDS: Record<string, string[]> = {
  addArtifact: [
    'id',
    'expectedRevision',
    'artifactId',
    'title',
    'mimeType',
    'dataBase64',
    'url',
    'versionOf',
    'executionId',
    'criterionId',
  ],
  readArtifact: ['id', 'artifactId'],
  annotateArtifact: [
    'id',
    'expectedRevision',
    'annotationId',
    'artifactId',
    'artifactSha256',
    'text',
    'point',
  ],
  createDemonstration: ['id', 'expectedRevision', 'demonstrationId', 'title', 'steps'],
  captureEvidence: ['id', 'expectedRevision', 'evidenceId', 'executionId', 'criterionId'],
  readEvidence: ['id', 'evidenceId'],
  knowledge: ['id'],
  captureKnowledge: ['id', 'expectedRevision', 'captureId', 'path', 'expectedSha256'],
  prepareKnowledgePromotion: ['id', 'expectedRevision', 'proposalId', 'findingId', 'kind', 'path'],
  publishKnowledgePromotion: ['id', 'proposalId'],
  reconcileKnowledgePromotion: ['id', 'proposalId'],
};
export function intentFileRequest(input: Record<string, unknown>): Record<string, unknown> {
  const action = String(input.action);
  const fields = FILE_REQUEST_FIELDS[action];
  if (!fields) throw new Error('Action is not an allowed intent file operation');
  const request = {
    action,
    ...Object.fromEntries(
      fields.filter((key) => input[key] !== undefined).map((key) => [key, input[key]]),
    ),
  };
  if (Buffer.byteLength(JSON.stringify(request), 'utf8') > 1024 * 1024)
    throw new Error('Intent file request exceeds its one MiB transport limit');
  return request;
}
export function intentFileSessions(sessions: readonly IntentLiveSession[]): IntentLiveSession[] {
  return sessions.map(({ sessionId, hub, hubOffline, status, cwd, liveCwd, label, provider }) => ({
    sessionId,
    hub,
    hubOffline,
    status,
    cwd,
    liveCwd,
    label,
    provider,
  }));
}
