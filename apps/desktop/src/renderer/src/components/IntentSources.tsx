import React, { useCallback, useEffect, useRef, useState } from 'react';
import { Surface } from './Surface';
import { inputStyle } from './settings/primitives';
import type { IntentWorkspace } from '../../../main/shared/intentWorkspace';
import {
  SOURCE_CAPABILITIES,
  type IntentSource,
  type IntentSourceComment,
  type IntentSourceDraft,
  type IntentSourceRequest,
  type IntentSourceResponse,
} from '../../../main/shared/intentSources';

export interface IntentSourceEditorDraft extends IntentSourceDraft {
  addRevision?: number;
  commentRevision?: number;
  commentSourceVersion?: number;
  publishAttempts?: Record<string, string>;
}
export const EMPTY_SOURCE_DRAFT: IntentSourceEditorDraft = {
  provider: 'manual',
  url: '',
  credentialEnv: '',
  title: '',
  content: '',
  sourceId: '',
  comment: '',
};
interface Props {
  workspace: IntentWorkspace;
  visible?: boolean;
  disabled?: boolean;
  draft?: IntentSourceEditorDraft;
  onDraftChange?: (draft: IntentSourceEditorDraft) => void;
}
const receiptLabel = {
  accepted: 'Comment accepted by provider',
  failed: 'Comment publishing failed',
  unknown: 'Comment publishing uncertain',
};
const field: React.CSSProperties = { ...inputStyle, width: '100%', boxSizing: 'border-box' };
export default function IntentSources({
  workspace,
  visible = true,
  disabled = false,
  draft: controlled,
  onDraftChange,
}: Props) {
  const [local, setLocal] = useState(EMPTY_SOURCE_DRAFT);
  const draft = controlled ?? local;
  const change = (patch: Partial<IntentSourceEditorDraft>) => {
    setNotice('');
    const next = { ...draft, ...patch };
    setLocal(next);
    onDraftChange?.(next);
  };
  const [sources, setSources] = useState<IntentSource[]>([]);
  const [comments, setComments] = useState<IntentSourceComment[]>([]);
  const [error, setError] = useState('');
  const [pending, setPending] = useState(false);
  const [loaded, setLoaded] = useState(false);
  const [loading, setLoading] = useState(false);
  const [loadError, setLoadError] = useState('');
  const [notice, setNotice] = useState('');
  const mounted = useRef(true);
  const busy = useRef(false);
  const visibility = useRef(visible);
  visibility.current = visible;
  useEffect(() => {
    mounted.current = true;
    return () => {
      mounted.current = false;
    };
  }, []);
  const generation = useRef(0);
  const active = useRef(workspace.id);
  active.current = workspace.id;
  const call = useCallback(async (request: IntentSourceRequest): Promise<IntentSourceResponse> => {
    if (!window.electronAPI.intentWorkspaceRequest)
      throw new Error('Update the host to use source integration.');
    return (await window.electronAPI.intentWorkspaceRequest(request)) as IntentSourceResponse;
  }, []);
  const load = useCallback(async () => {
    const sequence = ++generation.current;
    setLoading(true);
    try {
      const response = await call({ action: 'sources', id: workspace.id });
      if (!mounted.current || sequence !== generation.current || active.current !== workspace.id)
        return;
      if (
        response?.action !== 'sources' ||
        !Array.isArray(response.sources) ||
        !Array.isArray(response.comments)
      )
        throw new Error('Host does not support source integration.');
      setSources(response.sources);
      setComments(response.comments);
      setLoaded(true);
      setLoadError('');
    } catch (error) {
      if (mounted.current && sequence === generation.current && active.current === workspace.id) {
        setLoadError(error instanceof Error ? error.message : String(error));
        setLoaded(false);
      }
    } finally {
      if (mounted.current && sequence === generation.current && active.current === workspace.id)
        setLoading(false);
    }
  }, [call, workspace.id]);
  useEffect(() => {
    setSources([]);
    setComments([]);
    setLoaded(false);
    setError('');
    setPending(false);
    busy.current = false;
    setNotice('');
    setLoadError('');
    setLocal(EMPTY_SOURCE_DRAFT);
    return () => {
      generation.current++;
    };
  }, [load, workspace.id]);
  useEffect(() => {
    if (visible) void load();
    return () => {
      generation.current++;
    };
  }, [visible, load, workspace.id]);
  async function run(request: IntentSourceRequest, success?: () => void) {
    if (busy.current) return;
    busy.current = true;
    generation.current++;
    setLoading(false);
    setPending(true);
    setError('');
    setNotice('');
    try {
      const response = await call(request);
      if (response.action !== request.action) throw new Error('Unexpected host response');
      if (!mounted.current || active.current !== workspace.id) return;
      success?.();
      setNotice(
        request.action === 'publishSourceComment'
          ? 'Publishing attempt recorded. Check its receipt below.'
          : request.action === 'refreshSource'
            ? 'Source check completed. Review any changes below.'
            : 'Saved.',
      );
      if (visibility.current) await load();
    } catch (e) {
      if (mounted.current && active.current === workspace.id) {
        setError(e instanceof Error ? e.message : String(e));
        if (request.action === 'publishSourceComment' && visibility.current) await load();
      }
    } finally {
      if (mounted.current && active.current === workspace.id) {
        setPending(false);
        busy.current = false;
      }
    }
  }
  const blocked = disabled || pending || !loaded;
  const selected = sources.find((s) => s.id === draft.sourceId);
  return (
    <section
      hidden={!visible}
      aria-label="Workspace sources"
      style={{ display: visible ? 'grid' : undefined, gap: 16 }}
    >
      <div>
        <h3>Sources</h3>
        <p className="intent-muted">
          Keep accepted requirements beside your intent, then review changes before adopting them.
        </p>
        <button
          type="button"
          disabled={pending}
          onClick={() => {
            void load();
          }}
        >
          Reload sources
        </button>
      </div>
      {error && (
        <p role="alert" style={{ color: 'var(--wks-error)' }}>
          {error}
        </p>
      )}
      {loadError && (
        <p role="alert" className="intent-error">
          {loadError}
        </p>
      )}
      {(loading || pending || notice) && (
        <p role="status" className="intent-feedback">
          {pending ? 'Saving source changes…' : loading ? 'Loading sources…' : notice}
        </p>
      )}
      {disabled && (
        <p className="intent-muted">Save your intent changes before importing or publishing.</p>
      )}
      {!loaded && !loading && loadError && (
        <p className="intent-muted">
          Reload sources to enable source changes. Your draft is preserved.
        </p>
      )}
      <Surface elevation="flat" pad="md">
        <details open={!sources.length || !!draft.url || !!draft.content}>
          <summary>Add a source</summary>
          <form
            onSubmit={(e) => {
              e.preventDefault();
              if (blocked || busy.current) return;
              const sourceId = draft.addId ?? crypto.randomUUID();
              const expectedRevision = draft.addRevision ?? workspace.revision;
              change({ addId: sourceId, addRevision: expectedRevision });
              void run(
                {
                  action: 'addSource',
                  id: workspace.id,
                  sourceId,
                  expectedRevision,
                  connection: {
                    provider: draft.provider,
                    url: draft.url,
                    credentialEnv: draft.credentialEnv,
                  },
                  title: draft.title,
                  content: draft.content,
                },
                () =>
                  change({
                    addId: undefined,
                    addRevision: undefined,
                    url: '',
                    title: '',
                    content: '',
                  }),
              );
            }}
            style={{ display: 'grid', gap: 12 }}
          >
            <label>
              Source provider
              <select
                disabled={blocked}
                style={field}
                value={draft.provider}
                onChange={(e) =>
                  change({
                    provider: e.target.value as IntentSourceDraft['provider'],
                    addId: undefined,
                    addRevision: undefined,
                  })
                }
              >
                <option value="manual">Manual reference</option>
                <option value="jira">Jira Cloud</option>
                <option value="ado">Azure DevOps</option>
              </select>
            </label>
            <label>
              Source URL
              <input
                type="url"
                maxLength={4096}
                disabled={blocked}
                style={field}
                value={draft.url}
                onChange={(e) =>
                  change({ url: e.target.value, addId: undefined, addRevision: undefined })
                }
                required
                placeholder={
                  draft.provider === 'jira'
                    ? 'https://site.atlassian.net/browse/TEAM-123'
                    : draft.provider === 'ado'
                      ? 'https://dev.azure.com/org/project/_workitems/edit/123'
                      : 'https://…'
                }
              />
            </label>
            {draft.provider === 'manual' ? (
              <>
                <label>
                  Source title
                  <input
                    disabled={blocked}
                    style={field}
                    value={draft.title}
                    onChange={(e) =>
                      change({ title: e.target.value, addId: undefined, addRevision: undefined })
                    }
                  />
                </label>
                <label>
                  Source snapshot
                  <textarea
                    aria-label="Source snapshot"
                    disabled={blocked}
                    style={field}
                    rows={4}
                    value={draft.content}
                    onChange={(e) =>
                      change({ content: e.target.value, addId: undefined, addRevision: undefined })
                    }
                  />
                </label>
              </>
            ) : (
              <>
                <label>
                  Host credential environment name
                  <input
                    disabled={blocked}
                    style={field}
                    value={draft.credentialEnv}
                    placeholder="WORKSPACER_SOURCE_TEAM"
                    onChange={(e) =>
                      change({
                        credentialEnv: e.target.value,
                        addId: undefined,
                        addRevision: undefined,
                      })
                    }
                    required
                  />
                </label>
                <p className="intent-muted">
                  Set this variable on the owning host before starting Workspacer:{' '}
                  {draft.provider === 'jira'
                    ? 'email:API-token for this Jira site'
                    : 'a personal access token for this Azure organization'}
                  . Enter only the variable name here. The credential stays on the host. Import and
                  refresh read this ticket; comments require a separate review and Publish action.
                </p>
              </>
            )}
            <button type="submit" disabled={blocked}>
              Import source
            </button>
          </form>
        </details>
      </Surface>
      {sources.map((source) => (
        <Surface key={source.id} elevation="flat" pad="md">
          <h3>{source.accepted.title || source.nativeId}</h3>
          <a href={source.url} target="_blank" rel="noreferrer">
            {source.provider}: {source.nativeId}
          </a>
          <p className="intent-muted">
            Accepted revision {source.accepted.revision} · fetched {source.accepted.fetchedAt}
          </p>
          <pre style={{ whiteSpace: 'pre-wrap', overflowWrap: 'anywhere' }}>
            {source.accepted.content || 'No description recorded.'}
          </pre>
          <details>
            <summary>Accepted provider fields</summary>
            <pre style={{ whiteSpace: 'pre-wrap', overflowWrap: 'anywhere' }}>
              {JSON.stringify(source.accepted.fields, null, 2)}
            </pre>
          </details>
          {SOURCE_CAPABILITIES[source.provider].changeDetection && (
            <button
              type="button"
              disabled={blocked}
              onClick={() =>
                void run({
                  action: 'refreshSource',
                  id: workspace.id,
                  sourceId: source.id,
                  expectedVersion: source.version,
                })
              }
            >
              Check source for changes
            </button>
          )}
          {source.candidate && (
            <div
              style={{ borderLeft: '2px solid var(--wks-warning)', paddingLeft: 12, marginTop: 12 }}
            >
              <strong>Source changed — review candidate</strong>
              <p className="intent-muted">
                Revision {source.candidate.revision} · fetched {source.candidate.fetchedAt}
              </p>
              <p>{source.candidate.title}</p>
              <pre style={{ whiteSpace: 'pre-wrap', overflowWrap: 'anywhere' }}>
                {source.candidate.content}
              </pre>
              <details>
                <summary>Candidate provider fields</summary>
                <pre style={{ whiteSpace: 'pre-wrap', overflowWrap: 'anywhere' }}>
                  {JSON.stringify(source.candidate.fields, null, 2)}
                </pre>
              </details>
              <button
                type="button"
                disabled={blocked}
                onClick={() =>
                  void run({
                    action: 'acceptSource',
                    id: workspace.id,
                    sourceId: source.id,
                    expectedVersion: source.version,
                    candidateDigest: source.candidate!.digest,
                  })
                }
              >
                Accept reviewed source revision
              </button>
            </div>
          )}
          {!!source.history.length && (
            <details>
              <summary>Earlier accepted snapshots ({source.history.length})</summary>
              {source.history.map((snapshot, index) => (
                <div key={index}>
                  <p>
                    {snapshot.revision} · {snapshot.fetchedAt}
                  </p>
                  <pre style={{ whiteSpace: 'pre-wrap' }}>{JSON.stringify(snapshot, null, 2)}</pre>
                </div>
              ))}
            </details>
          )}
        </Surface>
      ))}
      {sources.some((s) => SOURCE_CAPABILITIES[s.provider].publishComment) && (
        <Surface elevation="flat" pad="md">
          <h3>Prepare a source comment</h3>
          <p className="intent-muted">
            This publishes only the exact comment you review. Ticket status stays unchanged. The
            host checks for source changes before posting; the provider cannot make that check and
            comment creation atomic.
          </p>
          <form
            style={{ display: 'grid', gap: 12 }}
            onSubmit={(e) => {
              e.preventDefault();
              if (blocked || busy.current) return;
              if (!selected) return;
              const commentId = draft.commentId ?? crypto.randomUUID();
              const expectedRevision = draft.commentRevision ?? workspace.revision;
              const expectedVersion = draft.commentSourceVersion ?? selected.version;
              change({
                commentId,
                commentRevision: expectedRevision,
                commentSourceVersion: expectedVersion,
              });
              void run(
                {
                  action: 'prepareSourceComment',
                  id: workspace.id,
                  sourceId: selected.id,
                  commentId,
                  expectedRevision,
                  expectedVersion,
                  text: draft.comment,
                },
                () =>
                  change({
                    commentId: undefined,
                    commentRevision: undefined,
                    commentSourceVersion: undefined,
                    comment: '',
                  }),
              );
            }}
          >
            <label>
              Comment source
              <select
                disabled={blocked}
                style={field}
                value={draft.sourceId}
                onChange={(e) =>
                  change({
                    sourceId: e.target.value,
                    commentId: undefined,
                    commentRevision: undefined,
                    commentSourceVersion: undefined,
                  })
                }
              >
                <option value="">Select a source</option>
                {sources
                  .filter((s) => SOURCE_CAPABILITIES[s.provider].publishComment)
                  .map((s) => (
                    <option key={s.id} value={s.id}>
                      {s.accepted.title || s.nativeId}
                    </option>
                  ))}
              </select>
            </label>
            <label>
              Comment text
              <textarea
                aria-label="Comment text"
                disabled={blocked}
                style={field}
                rows={4}
                value={draft.comment}
                onChange={(e) =>
                  change({
                    comment: e.target.value,
                    commentId: undefined,
                    commentRevision: undefined,
                    commentSourceVersion: undefined,
                  })
                }
              />
            </label>
            <button
              type="submit"
              disabled={blocked || !selected || !!selected.candidate || !draft.comment.trim()}
            >
              Save comment for review
            </button>
            {selected?.candidate && (
              <p className="intent-muted">
                Review and accept this source's changed snapshot before preparing a comment.
              </p>
            )}
          </form>
        </Surface>
      )}
      {comments.map((comment) => (
        <Surface key={comment.id} elevation="flat" pad="md">
          <h3>Review saved comment</h3>
          <p className="intent-muted">
            {sources.find((s) => s.id === comment.sourceId)?.url} · source revision{' '}
            {comment.sourceRevision} · intent revision {comment.intentRevision}
          </p>
          <pre style={{ whiteSpace: 'pre-wrap', overflowWrap: 'anywhere' }}>{comment.text}</pre>
          {comment.attempts.map((attempt) => (
            <p key={attempt.id}>
              {receiptLabel[attempt.status]} — {attempt.detail}
              {attempt.remoteId ? ` (comment ${attempt.remoteId})` : ''}
            </p>
          ))}
          {!comment.attempts.some((a) => a.status !== 'failed') && (
            <button
              type="button"
              disabled={blocked || comment.intentRevision !== workspace.revision}
              onClick={() => {
                if (busy.current) return;
                const prior = draft.publishAttempts?.[comment.id];
                const attemptId =
                  prior && !comment.attempts.some((a) => a.id === prior && a.status === 'failed')
                    ? prior
                    : crypto.randomUUID();
                change({ publishAttempts: { ...draft.publishAttempts, [comment.id]: attemptId } });
                void run({
                  action: 'publishSourceComment',
                  id: workspace.id,
                  commentId: comment.id,
                  attemptId,
                });
              }}
            >
              {comment.attempts.length
                ? 'Retry publishing reviewed comment'
                : 'Publish reviewed comment'}
            </button>
          )}
          {comment.intentRevision !== workspace.revision && (
            <p className="intent-muted">
              This comment uses an earlier intent revision. Prepare a new comment from the saved
              intent.
            </p>
          )}
        </Surface>
      ))}
    </section>
  );
}
