import type { IntentSourceConnection } from './intentSources';

export interface IntentIntegrationDraft {
  provider: 'jira' | 'ado';
  name: string;
  baseUrl: string;
  defaultProjectKey: string;
  repository: string;
  credentialEnv: string;
  enabled: boolean;
}
export interface IntentIntegration extends IntentIntegrationDraft {
  id: string;
  projectRoot: string;
  version: number;
  deleted: boolean;
}
export interface IntentIntegrationView extends IntentIntegration {
  references: number;
}
export interface IntentIntegrationReference {
  integrationId: string;
  expectedIntegrationVersion: number;
  objectType: 'issue' | 'work-item' | 'pull-request';
  identifier: string;
  repository?: string;
}
export type IntentIntegrationRequest =
  | { action: 'integrations'; id: string }
  | {
      action: 'saveIntegration';
      id: string;
      integrationId: string;
      expectedVersion: number;
      operationId: string;
      integration: IntentIntegrationDraft;
    }
  | {
      action: 'removeIntegration';
      id: string;
      integrationId: string;
      expectedVersion: number;
      operationId: string;
    }
  | { action: 'previewSource'; id: string; reference: IntentIntegrationReference }
  | {
      action: 'attachSource';
      id: string;
      sourceId: string;
      expectedRevision: number;
      reference: IntentIntegrationReference;
    };
export type IntentIntegrationResponse =
  | { action: 'integrations'; integrations: IntentIntegrationView[] }
  | { action: 'saveIntegration' | 'removeIntegration'; integration: IntentIntegration }
  | { action: 'previewSource'; connection: IntentSourceConnection };

function text(value: unknown, label: string, optional = false): string {
  if (optional && (value === undefined || value === '')) return '';
  if (
    typeof value !== 'string' ||
    !value.trim() ||
    value.length > 256 ||
    /[\x00-\x1f\x7f]/.test(value)
  )
    throw new Error(`Invalid ${label}`);
  return value.trim();
}
function segment(value: unknown, label: string): string {
  const result = text(value, label);
  if (/[\\/%?#]/.test(result) || result === '.' || result === '..')
    throw new Error(`Invalid ${label}`);
  return result;
}
/** Shared preview validation; the host repeats this before any provider access. */
export function normalizeIntegration(value: unknown): IntentIntegrationDraft {
  const v = value as Partial<IntentIntegrationDraft> | null;
  if (!v || !['jira', 'ado'].includes(v.provider ?? ''))
    throw new Error('Unknown integration provider');
  const raw = text(v.baseUrl, 'integration base URL');
  // Validate raw syntax before URL normalization can erase dot segments, ports or backslashes.
  if (!/^https:\/\/[^/@:?#\\]+(?:\/[^?#\\]*)?$/.test(raw))
    throw new Error('Use a canonical HTTPS base URL without credentials, port, query or fragment');
  const url = new URL(raw);
  let baseUrl: string;
  if (v.provider === 'jira') {
    if (
      !/^[a-z0-9][a-z0-9-]*\.atlassian\.net$/.test(url.hostname) ||
      !/^https:\/\/[^/]+\/?$/.test(raw)
    )
      throw new Error('Use a Jira Cloud site base URL, such as https://team.atlassian.net');
    baseUrl = url.origin;
  } else {
    const match = raw.match(/^https:\/\/dev\.azure\.com\/([a-zA-Z0-9_-]+)\/([^/]+)\/?$/);
    if (!match) throw new Error('Use https://dev.azure.com/organization/project');
    let project: string;
    try {
      project = segment(decodeURIComponent(match[2]), 'Azure project');
    } catch {
      throw new Error('Invalid Azure project');
    }
    baseUrl = `https://dev.azure.com/${match[1]}/${encodeURIComponent(project)}`;
  }
  const credentialEnv = text(v.credentialEnv, 'credential environment name');
  if (credentialEnv.length > 128 || !/^WORKSPACER_SOURCE_[A-Z0-9_]+$/.test(credentialEnv))
    throw new Error('Credential name must match WORKSPACER_SOURCE_[A-Z0-9_]+');
  const defaultProjectKey =
    v.provider === 'jira'
      ? text(v.defaultProjectKey, 'default project key', true).toUpperCase()
      : '';
  if (defaultProjectKey && !/^[A-Z][A-Z0-9_]*$/.test(defaultProjectKey))
    throw new Error('Invalid default project key');
  const repository =
    v.provider === 'ado' && v.repository ? segment(v.repository, 'repository') : '';
  if (typeof v.enabled !== 'boolean') throw new Error('Invalid integration enabled flag');
  return {
    provider: v.provider!,
    name: text(v.name, 'connection name'),
    baseUrl,
    defaultProjectKey,
    repository,
    credentialEnv,
    enabled: v.enabled,
  };
}
export function resolveIntegration(
  integration: IntentIntegrationDraft,
  reference: Pick<IntentIntegrationReference, 'objectType' | 'identifier' | 'repository'>,
): IntentSourceConnection {
  const c = normalizeIntegration(integration);
  let identifier = text(reference.identifier, 'native identifier');
  let url: string;
  if (c.provider === 'jira') {
    if (reference.objectType !== 'issue') throw new Error('Choose Jira issue for this connection');
    identifier = identifier.toUpperCase();
    if (/^[1-9][0-9]*$/.test(identifier) && c.defaultProjectKey)
      identifier = `${c.defaultProjectKey}-${identifier}`;
    if (!/^[A-Z][A-Z0-9_]*-[1-9][0-9]*$/.test(identifier))
      throw new Error('Enter a Jira issue key such as TEAM-123');
    url = `${c.baseUrl}/browse/${identifier}`;
  } else {
    if (!['work-item', 'pull-request'].includes(reference.objectType))
      throw new Error('Choose an Azure work item or pull request');
    let repository = reference.repository || c.repository;
    if (reference.objectType === 'pull-request' && identifier.includes('#')) {
      const parts = identifier.split('#');
      if (parts.length !== 2 || (reference.repository && reference.repository !== parts[0]))
        throw new Error('Ambiguous repository; enter one repository and PR number');
      repository = parts[0];
      identifier = parts[1];
    }
    if (!/^[1-9][0-9]*$/.test(identifier)) throw new Error('Enter a positive numeric Azure ID');
    if (reference.objectType === 'pull-request') {
      if (!repository)
        throw new Error('A repository is required for pull requests; enter repository or repo#PR');
      url = `${c.baseUrl}/_git/${encodeURIComponent(segment(repository, 'repository'))}/pullrequest/${identifier}`;
    } else url = `${c.baseUrl}/_workitems/edit/${identifier}`;
  }
  return { provider: c.provider, url, credentialEnv: c.credentialEnv };
}
