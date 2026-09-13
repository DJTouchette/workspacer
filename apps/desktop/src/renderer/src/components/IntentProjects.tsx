import React, { useCallback, useEffect, useRef, useState } from 'react';
import { RefreshCw } from 'lucide-react';
import { Surface } from './Surface';
import type { IntentProject, IntentProjectRequest } from '../../../main/shared/intentProject';

export interface IntentProjectsDraft {
  projectId: string;
  action: 'renameIntentProject' | 'addIntentRepository' | 'relocateIntentRepository';
  repositoryId: string;
  name: string;
  root: string;
  reason: string;
  revision?: number;
}
const EMPTY: IntentProjectsDraft = {
  projectId: '',
  action: 'renameIntentProject',
  repositoryId: '',
  name: '',
  root: '',
  reason: '',
};
interface Props {
  visible: boolean;
  disabled?: boolean;
  onChanged?: () => void;
  draft?: IntentProjectsDraft;
  onDraftChange?: (draft: IntentProjectsDraft) => void;
}
export default function IntentProjects({
  visible,
  disabled = false,
  onChanged,
  draft: controlled,
  onDraftChange,
}: Props) {
  const [local, setLocal] = useState(EMPTY);
  const draft = controlled ?? local,
    change = onDraftChange ?? setLocal;
  const [projects, setProjects] = useState<IntentProject[]>([]);
  const [pending, setPending] = useState(false);
  const [error, setError] = useState('');
  const [loadError, setLoadError] = useState('');
  const [loading, setLoading] = useState(false),
    [loaded, setLoaded] = useState(false),
    [notice, setNotice] = useState('');
  const mounted = useRef(true),
    busy = useRef(false),
    visibility = useRef(visible);
  visibility.current = visible;
  useEffect(() => {
    mounted.current = true;
    return () => {
      mounted.current = false;
    };
  }, []);
  const sequence = useRef(0);
  const load = useCallback(async () => {
    const mine = ++sequence.current;
    setLoading(true);
    try {
      if (!window.electronAPI.intentWorkspaceRequest)
        throw new Error('Update the host to manage intent projects.');
      const result = await window.electronAPI.intentWorkspaceRequest({ action: 'projects' });
      if (!mounted.current || mine !== sequence.current) return;
      if (result.action !== 'projects')
        throw new Error('Update the host to manage intent projects.');
      setProjects(result.projects);
      setLoadError('');
      setLoaded(true);
    } catch (error) {
      if (mounted.current && mine === sequence.current)
        setLoadError(error instanceof Error ? error.message : String(error));
    } finally {
      if (mounted.current && mine === sequence.current) setLoading(false);
    }
  }, []);
  useEffect(() => {
    if (visible) void load();
    return () => {
      sequence.current++;
    };
  }, [visible, load]);
  if (!visible) return null;
  const project = projects.find((item) => item.id === draft.projectId);
  const edit = (patch: Partial<IntentProjectsDraft>) => {
    setNotice('');
    change({ ...draft, ...patch });
  };
  return (
    <Surface
      elevation="flat"
      className="intent-executions"
      role="region"
      aria-label="Intent projects"
    >
      <div className="intent-heading">
        <h3>Projects and repositories</h3>
        <button
          type="button"
          aria-label="Refresh intent projects"
          disabled={pending}
          onClick={() => void load()}
        >
          <RefreshCw size={14} />
        </button>
      </div>
      <p className="intent-muted">
        Group repositories under one project. Work created in any listed directory joins that
        project.
      </p>
      {(error || loadError) && (
        <p role="alert" className="intent-error">
          {error || loadError}
        </p>
      )}
      {error && loadError && (
        <p role="alert" className="intent-error">
          {loadError}
        </p>
      )}
      {(pending || loading || notice) && (
        <p role="status" className="intent-feedback">
          {pending ? 'Saving project changes…' : loading ? 'Loading projects…' : notice}
        </p>
      )}
      {disabled && (
        <p className="intent-inline-notice">
          Finish saving your work item before changing project settings.
        </p>
      )}
      {loaded && !projects.length && !loadError && (
        <p className="intent-muted">Create a work item to add your first project.</p>
      )}
      {projects.map((item) => (
        <article className="intent-execution" key={item.id}>
          <div className="intent-heading">
            <strong>{item.name}</strong>
            <button
              type="button"
              disabled={pending || disabled || !!project}
              onClick={() => {
                setNotice('');
                change({ ...EMPTY, projectId: item.id, name: item.name, revision: item.revision });
              }}
            >
              Manage {item.name}
            </button>
          </div>
          <details className="intent-record-details">
            <summary>Project identity</summary>
            <p className="intent-muted">
              Project {item.id} · Revision {item.revision}
            </p>
          </details>
          <ul>
            {item.repositories.map((repo) => (
              <li key={repo.id}>
                <span className="intent-root">{repo.root}</span>
                <details className="intent-record-details">
                  <summary>Repository identity</summary>
                  <p className="intent-muted">Repository {repo.id}</p>
                </details>
              </li>
            ))}
          </ul>
        </article>
      ))}
      {project && (
        <form
          onSubmit={async (event) => {
            event.preventDefault();
            if (busy.current) return;
            busy.current = true;
            setPending(true);
            setError('');
            setNotice('');
            sequence.current++;
            const saved = { ...draft, revision: draft.revision ?? project.revision };
            change(saved);
            const common = { projectId: saved.projectId, expectedRevision: saved.revision };
            const request: IntentProjectRequest =
              saved.action === 'renameIntentProject'
                ? { ...common, action: saved.action, name: saved.name }
                : saved.action === 'addIntentRepository'
                  ? { ...common, action: saved.action, root: saved.root }
                  : {
                      ...common,
                      action: saved.action,
                      repositoryId: saved.repositoryId,
                      root: saved.root,
                      reason: saved.reason,
                    };
            try {
              const result = await window.electronAPI.intentWorkspaceRequest!(request);
              if (!mounted.current) return;
              if (result.action !== request.action)
                throw new Error('Unexpected project response. Refresh before continuing.');
              change(EMPTY);
              setNotice(
                request.action === 'relocateIntentRepository'
                  ? 'Repository location updated. Related work items received a new revision.'
                  : 'Project saved.',
              );
              if (visibility.current) await load();
              onChanged?.();
            } catch (error) {
              if (!mounted.current) return;
              setError(error instanceof Error ? error.message : String(error));
              if (visibility.current) await load();
              onChanged?.();
            } finally {
              if (mounted.current) {
                setPending(false);
                busy.current = false;
              }
            }
          }}
        >
          <fieldset disabled={pending || disabled}>
            <h4>Manage {project.name}</h4>
            <label>
              Project change
              <select
                value={draft.action}
                onChange={(event) =>
                  edit({ action: event.target.value as IntentProjectsDraft['action'] })
                }
              >
                <option value="renameIntentProject">Rename project</option>
                <option value="addIntentRepository">Add repository</option>
                <option value="relocateIntentRepository">Relocate repository root</option>
              </select>
            </label>
            {draft.action === 'renameIntentProject' ? (
              <label>
                Project name
                <input
                  required
                  maxLength={240}
                  value={draft.name}
                  onChange={(event) => edit({ name: event.target.value })}
                />
              </label>
            ) : (
              <>
                {draft.action === 'relocateIntentRepository' && (
                  <label>
                    Repository to relocate
                    <select
                      required
                      value={draft.repositoryId}
                      onChange={(event) => edit({ repositoryId: event.target.value })}
                    >
                      <option value="">Choose a repository</option>
                      {project.repositories.map((repo) => (
                        <option key={repo.id} value={repo.id}>
                          {repo.root}
                        </option>
                      ))}
                    </select>
                  </label>
                )}
                <label>
                  {draft.action === 'addIntentRepository'
                    ? 'Repository root'
                    : 'New repository root'}
                  <input
                    required
                    maxLength={4096}
                    value={draft.root}
                    onChange={(event) => edit({ root: event.target.value })}
                    placeholder="Absolute directory on the connected host"
                  />
                </label>
                {draft.action === 'relocateIntentRepository' && (
                  <>
                    <label>
                      Reason for relocation
                      <textarea
                        aria-label="Reason for relocation"
                        required
                        rows={3}
                        maxLength={4000}
                        value={draft.reason}
                        onChange={(event) => edit({ reason: event.target.value })}
                      />
                    </label>
                    <p className="intent-muted">
                      Review the old and new root before saving. All work items in this repository
                      receive a new intent revision pointing to the new root. Files, existing agent
                      sessions, earlier launch packets, and configured fleet settings stay as
                      recorded.
                    </p>
                  </>
                )}
              </>
            )}
            {draft.revision !== undefined && draft.revision !== project.revision && (
              <div className="intent-inline-notice">
                <p>
                  This project changed elsewhere. Check the current repository locations above, then
                  review your preserved draft.
                </p>
                <button
                  type="button"
                  onClick={() => {
                    change({ ...draft, revision: project.revision });
                    setError('');
                    setNotice(
                      'Draft retained for the latest project revision. Review it before saving.',
                    );
                  }}
                >
                  Use latest project revision for this draft
                </button>
              </div>
            )}
            <div className="intent-actions">
              <button
                type="submit"
                disabled={draft.revision !== undefined && draft.revision !== project.revision}
              >
                {draft.action === 'relocateIntentRepository'
                  ? 'Save root relocation'
                  : draft.action === 'addIntentRepository'
                    ? 'Add repository'
                    : 'Save project name'}
              </button>
              <button type="button" onClick={() => change(EMPTY)}>
                Cancel project change
              </button>
            </div>
          </fieldset>
        </form>
      )}
    </Surface>
  );
}
