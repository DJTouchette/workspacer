import React, { useEffect, useState } from 'react';
import { RefreshCw, Link2 } from 'lucide-react';
import type { IntentIntegrationView } from '../../../main/shared/intentIntegrations';
import type { IntentWorkspace } from '../../../main/shared/intentWorkspace';

export default function IntentJiraImport({
  visible,
  defaultRoot,
  roots,
  onImported,
  onBusyChange,
}: {
  visible: boolean;
  defaultRoot: string;
  roots: string[];
  onImported: (workspace: IntentWorkspace) => void;
  onBusyChange: (busy: boolean) => void;
}) {
  const [projectRoot, setProjectRoot] = useState(defaultRoot);
  const [connections, setConnections] = useState<IntentIntegrationView[]>([]);
  const [integrationId, setIntegrationId] = useState('');
  const [identifier, setIdentifier] = useState('');
  const [loading, setLoading] = useState(false);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState('');
  const [reload, setReload] = useState(0);
  const [attempt, setAttempt] = useState<{ key: string; id: string } | null>(null);
  useEffect(() => {
    if (!visible) return;
    let alive = true;
    setLoading(true);
    setError('');
    setConnections([]);
    if (!projectRoot.trim()) {
      setLoading(false);
      return;
    }
    const api = window.electronAPI.intentWorkspaceRequest;
    if (!api) {
      setError('Jira import requires an updated Workspacer host.');
      setLoading(false);
      return;
    }
    void api({ action: 'jiraIntegrations', projectRoot })
      .then((result) => {
        if (!alive) return;
        if (result.action !== 'jiraIntegrations')
          throw new Error('Unexpected Jira connections response');
        setConnections(result.integrations);
        setIntegrationId((current) =>
          result.integrations.some((item) => item.id === current)
            ? current
            : result.integrations[0]?.id || '',
        );
      })
      .catch((err) => {
        if (alive) setError(err instanceof Error ? err.message : String(err));
      })
      .finally(() => {
        if (alive) setLoading(false);
      });
    return () => {
      alive = false;
    };
  }, [visible, projectRoot, reload]);
  if (!visible) return null;
  const connection = connections.find((item) => item.id === integrationId);
  return (
    <form
      className="intent-editor"
      onSubmit={async (event) => {
        event.preventDefault();
        if (busy || !connection) return;
        setBusy(true);
        onBusyChange(true);
        setError('');
        const key = JSON.stringify([projectRoot, integrationId, connection.version, identifier]);
        const operationId = attempt?.key === key ? attempt.id : crypto.randomUUID();
        setAttempt({ key, id: operationId });
        try {
          const response = await window.electronAPI.intentWorkspaceRequest?.({
            action: 'importJiraIntent',
            projectRoot,
            operationId,
            integrationId,
            expectedIntegrationVersion: connection.version,
            identifier,
          });
          if (response?.action !== 'importJiraIntent')
            throw new Error('Unexpected Jira import response');
          setIdentifier('');
          setAttempt(null);
          onImported(response.workspace);
        } catch (err) {
          setError(err instanceof Error ? err.message : String(err));
        } finally {
          setBusy(false);
          onBusyChange(false);
        }
      }}
    >
      <h2>From Jira</h2>
      <p className="intent-muted">
        Bring a ticket into Work as a Draft. Its summary and description become the starting
        requirements, and the original ticket stays attached in Sources. Review the outcome,
        constraints and success criteria before starting work.
      </p>
      {error && (
        <p role="alert" className="intent-error">
          {error}
        </p>
      )}
      <fieldset disabled={busy}>
        <label>
          Project directory
          <input
            required
            list="jira-project-roots"
            value={projectRoot}
            onChange={(event) => setProjectRoot(event.target.value)}
          />
        </label>
        <datalist id="jira-project-roots">
          {roots.map((root) => (
            <option key={root} value={root} />
          ))}
        </datalist>
        <label>
          Jira connection
          <select
            value={integrationId}
            onChange={(event) => setIntegrationId(event.target.value)}
            disabled={loading || !connections.length}
          >
            {!connections.length && (
              <option value="">{loading ? 'Loading connections…' : 'No Jira connections'}</option>
            )}
            {connections.map((item) => (
              <option key={item.id} value={item.id}>
                {item.name}
              </option>
            ))}
          </select>
        </label>
        {!loading && !connections.length && (
          <p className="intent-muted">
            Configure a Jira connection in an existing intent’s Sources → Connections for this
            project, then refresh connections here.
          </p>
        )}
        <label>
          Jira issue key or URL
          <input
            required
            value={identifier}
            onChange={(event) => setIdentifier(event.target.value)}
            placeholder="TEAM-123 or https://team.atlassian.net/browse/TEAM-123"
            maxLength={2048}
          />
        </label>
        <div className="intent-actions">
          <button
            type="submit"
            className="intent-primary"
            disabled={loading || !connection || !identifier.trim()}
          >
            <Link2 size={14} />
            {busy ? 'Importing…' : 'Import as draft'}
          </button>
          <button
            type="button"
            disabled={loading}
            onClick={() => setReload((current) => current + 1)}
          >
            <RefreshCw size={14} />
            Refresh connections
          </button>
        </div>
      </fieldset>
    </form>
  );
}
