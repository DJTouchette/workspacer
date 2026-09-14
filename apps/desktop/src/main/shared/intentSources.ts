/** Source requirements retain provider semantics and never overwrite personal intent. */
export type IntentSourceProvider = 'manual' | 'jira' | 'ado';
export const SOURCE_ACTIONS = [
  'sources',
  'sourceArtifacts',
  'addSource',
  'refreshSource',
  'acceptSource',
  'prepareSourceComment',
  'publishSourceComment',
] as const;
export interface IntentSourceConnection {
  provider: IntentSourceProvider;
  url: string;
  /** Name only. The host reads this environment variable at request time. */
  credentialEnv: string;
}
export interface IntentSourceSnapshot {
  revision: string;
  digest: string;
  fetchedAt: string;
  title: string;
  content: string;
  /** Original provider fields, including native status/hierarchy. */
  fields: Record<string, unknown>;
}
export interface IntentSource extends IntentSourceConnection {
  id: string;
  workspaceId: string;
  nativeId: string;
  version: number;
  accepted: IntentSourceSnapshot;
  candidate: IntentSourceSnapshot | null;
  history: IntentSourceSnapshot[];
  createdAt: string;
  /** Observations never imply intent lifecycle, acceptance, or verification. */
  external?: IntentExternalState;
}
export interface IntentSourceComment {
  id: string;
  workspaceId: string;
  sourceId: string;
  sourceRevision: string;
  sourceDigest: string;
  intentRevision: number;
  text: string;
  createdAt: string;
  attempts: Array<{
    id: string;
    status: 'accepted' | 'failed' | 'unknown';
    detail: string;
    at: string;
    remoteId?: string;
  }>;
}
export const SOURCE_CAPABILITIES: Record<
  IntentSourceProvider,
  { import: boolean; changeDetection: boolean; publishComment: boolean }
> = {
  manual: { import: true, changeDetection: false, publishComment: false },
  jira: { import: true, changeDetection: true, publishComment: true },
  ado: { import: true, changeDetection: true, publishComment: true },
};
export interface IntentSourceDraft extends IntentSourceConnection {
  addId?: string;
  commentId?: string;
  title: string;
  content: string;
  sourceId: string;
  comment: string;
}
export type IntentSourceRequest =
  | { action: 'sources'; id: string }
  | { action: 'sourceArtifacts'; id: string; sourceId: string; before?: number }
  | {
      action: 'addSource';
      id: string;
      sourceId: string;
      expectedRevision: number;
      connection: IntentSourceConnection;
      title?: string;
      content?: string;
    }
  | { action: 'refreshSource'; id: string; sourceId: string; expectedVersion: number }
  | {
      action: 'acceptSource';
      id: string;
      sourceId: string;
      expectedVersion: number;
      candidateDigest: string;
    }
  | {
      action: 'prepareSourceComment';
      id: string;
      sourceId: string;
      commentId: string;
      expectedRevision: number;
      expectedVersion: number;
      text: string;
    }
  | { action: 'publishSourceComment'; id: string; commentId: string; attemptId: string };
export type IntentSourceResponse =
  | { action: 'sourceArtifacts'; artifacts: IntentSourceArtifact[] }
  | { action: 'sources'; sources: IntentSource[]; comments: IntentSourceComment[] }
  | { action: 'addSource' | 'refreshSource' | 'acceptSource'; source: IntentSource }
  | { action: 'prepareSourceComment' | 'publishSourceComment'; comment: IntentSourceComment };

/** No lifecycle mapping: state and summary are provider-authored quoted data. */
export interface IntentExternalProjection {
  objectType: 'issue' | 'work-item' | 'pull-request';
  nativeId: string;
  url: string;
  state: string;
  revision: string;
  summary: Record<string, unknown>;
}
export interface IntentExternalState {
  projection: IntentExternalProjection | null;
  observedAt: string;
  lastSuccess: string | null;
  lastFailure: string | null;
  freshnessUntil: string;
  nextAttempt: number;
  failures: number;
  status: 'fresh' | 'partial' | 'error' | 'rate-limited' | 'missing';
  detail: string;
  etag?: string;
  reconciledAt?: string;
  artifactDigest?: string;
}
export interface IntentSourceArtifact {
  sequence: number;
  digest: string;
  observedAt: string;
  /** Full bounded, recursively redacted provider data; never instructions. */
  payload: Record<string, unknown>;
}
