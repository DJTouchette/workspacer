import React, { useCallback, useEffect, useRef, useState } from 'react';
import { RefreshCw } from 'lucide-react';
import { Surface } from './Surface';
import type { IntentKnowledgeResponse } from '../../../main/shared/intentKnowledge';
import type { IntentWorkspace, IntentWorkspaceRequest } from '../../../main/shared/intentWorkspace';
export interface IntentKnowledgeDraft {
  title: string;
  observation: string;
  captureIds: string[];
  findingId?: string;
  findingRevision?: number;
  proposalFindingId: string;
  proposalKind: 'learning' | 'context';
  proposalPath: string;
  proposalId?: string;
  proposalRevision?: number;
  captureRequests?: Record<string, string>;
}
const EMPTY: IntentKnowledgeDraft = {
  title: '',
  observation: '',
  captureIds: [],
  proposalFindingId: '',
  proposalKind: 'learning',
  proposalPath: '',
};
type Listing = Extract<IntentKnowledgeResponse, { action: 'knowledge' }>;
interface Props {
  workspace: IntentWorkspace;
  visible: boolean;
  disabled: boolean;
  draft?: IntentKnowledgeDraft;
  onDraftChange: (draft: IntentKnowledgeDraft) => void;
}
export default function IntentKnowledge({
  workspace,
  visible,
  disabled,
  draft = EMPTY,
  onDraftChange,
}: Props) {
  const [data, setData] = useState<Listing>({
    action: 'knowledge',
    available: false,
    documents: [],
    captures: [],
    findings: [],
    proposals: [],
  });
  const [pending, setPending] = useState(false),
    [error, setError] = useState(''),
    [loadError, setLoadError] = useState('');
  const generation = useRef(0);
  const mounted = useRef(true);
  const active = useRef(workspace.id);
  const busy = useRef(false);
  const visibility = useRef(visible);
  active.current = workspace.id;
  visibility.current = visible;
  const [loading, setLoading] = useState(false);
  const [loaded, setLoaded] = useState(false);
  const [notice, setNotice] = useState('');
  useEffect(() => {
    mounted.current = true;
    return () => {
      mounted.current = false;
    };
  }, []);
  const call = useCallback((request: IntentWorkspaceRequest) => {
    if (!window.electronAPI.intentWorkspaceRequest)
      throw new Error('Update the host to use project knowledge');
    return window.electronAPI.intentWorkspaceRequest(request);
  }, []);
  const load = useCallback(async () => {
    const mine = ++generation.current;
    setLoading(true);
    try {
      const result = await call({ action: 'knowledge', id: workspace.id });
      if (!mounted.current || active.current !== workspace.id || mine !== generation.current)
        return;
      if (result.action !== 'knowledge')
        throw new Error('Update the host to use project knowledge');
      setData(result);
      setLoadError('');
      setLoaded(true);
    } catch (error) {
      if (mounted.current && active.current === workspace.id && mine === generation.current)
        setLoadError(error instanceof Error ? error.message : String(error));
    } finally {
      if (mounted.current && active.current === workspace.id && mine === generation.current)
        setLoading(false);
    }
  }, [workspace.id, call]);
  useEffect(() => {
    setData({
      action: 'knowledge',
      available: false,
      documents: [],
      captures: [],
      findings: [],
      proposals: [],
    });
    setLoaded(false);
    setPending(false);
    setError('');
    setNotice('');
    busy.current = false;
  }, [workspace.id]);
  useEffect(() => {
    if (visible) void load();
    return () => {
      generation.current++;
    };
  }, [visible, workspace.revision, load]);
  async function mutate(request: IntentWorkspaceRequest) {
    if (busy.current) return false;
    busy.current = true;
    generation.current++;
    setLoading(false);
    setPending(true);
    setError('');
    setNotice('');
    try {
      const result = await call(request);
      if (!mounted.current || active.current !== workspace.id) return false;
      if (result.action !== request.action) throw new Error('Unexpected knowledge response');
      setNotice(
        request.action === 'publishKnowledgePromotion' ||
          request.action === 'reconcileKnowledgePromotion'
          ? 'Project write receipt updated below.'
          : 'Saved to this work item.',
      );
      if (visibility.current) await load();
      return true;
    } catch (error) {
      if (!mounted.current || active.current !== workspace.id) return false;
      setError(error instanceof Error ? error.message : String(error));
      if (request.action === 'publishKnowledgePromotion' && visibility.current) await load();
      return false;
    } finally {
      if (mounted.current && active.current === workspace.id) {
        setPending(false);
        busy.current = false;
      }
    }
  }
  if (!visible) return null;
  const edit = (patch: Partial<IntentKnowledgeDraft>) => {
    setNotice('');
    const findingChanged = ['title', 'observation', 'captureIds'].some((key) => key in patch);
    const promotionChanged = ['proposalFindingId', 'proposalKind', 'proposalPath'].some(
      (key) => key in patch,
    );
    onDraftChange({
      ...draft,
      ...patch,
      ...(findingChanged ? { findingId: undefined, findingRevision: undefined } : {}),
      ...(promotionChanged ? { proposalId: undefined, proposalRevision: undefined } : {}),
    });
  };
  return (
    <Surface
      elevation="flat"
      className="intent-executions"
      role="region"
      aria-label="Project knowledge"
    >
      <div className="intent-heading">
        <h3>Project knowledge</h3>
        <button
          type="button"
          disabled={pending}
          aria-label="Refresh project knowledge"
          onClick={() => void load()}
        >
          <RefreshCw size={14} />
        </button>
      </div>
      <p className="intent-muted">
        Capture versions of Rivet context for this work. Findings stay with the feature until you
        review an explicit project learning or context promotion.
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
          {pending ? 'Saving knowledge changes…' : loading ? 'Loading project knowledge…' : notice}
        </p>
      )}
      {disabled && (
        <p className="intent-muted">
          Save your intent changes before recording findings or publishing knowledge.
        </p>
      )}
      {!data.available && loaded && !loadError && (
        <p className="intent-muted">
          No Rivet context documents are available in this repository. You can still record findings
          and prepare a learning file.
        </p>
      )}
      {data.documents.map((document) => {
        const saved = data.captures.find((capture) => capture.path === document.path);
        return (
          <article key={document.path} className="intent-execution">
            <strong>{document.title}</strong>
            <p className="intent-root">{document.path}</p>
            <p className="intent-muted">
              {document.bytes} bytes
              {saved
                ? saved.sha256 === document.sha256
                  ? ' · Captured version is current'
                  : ' · Changed since your last capture'
                : ''}
            </p>
            <button
              type="button"
              disabled={disabled || pending || saved?.sha256 === document.sha256}
              onClick={() => {
                if (busy.current) return;
                const key = `${workspace.revision}:${document.path}:${document.sha256}`;
                const captureId = draft.captureRequests?.[key] || crypto.randomUUID();
                onDraftChange({
                  ...draft,
                  captureRequests: { ...draft.captureRequests, [key]: captureId },
                });
                void mutate({
                  action: 'captureKnowledge',
                  id: workspace.id,
                  captureId,
                  expectedRevision: workspace.revision,
                  path: document.path,
                  expectedSha256: document.sha256,
                });
              }}
            >
              Capture document version
            </button>
          </article>
        );
      })}
      {!!data.captures.length && (
        <details>
          <summary>Captured knowledge ({data.captures.length})</summary>
          <p className="intent-muted">
            The latest capture of each document accompanies future execution packets. Existing
            packets keep their original content.
          </p>
          {data.captures.map((capture) => (
            <article key={capture.id} className="intent-execution">
              <strong>{capture.title}</strong>
              <p className="intent-muted">
                Revision {capture.intentRevision} · {new Date(capture.capturedAt).toLocaleString()}
              </p>
              <p className="intent-root">
                {capture.path} · SHA-256 {capture.sha256}
              </p>
              <details>
                <summary>Read captured version</summary>
                <pre className="intent-packet">{capture.content}</pre>
              </details>
            </article>
          ))}
        </details>
      )}
      <h3>Reusable finding</h3>
      <form
        onSubmit={async (event) => {
          event.preventDefault();
          if (busy.current) return;
          const next = {
            ...draft,
            findingId: draft.findingId || crypto.randomUUID(),
            findingRevision: draft.findingRevision ?? workspace.revision,
          };
          onDraftChange(next);
          if (
            await mutate({
              action: 'recordFinding',
              id: workspace.id,
              expectedRevision: next.findingRevision,
              findingId: next.findingId,
              title: next.title,
              observation: next.observation,
              captureIds: next.captureIds,
            })
          )
            onDraftChange({
              ...EMPTY,
              captureRequests: next.captureRequests,
              proposalFindingId: next.findingId,
            });
        }}
      >
        <fieldset disabled={disabled || pending}>
          <label>
            Finding title
            <input
              required
              maxLength={240}
              value={draft.title}
              onChange={(event) => edit({ title: event.target.value })}
            />
          </label>
          <label>
            What should future work know?
            <textarea
              aria-label="What should future work know?"
              required
              rows={4}
              maxLength={16000}
              value={draft.observation}
              onChange={(event) => edit({ observation: event.target.value })}
            />
          </label>
          {data.captures.map((capture) => (
            <label className="intent-check" key={capture.id}>
              <input
                type="checkbox"
                checked={draft.captureIds.includes(capture.id)}
                onChange={(event) =>
                  edit({
                    captureIds: event.target.checked
                      ? [...draft.captureIds, capture.id]
                      : draft.captureIds.filter((id) => id !== capture.id),
                  })
                }
              />
              {capture.path} · {capture.sha256.slice(0, 12)}
            </label>
          ))}
          <button type="submit" disabled={!draft.title.trim() || !draft.observation.trim()}>
            Record feature finding
          </button>
        </fieldset>
      </form>
      {data.findings.map((finding) => (
        <details key={finding.id} className="intent-execution">
          <summary>{finding.title}</summary>
          <p className="intent-muted">
            By you · Revision {finding.intentRevision} ·{' '}
            {new Date(finding.createdAt).toLocaleString()}
          </p>
          <p className="intent-result">{finding.observation}</p>
        </details>
      ))}
      <h3>Promote deliberately</h3>
      <form
        onSubmit={async (event) => {
          event.preventDefault();
          if (busy.current) return;
          const next = {
            ...draft,
            proposalId: draft.proposalId || crypto.randomUUID(),
            proposalRevision: draft.proposalRevision ?? workspace.revision,
          };
          onDraftChange(next);
          if (
            await mutate({
              action: 'prepareKnowledgePromotion',
              id: workspace.id,
              proposalId: next.proposalId,
              findingId: next.proposalFindingId,
              expectedRevision: next.proposalRevision,
              kind: next.proposalKind,
              ...(next.proposalKind === 'context' ? { path: next.proposalPath } : {}),
            })
          )
            onDraftChange({
              ...draft,
              proposalId: undefined,
              proposalRevision: undefined,
              proposalFindingId: '',
            });
        }}
      >
        <fieldset disabled={disabled || pending}>
          <label>
            Finding to promote
            <select
              required
              value={draft.proposalFindingId}
              onChange={(event) => edit({ proposalFindingId: event.target.value })}
            >
              <option value="">Choose a recorded finding</option>
              {data.findings.map((finding) => (
                <option key={finding.id} value={finding.id}>
                  {finding.title}
                </option>
              ))}
            </select>
          </label>
          <label>
            Destination
            <select
              value={draft.proposalKind}
              onChange={(event) =>
                edit({ proposalKind: event.target.value as 'learning' | 'context' })
              }
            >
              <option value="learning">Rivet learning log · awaiting curation</option>
              <option value="context">Append to curated project context</option>
            </select>
          </label>
          {draft.proposalKind === 'context' && (
            <label>
              Context document
              <select
                required
                value={draft.proposalPath}
                onChange={(event) => edit({ proposalPath: event.target.value })}
              >
                <option value="">Choose a context document</option>
                {data.documents.map((document) => (
                  <option key={document.path} value={document.path}>
                    {document.title}
                  </option>
                ))}
              </select>
            </label>
          )}
          <button
            type="submit"
            disabled={
              !draft.proposalFindingId || (draft.proposalKind === 'context' && !draft.proposalPath)
            }
          >
            Prepare promotion for review
          </button>
        </fieldset>
      </form>
      {data.proposals.map((proposal) => (
        <article key={proposal.id} className="intent-execution">
          <strong>
            {proposal.kind === 'learning' ? 'Learning capture' : 'Curated context promotion'}
          </strong>
          <p className="intent-root">{proposal.path}</p>
          <p role="status">
            {proposal.status === 'draft'
              ? 'Prepared · No project file changed'
              : proposal.status === 'written'
                ? 'Reviewed content was written'
                : 'Write outcome uncertain'}
          </p>
          <p className="intent-muted">{proposal.detail}</p>
          <details open={proposal.status === 'draft'}>
            <summary>Exact proposed file content</summary>
            <pre className="intent-packet">{proposal.content}</pre>
          </details>
          {proposal.status === 'draft' && (
            <button
              type="button"
              disabled={disabled || pending}
              onClick={() =>
                void mutate({
                  action: 'publishKnowledgePromotion',
                  id: workspace.id,
                  proposalId: proposal.id,
                })
              }
            >
              {proposal.kind === 'learning'
                ? 'Write reviewed learning file'
                : 'Apply reviewed context promotion'}
            </button>
          )}
          {proposal.status === 'unknown' && (
            <button
              type="button"
              disabled={pending}
              onClick={() =>
                void mutate({
                  action: 'reconcileKnowledgePromotion',
                  id: workspace.id,
                  proposalId: proposal.id,
                })
              }
            >
              Verify file against proposal
            </button>
          )}
        </article>
      ))}
    </Surface>
  );
}
