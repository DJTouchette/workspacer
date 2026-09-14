import React, { useEffect, useRef, useState } from 'react';
import type { IntentWorkspace } from '../../../main/shared/intentWorkspace';
import {
  resolveIntegration,
  type IntentIntegrationDraft,
  type IntentIntegrationView,
  type IntentIntegrationReference,
} from '../../../main/shared/intentIntegrations';
import type { IntentSourceRequest, IntentSourceResponse } from '../../../main/shared/intentSources';
import { inputStyle } from './settings/primitives';

const empty: IntentIntegrationDraft = {
  provider: 'jira',
  name: '',
  baseUrl: '',
  defaultProjectKey: '',
  repository: '',
  credentialEnv: '',
  enabled: true,
};
const field = { ...inputStyle, width: '100%', boxSizing: 'border-box' as const };
export default function IntentIntegrations({
  workspace,
  disabled,
  onAttached,
}: {
  workspace: IntentWorkspace;
  disabled: boolean;
  onAttached: () => void;
}) {
  const [connections, setConnections] = useState<IntentIntegrationView[]>([]);
  const [loaded, setLoaded] = useState(false);
  const [pending, setPending] = useState(false);
  const [error, setError] = useState('');
  const [notice, setNotice] = useState('');
  const [draft, setDraft] = useState(empty);
  const [editing, setEditing] = useState<{ id: string; version: number }>(() => ({
    id: crypto.randomUUID(),
    version: 0,
  }));
  const [selected, setSelected] = useState('');
  const [objectType, setObjectType] = useState<IntentIntegrationReference['objectType']>('issue');
  const [identifier, setIdentifier] = useState('');
  const [repository, setRepository] = useState('');
  const alive = useRef(true);
  const busy = useRef(false);
  const operations = useRef(new Map<string, string>());
  useEffect(() => {
    alive.current = true;
    return () => {
      alive.current = false;
    };
  }, []);
  const connection = connections.find((c) => c.id === selected && c.enabled);
  const reference: IntentIntegrationReference = {
    integrationId: selected,
    expectedIntegrationVersion: connection?.version ?? 0,
    objectType,
    identifier,
    repository,
  };
  let preview = '';
  let previewError = '';
  if (connection && identifier) {
    try {
      preview = resolveIntegration(connection, reference).url;
    } catch (e) {
      previewError = e instanceof Error ? e.message : String(e);
    }
  }
  function operation(value: unknown): string {
    const key = JSON.stringify(value);
    let id = operations.current.get(key);
    if (!id) {
      id = crypto.randomUUID();
      operations.current.set(key, id);
    }
    return id;
  }
  async function call(input: IntentSourceRequest): Promise<IntentSourceResponse> {
    if (!window.electronAPI.intentWorkspaceRequest)
      throw new Error('Update the host to use project integrations.');
    const result = (await window.electronAPI.intentWorkspaceRequest(input)) as IntentSourceResponse;
    if (result?.action !== input.action)
      throw new Error('Update the host to use project integrations.');
    return result;
  }
  async function load() {
    const result = await call({ action: 'integrations', id: workspace.id });
    if (alive.current && result.action === 'integrations') {
      setConnections(result.integrations);
      setLoaded(true);
    }
  }
  async function run(task: () => Promise<void>) {
    if (busy.current) return;
    busy.current = true;
    setPending(true);
    setError('');
    setNotice('');
    try {
      await task();
    } catch (e) {
      if (alive.current) setError(e instanceof Error ? e.message : String(e));
    } finally {
      busy.current = false;
      if (alive.current) setPending(false);
    }
  }
  function reset() {
    setDraft(empty);
    setEditing({ id: crypto.randomUUID(), version: 0 });
  }
  const blocked = disabled || pending || !loaded;
  return (
    <details
      onToggle={(e) => {
        if (e.currentTarget.open && !loaded) void run(load);
      }}
    >
      <summary>Project tracker integrations</summary>
      <p className="intent-muted">
        Configure a named connection once, then attach short references to any Intent in this
        project. Only the credential environment variable name is stored.
      </p>
      {error && <p role="alert">{error}</p>}
      {(pending || notice) && <p role="status">{pending ? 'Working…' : notice}</p>}
      <button type="button" disabled={pending} onClick={() => void run(load)}>
        Reload integrations
      </button>
      <form
        aria-label="Attach project source"
        style={{ display: 'grid', gap: 12 }}
        onSubmit={(e) => {
          e.preventDefault();
          if (blocked || !preview || !connection) return;
          const request = {
            action: 'attachSource' as const,
            id: workspace.id,
            expectedRevision: workspace.revision,
            reference,
          };
          void run(async () => {
            await call({ ...request, sourceId: operation(request) });
            if (!alive.current) return;
            setIdentifier('');
            setNotice('Source attached. Automatic synchronization is enabled.');
            onAttached();
            await load();
          });
        }}
      >
        <label>
          Named integration
          <select
            style={field}
            disabled={blocked}
            value={selected}
            onChange={(e) => {
              setSelected(e.target.value);
              setIdentifier('');
              setRepository('');
              setObjectType(
                connections.find((c) => c.id === e.target.value)?.provider === 'jira'
                  ? 'issue'
                  : 'work-item',
              );
            }}
          >
            <option value="">Choose a connection</option>
            {connections
              .filter((c) => c.enabled)
              .map((c) => (
                <option key={c.id} value={c.id}>
                  {c.name}
                </option>
              ))}
          </select>
        </label>
        <label>
          Object type
          <select
            style={field}
            disabled={blocked || !connection}
            value={objectType}
            onChange={(e) => setObjectType(e.target.value as typeof objectType)}
          >
            {connection?.provider === 'ado' ? (
              <>
                <option value="work-item">ADO work item</option>
                <option value="pull-request">ADO pull request</option>
              </>
            ) : (
              <option value="issue">Jira issue</option>
            )}
          </select>
        </label>
        <label>
          Native identifier
          <input
            style={field}
            disabled={blocked || !connection}
            value={identifier}
            onChange={(e) => setIdentifier(e.target.value)}
            required
            placeholder={
              objectType === 'issue'
                ? 'TEAM-123'
                : objectType === 'pull-request'
                  ? '123 or repo#123'
                  : '123'
            }
          />
        </label>
        {objectType === 'pull-request' && (
          <label>
            PR repository
            {connection?.repository
              ? ` (default: ${connection.repository})`
              : ' (required, or use repo#PR)'}
            <input
              style={field}
              disabled={blocked}
              value={repository}
              onChange={(e) => setRepository(e.target.value)}
            />
          </label>
        )}
        {previewError && <p role="status">{previewError}</p>}
        {preview && (
          <p>
            Source URL preview:{' '}
            <output aria-label="Canonical source URL" style={{ overflowWrap: 'anywhere' }}>
              {preview}
            </output>
          </p>
        )}
        <button type="submit" disabled={blocked || !preview}>
          Attach source
        </button>
      </form>
      <h4>Project connections</h4>
      <p className="intent-muted">
        Edits apply to future attachments. Existing sources keep their original URL and credential
        reference. Disabling prevents new attachments; existing sources continue to synchronize.
      </p>
      <ul style={{ display: 'grid', gap: 12, padding: 0, listStyle: 'none' }}>
        {connections.map((c) => (
          <li key={c.id} style={{ display: 'grid', gap: 8 }}>
            <div style={{ overflowWrap: 'anywhere' }}>
              <strong>{c.name}</strong> — {c.baseUrl} — {c.references} source reference
              {c.references !== 1 && 's'}
              {!c.enabled && ' — disabled'}
            </div>
            <div style={{ display: 'flex', flexWrap: 'wrap', gap: 8 }}>
              <button
                type="button"
                disabled={blocked}
                aria-label={`Edit ${c.name}`}
                onClick={() => {
                  setDraft(c);
                  setEditing({ id: c.id, version: c.version });
                }}
              >
                Edit
              </button>
              <button
                type="button"
                disabled={blocked}
                aria-label={`${c.enabled ? 'Disable' : 'Enable'} ${c.name}`}
                onClick={() =>
                  void run(async () => {
                    const request = {
                      action: 'saveIntegration' as const,
                      id: workspace.id,
                      integrationId: c.id,
                      expectedVersion: c.version,
                      integration: { ...c, enabled: !c.enabled },
                    };
                    await call({ ...request, operationId: operation(request) });
                    if (!alive.current) return;
                    await load();
                  })
                }
              >
                {c.enabled ? 'Disable' : 'Enable'}
              </button>
              <button
                type="button"
                disabled={blocked || c.references > 0}
                title={
                  c.references
                    ? 'Referenced connections can be disabled, but cannot be removed'
                    : 'Remove unused connection'
                }
                aria-label={`Remove ${c.name}`}
                onClick={() =>
                  void run(async () => {
                    const request = {
                      action: 'removeIntegration' as const,
                      id: workspace.id,
                      integrationId: c.id,
                      expectedVersion: c.version,
                    };
                    await call({ ...request, operationId: operation(request) });
                    if (!alive.current) return;
                    if (editing.id === c.id) reset();
                    await load();
                  })
                }
              >
                Remove
              </button>
            </div>
          </li>
        ))}
      </ul>
      <form
        aria-label="Project connection editor"
        style={{ display: 'grid', gap: 12 }}
        onSubmit={(e) => {
          e.preventDefault();
          if (blocked) return;
          const request = {
            action: 'saveIntegration' as const,
            id: workspace.id,
            integrationId: editing.id,
            expectedVersion: editing.version,
            integration: draft,
          };
          void run(async () => {
            await call({ ...request, operationId: operation(request) });
            if (!alive.current) return;
            reset();
            setNotice('Project connection saved.');
            await load();
          });
        }}
      >
        <h4>{editing.version ? 'Edit connection' : 'Create connection'}</h4>
        <label>
          Connection provider
          <select
            style={field}
            disabled={blocked}
            value={draft.provider}
            onChange={(e) =>
              setDraft({ ...empty, name: draft.name, provider: e.target.value as 'jira' | 'ado' })
            }
          >
            <option value="jira">Jira Cloud</option>
            <option value="ado">Azure DevOps</option>
          </select>
        </label>
        <label>
          Connection name
          <input
            style={field}
            disabled={blocked}
            required
            maxLength={256}
            value={draft.name}
            onChange={(e) => setDraft({ ...draft, name: e.target.value })}
          />
        </label>
        <label>
          {draft.provider === 'jira' ? 'Jira site base URL' : 'Azure organization/project URL'}
          <input
            type="url"
            style={field}
            disabled={blocked}
            required
            value={draft.baseUrl}
            placeholder={
              draft.provider === 'jira'
                ? 'https://team.atlassian.net'
                : 'https://dev.azure.com/org/project'
            }
            onChange={(e) => setDraft({ ...draft, baseUrl: e.target.value })}
          />
        </label>
        {draft.provider === 'jira' ? (
          <label>
            Default project key (optional)
            <input
              style={field}
              disabled={blocked}
              value={draft.defaultProjectKey}
              onChange={(e) => setDraft({ ...draft, defaultProjectKey: e.target.value })}
            />
          </label>
        ) : (
          <label>
            Default repository (optional)
            <input
              style={field}
              disabled={blocked}
              value={draft.repository}
              onChange={(e) => setDraft({ ...draft, repository: e.target.value })}
            />
          </label>
        )}
        <label>
          Connection credential environment name
          <input
            style={field}
            disabled={blocked}
            required
            pattern="WORKSPACER_SOURCE_[A-Z0-9_]+"
            placeholder="WORKSPACER_SOURCE_TEAM"
            value={draft.credentialEnv}
            onChange={(e) => setDraft({ ...draft, credentialEnv: e.target.value })}
          />
        </label>
        <label>
          <input
            type="checkbox"
            disabled={blocked}
            checked={draft.enabled}
            onChange={(e) => setDraft({ ...draft, enabled: e.target.checked })}
          />
          Enabled for new attachments
        </label>
        <button type="submit" disabled={blocked}>
          Save connection
        </button>
        {editing.version > 0 && (
          <button type="button" disabled={pending} onClick={reset}>
            Cancel edit
          </button>
        )}
      </form>
    </details>
  );
}
