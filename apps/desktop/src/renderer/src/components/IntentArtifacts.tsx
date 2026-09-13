import React, { useCallback, useEffect, useId, useRef, useState } from 'react';
import DOMPurify from 'dompurify';
import { RefreshCw } from 'lucide-react';
import { Surface } from './Surface';
import { intentCriteria } from '../../../main/shared/intentEvidence';
import {
  INTENT_ARTIFACT_LIMITS,
  INTENT_ARTIFACT_MIMES,
  type IntentArtifact,
  type IntentArtifactMime,
  type IntentArtifactRequest,
  type IntentArtifactResponse,
  type IntentAlternative,
} from '../../../main/shared/intentArtifacts';
import type { IntentWorkspace, IntentExecution } from '../../../main/shared/intentWorkspace';

interface Props {
  workspace: IntentWorkspace;
  visible: boolean;
  disabled: boolean;
  onOpenUrl?: (url: string) => void;
}
type Listing = Extract<IntentArtifactResponse, { action: 'artifacts' }>;
const EMPTY: Listing = {
  action: 'artifacts',
  artifacts: [],
  annotations: [],
  demonstrations: [],
  groups: [],
  selections: [],
};
const alternative = (): IntentAlternative => ({
  id: crypto.randomUUID(),
  title: '',
  hypothesis: '',
  artifactIds: [],
  executionIds: [],
});
const decode = (base64: string) =>
  new TextDecoder().decode(Uint8Array.from(atob(base64), (c) => c.charCodeAt(0)));
/** Static, opaque-origin preview: uploaded HTML cannot navigate, run scripts, or make network requests. */
export function staticArtifactHtml(html: string): string {
  const safe = DOMPurify.sanitize(html, {
    ALLOWED_TAGS: [
      'div',
      'span',
      'p',
      'h1',
      'h2',
      'h3',
      'h4',
      'h5',
      'h6',
      'strong',
      'em',
      'b',
      'i',
      'u',
      's',
      'br',
      'hr',
      'ul',
      'ol',
      'li',
      'pre',
      'code',
      'blockquote',
      'table',
      'thead',
      'tbody',
      'tr',
      'td',
      'th',
      'section',
      'article',
      'header',
      'footer',
      'main',
      'nav',
      'aside',
      'figure',
      'figcaption',
      'img',
      'style',
    ],
    ALLOWED_ATTR: ['class', 'style', 'alt', 'src', 'width', 'height', 'colspan', 'rowspan'],
    ALLOW_DATA_ATTR: false,
  });
  return `<!doctype html><html><head><meta http-equiv="Content-Security-Policy" content="default-src 'none'; img-src data:; style-src 'unsafe-inline'; form-action 'none'; base-uri 'none'; object-src 'none'; frame-src 'none'"><meta name="referrer" content="no-referrer"></head><body>${safe}</body></html>`;
}
function fileBytes(file: File): Promise<string> {
  return new Promise((resolve, reject) => {
    const reader = new FileReader();
    reader.onerror = () => reject(new Error('Could not read the selected file'));
    reader.onload = () => resolve(String(reader.result).split(',')[1]);
    reader.readAsDataURL(file);
  });
}
function mediaType(file: File): IntentArtifactMime {
  const extension = file.name.split('.').pop()?.toLowerCase();
  const known: Record<string, IntentArtifactMime> = {
    png: 'image/png',
    jpg: 'image/jpeg',
    jpeg: 'image/jpeg',
    webp: 'image/webp',
    gif: 'image/gif',
    txt: 'text/plain',
    md: 'text/markdown',
    markdown: 'text/markdown',
    html: 'text/html',
    htm: 'text/html',
  };
  const mime = known[extension || ''] ?? file.type;
  if (!INTENT_ARTIFACT_MIMES.includes(mime as IntentArtifactMime))
    throw new Error('Choose PNG, JPEG, WebP, GIF, UTF-8 text, Markdown, or HTML');
  return mime as IntentArtifactMime;
}

export default function IntentArtifacts({ workspace, visible, disabled, onOpenUrl }: Props) {
  const annotationHelp = useId();
  const [records, setRecords] = useState<Listing>(EMPTY);
  const [executions, setExecutions] = useState<IntentExecution[]>([]);
  const [error, setError] = useState('');
  const [loadError, setLoadError] = useState('');
  const [pending, setPending] = useState(false);
  const [selectedId, setSelectedId] = useState('');
  const [preview, setPreview] = useState<{ id: string; data: string }>();
  const [previewError, setPreviewError] = useState('');
  const [title, setTitle] = useState('');
  const [url, setUrl] = useState('');
  const [file, setFile] = useState<File>();
  const [versionOf, setVersionOf] = useState('');
  const [executionId, setExecutionId] = useState('');
  const [criterionId, setCriterionId] = useState('');
  const [mode, setMode] = useState<'view' | 'annotate'>('view');
  const [annotationDrafts, setAnnotationDrafts] = useState<
    Record<string, { note: string; point?: { x: number; y: number } }>
  >({});
  const [demoTitle, setDemoTitle] = useState('');
  const [steps, setSteps] = useState<{ artifactId: string; caption: string }[]>([]);
  const [groupTitle, setGroupTitle] = useState('');
  const [purpose, setPurpose] = useState('');
  const [budget, setBudget] = useState(30);
  const [alternatives, setAlternatives] = useState<IntentAlternative[]>(() => [
    alternative(),
    alternative(),
  ]);
  const [choices, setChoices] = useState<
    Record<string, { alternativeId: string; reason: string; expectedSelectionId?: string }>
  >({});
  const [loading, setLoading] = useState(false),
    [loaded, setLoaded] = useState(false);
  const identity = useRef(workspace.id),
    epoch = useRef(0),
    mounted = useRef(true),
    operation = useRef<number | null>(null);
  const mutationIds = useRef(new Map<string, { key: string; id: string }>());
  if (identity.current !== workspace.id) {
    identity.current = workspace.id;
    epoch.current++;
  }
  const owner = epoch.current;
  const currentScope = () => mounted.current && epoch.current === owner;
  useEffect(() => {
    mounted.current = true;
    return () => {
      mounted.current = false;
    };
  }, []);
  useEffect(() => {
    setPending(false);
    operation.current = null;
    setRecords(EMPTY);
    setExecutions([]);
    setLoaded(false);
    setError('');
    setLoadError('');
    mutationIds.current.clear();
    setTitle('');
    setUrl('');
    setFile(undefined);
    setVersionOf('');
    setExecutionId('');
    setCriterionId('');
    setAnnotationDrafts({});
    setDemoTitle('');
    setSteps([]);
    setGroupTitle('');
    setPurpose('');
    setBudget(30);
    setAlternatives([alternative(), alternative()]);
    setChoices({});
  }, [workspace.id]);
  const sequence = useRef(0);
  const previewSequence = useRef(0);
  const fileInput = useRef<HTMLInputElement>(null);
  const call = useCallback((request: IntentArtifactRequest) => {
    if (!window.electronAPI.intentWorkspaceRequest)
      throw new Error('Update the host to use saved artifacts');
    return window.electronAPI.intentWorkspaceRequest(request);
  }, []);
  const load = useCallback(async () => {
    const mine = ++sequence.current;
    setLoading(true);
    try {
      const [result, runs] = await Promise.all([
        call({ action: 'artifacts', id: workspace.id }),
        window.electronAPI.intentWorkspaceRequest?.({ action: 'executions', id: workspace.id }),
      ]);
      if (mine !== sequence.current || !currentScope()) return;
      if (result.action !== 'artifacts' || runs?.action !== 'executions')
        throw new Error('Update the host to use saved artifacts');
      setRecords(result);
      setExecutions(runs.executions);
      setLoadError('');
      setLoaded(true);
    } catch (error) {
      if (mine === sequence.current && currentScope())
        setLoadError(error instanceof Error ? error.message : String(error));
    } finally {
      if (mine === sequence.current && currentScope()) setLoading(false);
    }
  }, [workspace.id, call]);
  useEffect(() => {
    if (visible) void load();
    return () => {
      sequence.current++;
    };
  }, [visible, workspace.revision, load]);
  useEffect(() => {
    setSelectedId('');
    setPreview(undefined);
    setMode('view');
  }, [workspace.id]);
  const selected = records.artifacts.find((a) => a.id === selectedId);
  const annotationKey = `${workspace.id}:${selected?.id}:${selected?.sha256}`;
  const annotationDraft = annotationDrafts[annotationKey] ?? { note: '' };
  const { note, point } = annotationDraft;
  const setNote = (note: string) =>
    setAnnotationDrafts((previous) => ({
      ...previous,
      [annotationKey]: { ...(previous[annotationKey] ?? { note: '' }), note },
    }));
  const setPoint = (point: { x: number; y: number } | undefined) =>
    setAnnotationDrafts((previous) => ({
      ...previous,
      [annotationKey]: { ...(previous[annotationKey] ?? { note: '' }), point },
    }));
  useEffect(() => {
    const mine = ++previewSequence.current;
    setPreview(undefined);
    setPreviewError('');
    if (visible && selected && selected.kind !== 'url')
      void call({ action: 'readArtifact', id: workspace.id, artifactId: selected.id })
        .then((result) => {
          if (mine !== previewSequence.current) return;
          if (
            result.action !== 'readArtifact' ||
            result.artifact.id !== selected.id ||
            result.artifact.sha256 !== selected.sha256
          )
            throw new Error('Artifact response changed; reload before previewing');
          if (selected.kind === 'text' || selected.kind === 'html') decode(result.dataBase64);
          setPreview({ id: selected.id, data: result.dataBase64 });
        })
        .catch((error) => {
          if (mine === previewSequence.current)
            setPreviewError(error instanceof Error ? error.message : String(error));
        });
    return () => {
      previewSequence.current++;
    };
  }, [visible, selected?.id, selected?.sha256, workspace.id, call]);
  async function mutate(request: IntentArtifactRequest): Promise<boolean> {
    if (!currentScope() || operation.current === owner) return false;
    operation.current = owner;
    setPending(true);
    setError('');
    sequence.current++;
    const idFields: Record<string, string> = {
      addArtifact: 'artifactId',
      annotateArtifact: 'annotationId',
      createDemonstration: 'demonstrationId',
      createAlternativeGroup: 'groupId',
      selectAlternative: 'selectionId',
    };
    const idField = idFields[request.action];
    const raw = request as unknown as Record<string, unknown>;
    const slot = `${request.action}:${request.action === 'selectAlternative' ? raw.groupId : request.action === 'annotateArtifact' ? raw.artifactId : ''}`;
    // Reuse an unchanged request's identity after a lost response.
    const key = JSON.stringify({ ...raw, [idField]: undefined });
    const prior = mutationIds.current.get(slot);
    const recordId = prior?.key === key ? prior.id : String(raw[idField]);
    mutationIds.current.set(slot, { key, id: recordId });
    try {
      const result = await call({ ...request, [idField]: recordId } as IntentArtifactRequest);
      if (!currentScope()) return false;
      if (result.action !== request.action)
        throw new Error('Unexpected artifact response; reload before continuing');
      mutationIds.current.delete(slot);
      if (result.action === 'addArtifact') setSelectedId(result.artifact.id);
      await load();
      return currentScope();
    } catch (error) {
      if (currentScope()) setError(error instanceof Error ? error.message : String(error));
      return false;
    } finally {
      if (currentScope()) {
        setPending(false);
        operation.current = null;
      }
    }
  }
  const choose = (artifactId: string) => {
    setSelectedId(artifactId);
    setMode('view');
  };
  const current = records.artifacts.filter((a) => a.intentRevision === workspace.revision);
  const criteria = intentCriteria(workspace.revision, workspace.successCriteria);
  const staleCriterion =
    !!criterionId && !criteria.some((criterion) => criterion.id === criterionId);
  const staleSteps = steps.some(
    (step) => !current.some((artifact) => artifact.id === step.artifactId),
  );
  const staleAlternativeLinks = alternatives.some((choice) =>
    choice.artifactIds.some((id) => !current.some((artifact) => artifact.id === id)),
  );
  const write = { id: workspace.id, expectedRevision: workspace.revision };
  const locked = disabled || pending || !loaded;
  const patchAlternative = (index: number, patch: Partial<IntentAlternative>) =>
    setAlternatives((items) => items.map((a, i) => (i === index ? { ...a, ...patch } : a)));
  if (!visible) return null;
  return (
    <Surface
      elevation="flat"
      className="intent-executions"
      role="region"
      aria-label="Intent artifacts"
    >
      <div className="intent-heading">
        <h3>Artifacts and exploration</h3>
        <button
          type="button"
          onClick={() => void load()}
          disabled={pending || loading}
          aria-label="Refresh artifacts"
        >
          <RefreshCw size={14} />
        </button>
      </div>
      <p className="intent-muted">
        Save a version to review, annotate it, or compare alternatives. References open only when
        you choose them.
      </p>
      {loading && !loaded && (
        <p role="status" className="intent-muted">
          Loading artifacts…
        </p>
      )}
      {loadError && (
        <p role="alert" className="intent-feedback intent-error">
          {loadError}
        </p>
      )}
      {error && (
        <p role="alert" className="intent-feedback intent-error">
          {error}
        </p>
      )}
      <details className="intent-record-details">
        <summary>Add an artifact or version</summary>
        <form
          onSubmit={async (event) => {
            event.preventDefault();
            if (locked) return;
            setError('');
            if (!file && !url.trim()) {
              setError('Choose a file or enter a URL');
              return;
            }
            setPending(true);
            try {
              const artifactId = crypto.randomUUID();
              const payload = file
                ? { mimeType: mediaType(file), dataBase64: await fileBytes(file) }
                : { url: url.trim() };
              if (
                await mutate({
                  action: 'addArtifact',
                  ...write,
                  artifactId,
                  title,
                  ...payload,
                  ...(versionOf ? { versionOf } : {}),
                  ...(executionId ? { executionId } : {}),
                  ...(criterionId ? { criterionId } : {}),
                })
              ) {
                setTitle('');
                setUrl('');
                setFile(undefined);
                setVersionOf('');
                if (fileInput.current) fileInput.current.value = '';
              }
            } catch (error) {
              if (currentScope()) setError(error instanceof Error ? error.message : String(error));
            } finally {
              if (currentScope()) setPending(false);
            }
          }}
        >
          <fieldset disabled={locked}>
            <label>
              Artifact title
              <input
                required
                maxLength={240}
                value={title}
                onChange={(e) => setTitle(e.target.value)}
              />
            </label>
            <label>
              Upload file (up to 512 KiB)
              <input
                ref={fileInput}
                type="file"
                accept=".png,.jpg,.jpeg,.webp,.gif,.txt,.md,.markdown,.html,.htm"
                onChange={(e) => {
                  const selected = e.target.files?.[0];
                  if (selected && selected.size > INTENT_ARTIFACT_LIMITS.bytes) {
                    setError('Artifact exceeds 512 KiB');
                    e.target.value = '';
                    setFile(undefined);
                    return;
                  }
                  setFile(selected);
                  if (selected) {
                    setUrl('');
                    if (!title) setTitle(selected.name);
                  }
                }}
              />
            </label>
            <label>
              Or URL reference
              <input
                type="url"
                value={url}
                disabled={!!file}
                onChange={(e) => setUrl(e.target.value)}
                placeholder="https://example.com/preview"
              />
            </label>
            {file && (
              <button
                type="button"
                onClick={() => {
                  setFile(undefined);
                  if (fileInput.current) fileInput.current.value = '';
                }}
              >
                Clear selected file
              </button>
            )}
            <label>
              Previous artifact version
              <select value={versionOf} onChange={(e) => setVersionOf(e.target.value)}>
                <option value="">New artifact</option>
                {records.artifacts.map((a) => (
                  <option key={a.id} value={a.id}>
                    {a.title} · revision {a.intentRevision}
                  </option>
                ))}
              </select>
            </label>
            <label>
              Related execution
              <select value={executionId} onChange={(e) => setExecutionId(e.target.value)}>
                <option value="">No execution link</option>
                {executions.map((run) => (
                  <option key={run.id} value={run.id}>
                    {run.session?.label || run.task || run.id}
                  </option>
                ))}
              </select>
            </label>
            <label>
              Related success criterion
              <select value={criterionId} onChange={(e) => setCriterionId(e.target.value)}>
                <option value="">No criterion link</option>
                {staleCriterion && (
                  <option value={criterionId} disabled>
                    Earlier intent criterion — choose again
                  </option>
                )}
                {criteria.map((c) => (
                  <option key={c.id} value={c.id}>
                    {c.text}
                  </option>
                ))}
              </select>
            </label>
            {staleCriterion && (
              <p className="intent-inline-notice">
                The intent changed. Choose a current success criterion or remove the criterion link
                before saving.
              </p>
            )}
            <button
              type="submit"
              disabled={!title.trim() || (!file && !url.trim()) || staleCriterion}
            >
              Save artifact version
            </button>
          </fieldset>
        </form>
      </details>
      {!records.artifacts.length && loaded && !loadError && (
        <p className="intent-muted">No saved artifacts yet.</p>
      )}
      {records.artifacts.length > 0 && (
        <label>
          Saved artifact
          <select value={selectedId} disabled={pending} onChange={(e) => choose(e.target.value)}>
            <option value="">Choose an artifact</option>
            {records.artifacts.map((a) => (
              <option key={a.id} value={a.id}>
                {a.title} · {a.kind} · intent {a.intentRevision}
                {a.versionOf ? ' · new version' : ''}
              </option>
            ))}
          </select>
        </label>
      )}
      {selected && (
        <section aria-label="Artifact preview">
          <h4>{selected.title}</h4>
          <p className="intent-muted">
            Saved {new Date(selected.createdAt).toLocaleString()} · intent {selected.intentRevision}{' '}
            · {selected.bytes.toLocaleString()} bytes
          </p>
          {selected.versionOf && (
            <p>
              Previous version:{' '}
              <button type="button" onClick={() => choose(selected.versionOf!)}>
                {records.artifacts.find((a) => a.id === selected.versionOf)?.title ||
                  selected.versionOf}
              </button>
            </p>
          )}
          <details className="intent-record-details">
            <summary>{selected.kind === 'url' ? 'Reference identity' : 'Content identity'}</summary>
            <p className="intent-muted">
              {selected.kind === 'url' ? 'SHA-256 of the URL' : 'SHA-256 of the saved bytes'}:{' '}
              <code style={{ overflowWrap: 'anywhere' }}>{selected.sha256}</code>
            </p>
          </details>
          <div className="intent-actions">
            <button type="button" aria-pressed={mode === 'view'} onClick={() => setMode('view')}>
              View
            </button>
            <button
              type="button"
              aria-pressed={mode === 'annotate'}
              disabled={locked || selected.intentRevision !== workspace.revision}
              onClick={() => setMode('annotate')}
            >
              Annotate
            </button>
          </div>
          {selected.intentRevision !== workspace.revision && (
            <p className="intent-inline-notice">
              This version belongs to an earlier intent. Save a new artifact version to annotate the
              current intent.
            </p>
          )}
          {previewError && <p role="alert">{previewError}</p>}
          {selected.kind === 'url' ? (
            <>
              <p>{selected.url}</p>
              <button
                type="button"
                disabled={!onOpenUrl}
                onClick={() => onOpenUrl?.(selected.url!)}
              >
                Open reference
              </button>
              <p className="intent-muted">URL contents were not fetched or frozen.</p>
            </>
          ) : preview?.id === selected.id ? (
            <>
              {selected.kind === 'image' && (
                <div
                  style={{ position: 'relative', display: 'inline-block', maxWidth: '100%' }}
                  className="intent-annotation-canvas"
                  role={mode === 'annotate' ? 'group' : undefined}
                  aria-label={mode === 'annotate' ? 'Image annotation position' : undefined}
                  aria-describedby={mode === 'annotate' ? annotationHelp : undefined}
                  tabIndex={mode === 'annotate' && !locked ? 0 : undefined}
                  onKeyDown={(event) => {
                    if (mode !== 'annotate' || locked) return;
                    const delta: { [key: string]: [number, number] } = {
                      ArrowLeft: [-0.01, 0],
                      ArrowRight: [0.01, 0],
                      ArrowUp: [0, -0.01],
                      ArrowDown: [0, 0.01],
                    };
                    if (event.key === 'Enter' || event.key === ' ') {
                      event.preventDefault();
                      setPoint(point ?? { x: 0.5, y: 0.5 });
                    } else if (delta[event.key]) {
                      event.preventDefault();
                      const [x, y] = delta[event.key];
                      setPoint({
                        x: Math.max(0, Math.min(1, (point?.x ?? 0.5) + x)),
                        y: Math.max(0, Math.min(1, (point?.y ?? 0.5) + y)),
                      });
                    } else if (event.key === 'Escape') {
                      event.preventDefault();
                      setPoint(undefined);
                    }
                  }}
                >
                  <img
                    src={`data:${selected.mimeType};base64,${preview.data}`}
                    alt={selected.title}
                    onError={() =>
                      setPreviewError(
                        'The saved image could not be displayed. Select another version or try refreshing.',
                      )
                    }
                    style={{
                      display: 'block',
                      maxWidth: '100%',
                      cursor: mode === 'annotate' ? 'crosshair' : 'default',
                    }}
                    onClick={(event) => {
                      if (mode !== 'annotate' || locked) return;
                      const rect = event.currentTarget.getBoundingClientRect();
                      if (rect.width && rect.height)
                        setPoint({
                          x: Math.max(0, Math.min(1, (event.clientX - rect.left) / rect.width)),
                          y: Math.max(0, Math.min(1, (event.clientY - rect.top) / rect.height)),
                        });
                    }}
                  />
                  {records.annotations
                    .filter(
                      (a) =>
                        a.artifactId === selected.id &&
                        a.artifactSha256 === selected.sha256 &&
                        a.point,
                    )
                    .map((annotation, index) => (
                      <span
                        key={annotation.id}
                        title={annotation.text}
                        style={{
                          position: 'absolute',
                          left: `${annotation.point!.x * 100}%`,
                          top: `${annotation.point!.y * 100}%`,
                          transform: 'translate(-50%, -50%)',
                          borderRadius: '50%',
                          background: 'var(--wks-accent)',
                          color: 'var(--wks-bg-base)',
                          padding: '4px',
                          pointerEvents: 'none',
                          fontSize: '0.66rem',
                        }}
                      >
                        {index + 1}
                      </span>
                    ))}
                  {mode === 'annotate' && point && (
                    <span
                      aria-label="Selected annotation point"
                      style={{
                        position: 'absolute',
                        left: `${point.x * 100}%`,
                        top: `${point.y * 100}%`,
                        width: 12,
                        height: 12,
                        border: '2px solid var(--wks-warning)',
                        borderRadius: '50%',
                        transform: 'translate(-50%, -50%)',
                        pointerEvents: 'none',
                      }}
                    />
                  )}
                </div>
              )}
              {selected.kind === 'text' && (
                <pre style={{ whiteSpace: 'pre-wrap', overflowWrap: 'anywhere' }}>
                  {decode(preview.data)}
                </pre>
              )}
              {selected.kind === 'html' && (
                <>
                  <p className="intent-muted">
                    Static HTML preview. Scripts, navigation, forms, and network resources are
                    disabled.
                  </p>
                  <iframe
                    title={`Static preview of ${selected.title}`}
                    sandbox=""
                    referrerPolicy="no-referrer"
                    srcDoc={staticArtifactHtml(decode(preview.data))}
                    style={{
                      width: '100%',
                      height: 400,
                      border: '1px solid var(--wks-border)',
                      borderRadius: 'var(--wks-radius-sm)',
                    }}
                  />
                </>
              )}
            </>
          ) : (
            !previewError && <p className="intent-muted">Loading saved bytes…</p>
          )}
          {mode === 'annotate' && (
            <form
              onSubmit={async (event) => {
                event.preventDefault();
                if (
                  await mutate({
                    action: 'annotateArtifact',
                    ...write,
                    annotationId: crypto.randomUUID(),
                    artifactId: selected.id,
                    artifactSha256: selected.sha256,
                    text: note,
                    ...(point ? { point } : {}),
                  })
                ) {
                  setNote('');
                  setPoint(undefined);
                }
              }}
            >
              <fieldset disabled={locked}>
                {selected.kind === 'image' && (
                  <p id={annotationHelp} className="intent-muted">
                    Click the image to pin a location, or focus it and press Enter, then use arrow
                    keys to move the point. Escape clears it. You can also leave a note without a
                    point.
                  </p>
                )}
                {point && (
                  <p role="status" className="intent-muted">
                    Annotation point: {Math.round(point.x * 100)}% across,{' '}
                    {Math.round(point.y * 100)}% down.
                  </p>
                )}
                {point && (
                  <button type="button" onClick={() => setPoint(undefined)}>
                    Clear annotation point
                  </button>
                )}
                <label>
                  Annotation note
                  <textarea
                    required
                    rows={3}
                    maxLength={8000}
                    value={note}
                    onChange={(e) => setNote(e.target.value)}
                  />
                </label>
                <button
                  type="submit"
                  disabled={!note.trim() || selected.intentRevision !== workspace.revision}
                >
                  Save annotation
                </button>
              </fieldset>
            </form>
          )}
          {records.annotations
            .filter((a) => a.artifactId === selected.id && a.artifactSha256 === selected.sha256)
            .map((a) => (
              <p key={a.id}>
                <strong>
                  User annotation
                  {a.point
                    ? ` at ${Math.round(a.point.x * 100)}%, ${Math.round(a.point.y * 100)}%`
                    : ''}
                  :
                </strong>{' '}
                {a.text}
              </p>
            ))}
        </section>
      )}
      <details>
        <summary>Create a demonstration</summary>
        <p className="intent-muted">An ordered sequence of saved screenshots with captions.</p>
        <form
          onSubmit={async (event) => {
            event.preventDefault();
            if (
              await mutate({
                action: 'createDemonstration',
                ...write,
                demonstrationId: crypto.randomUUID(),
                title: demoTitle,
                steps,
              })
            ) {
              setDemoTitle('');
              setSteps([]);
            }
          }}
        >
          <fieldset disabled={locked}>
            <label>
              Demonstration title
              <input
                required
                maxLength={240}
                value={demoTitle}
                onChange={(e) => setDemoTitle(e.target.value)}
              />
            </label>
            <button
              type="button"
              disabled={
                !selected ||
                selected.kind !== 'image' ||
                selected.intentRevision !== workspace.revision ||
                steps.length >= 24
              }
              onClick={() =>
                selected && setSteps([...steps, { artifactId: selected.id, caption: '' }])
              }
            >
              Add selected screenshot as next step
            </button>
            <ol>
              {steps.map((step, index) => (
                <li key={index}>
                  <label>
                    Step {index + 1}:{' '}
                    {records.artifacts.find((a) => a.id === step.artifactId)?.title}
                    <input
                      required
                      aria-label={`Step ${index + 1} caption`}
                      value={step.caption}
                      maxLength={2000}
                      onChange={(e) =>
                        setSteps(
                          steps.map((s, i) =>
                            i === index ? { ...s, caption: e.target.value } : s,
                          ),
                        )
                      }
                    />
                  </label>
                  <button
                    type="button"
                    disabled={!index}
                    onClick={() => {
                      const next = [...steps];
                      [next[index - 1], next[index]] = [next[index], next[index - 1]];
                      setSteps(next);
                    }}
                  >
                    Move step {index + 1} up
                  </button>
                  <button
                    type="button"
                    onClick={() => setSteps(steps.filter((_, i) => i !== index))}
                  >
                    Remove step {index + 1}
                  </button>
                </li>
              ))}
            </ol>
            {staleSteps && (
              <p className="intent-inline-notice">
                Some steps use screenshots from an earlier intent. Save current versions and replace
                those steps before saving this demonstration.
              </p>
            )}
            <button
              type="submit"
              disabled={
                !demoTitle.trim() ||
                !steps.length ||
                steps.some((s) => !s.caption.trim()) ||
                staleSteps
              }
            >
              Save demonstration
            </button>
          </fieldset>
        </form>
      </details>
      {records.demonstrations.map((demo) => (
        <details key={demo.id}>
          <summary>
            {demo.title} · demonstration · intent {demo.intentRevision}
          </summary>
          <ol>
            {demo.steps.map((step, i) => (
              <li key={`${step.artifactId}:${i}`}>
                <button type="button" onClick={() => choose(step.artifactId)}>
                  View step {i + 1}
                </button>{' '}
                {step.caption}
              </li>
            ))}
          </ol>
        </details>
      ))}
      <details>
        <summary>Compare alternatives</summary>
        <p className="intent-muted">
          Record hypotheses and a time budget. Choosing an alternative records your decision; it
          does not start agents or merge code.
        </p>
        <form
          onSubmit={async (event) => {
            event.preventDefault();
            if (
              await mutate({
                action: 'createAlternativeGroup',
                ...write,
                groupId: crypto.randomUUID(),
                title: groupTitle,
                purpose,
                budgetMinutes: budget,
                alternatives,
              })
            ) {
              setGroupTitle('');
              setPurpose('');
              setAlternatives([alternative(), alternative()]);
            }
          }}
        >
          <fieldset disabled={locked}>
            <label>
              Comparison title
              <input
                required
                maxLength={240}
                value={groupTitle}
                onChange={(e) => setGroupTitle(e.target.value)}
              />
            </label>
            <label>
              Purpose
              <textarea
                required
                maxLength={4000}
                value={purpose}
                onChange={(e) => setPurpose(e.target.value)}
              />
            </label>
            <label>
              Exploration budget (minutes)
              <input
                type="number"
                min={1}
                max={1440}
                required
                value={budget}
                onChange={(e) => setBudget(Number(e.target.value))}
              />
            </label>
            {alternatives.map((a, index) => (
              <section key={a.id}>
                <h4>Alternative {index + 1}</h4>
                <label>
                  Alternative {index + 1} name
                  <input
                    required
                    maxLength={240}
                    value={a.title}
                    onChange={(e) => patchAlternative(index, { title: e.target.value })}
                  />
                </label>
                <label>
                  Alternative {index + 1} hypothesis
                  <textarea
                    required
                    maxLength={4000}
                    value={a.hypothesis}
                    onChange={(e) => patchAlternative(index, { hypothesis: e.target.value })}
                  />
                </label>
                <label>
                  Alternative {index + 1} artifacts
                  <select
                    multiple
                    value={a.artifactIds}
                    onChange={(e) =>
                      patchAlternative(index, {
                        artifactIds: Array.from(e.target.selectedOptions, (o) => o.value),
                      })
                    }
                  >
                    {current.map((artifact) => (
                      <option key={artifact.id} value={artifact.id}>
                        {artifact.title}
                      </option>
                    ))}
                  </select>
                </label>
                <label>
                  Alternative {index + 1} executions
                  <select
                    multiple
                    value={a.executionIds}
                    onChange={(e) =>
                      patchAlternative(index, {
                        executionIds: Array.from(e.target.selectedOptions, (o) => o.value),
                      })
                    }
                  >
                    {executions.map((run) => (
                      <option key={run.id} value={run.id}>
                        {run.session?.label || run.task || run.id}
                      </option>
                    ))}
                  </select>
                </label>
                {alternatives.length > 2 && (
                  <button
                    type="button"
                    onClick={() => setAlternatives(alternatives.filter((_, i) => i !== index))}
                  >
                    Remove alternative {index + 1}
                  </button>
                )}
              </section>
            ))}
            <button
              type="button"
              disabled={alternatives.length >= 6}
              onClick={() => setAlternatives([...alternatives, alternative()])}
            >
              Add alternative
            </button>
            {staleAlternativeLinks && (
              <p className="intent-inline-notice">
                Some artifact links belong to an earlier intent.{' '}
                <button
                  type="button"
                  onClick={() =>
                    setAlternatives((items) =>
                      items.map((item) => ({
                        ...item,
                        artifactIds: item.artifactIds.filter((id) =>
                          current.some((artifact) => artifact.id === id),
                        ),
                      })),
                    )
                  }
                >
                  Remove earlier artifact links
                </button>
              </p>
            )}
            <button
              type="submit"
              disabled={
                !groupTitle.trim() ||
                !purpose.trim() ||
                staleAlternativeLinks ||
                alternatives.some((a) => !a.title.trim() || !a.hypothesis.trim())
              }
            >
              Save comparison
            </button>
          </fieldset>
        </form>
      </details>
      {records.groups.map((group) => {
        const latest = records.selections.find((s) => s.groupId === group.id);
        const choice = choices[group.id] ?? {
          alternativeId: '',
          reason: '',
          expectedSelectionId: latest?.id,
        };
        const selectionChanged = !!choices[group.id] && choice.expectedSelectionId !== latest?.id;
        return (
          <details key={group.id}>
            <summary>
              {group.title} · alternatives · intent {group.intentRevision}
            </summary>
            <p>{group.purpose}</p>
            <p className="intent-muted">
              Declared budget: {group.budgetMinutes} minutes. No automatic work was started.
            </p>
            {group.alternatives.map((a) => (
              <section key={a.id}>
                <h4>
                  {a.title}
                  {latest?.alternativeId === a.id ? ' · selected' : ''}
                </h4>
                <p>{a.hypothesis}</p>
                {a.artifactIds.map((id) => (
                  <button key={id} type="button" onClick={() => choose(id)}>
                    View{' '}
                    {records.artifacts.find((artifact) => artifact.id === id)?.title || 'artifact'}
                  </button>
                ))}
                {a.executionIds.length > 0 && (
                  <p>
                    Executions:{' '}
                    {a.executionIds
                      .map((id) => executions.find((run) => run.id === id)?.session?.label || id)
                      .join(', ')}
                  </p>
                )}
              </section>
            ))}
            {group.intentRevision === workspace.revision && (
              <form
                onSubmit={async (event) => {
                  event.preventDefault();
                  if (locked || selectionChanged) return;
                  if (
                    await mutate({
                      action: 'selectAlternative',
                      ...write,
                      selectionId: crypto.randomUUID(),
                      groupId: group.id,
                      alternativeId: choice.alternativeId,
                      reason: choice.reason,
                      ...(choice.expectedSelectionId
                        ? { expectedSelectionId: choice.expectedSelectionId }
                        : {}),
                    })
                  )
                    setChoices((previous) => {
                      const next = { ...previous };
                      delete next[group.id];
                      return next;
                    });
                }}
              >
                <fieldset disabled={locked}>
                  <label>
                    Choose alternative
                    <select
                      required
                      value={choice.alternativeId}
                      onChange={(e) =>
                        setChoices({
                          ...choices,
                          [group.id]: { ...choice, alternativeId: e.target.value },
                        })
                      }
                    >
                      <option value="">Select one</option>
                      {group.alternatives.map((a) => (
                        <option key={a.id} value={a.id}>
                          {a.title}
                        </option>
                      ))}
                    </select>
                  </label>
                  <label>
                    Selection reason
                    <textarea
                      required
                      value={choice.reason}
                      maxLength={8000}
                      onChange={(e) =>
                        setChoices({
                          ...choices,
                          [group.id]: { ...choice, reason: e.target.value },
                        })
                      }
                    />
                  </label>
                  {selectionChanged && (
                    <p className="intent-inline-notice">
                      The selection changed while you were deciding. Review the current choice
                      above.{' '}
                      <button
                        type="button"
                        onClick={() =>
                          setChoices((previous) => ({
                            ...previous,
                            [group.id]: { ...choice, expectedSelectionId: latest?.id },
                          }))
                        }
                      >
                        I reviewed the current selection
                      </button>
                    </p>
                  )}
                  <button
                    type="submit"
                    disabled={!choice.alternativeId || !choice.reason.trim() || selectionChanged}
                  >
                    Record selection
                  </button>
                </fieldset>
              </form>
            )}
            {records.selections
              .filter((s) => s.groupId === group.id)
              .map((selection) => (
                <p key={selection.id}>
                  <strong>
                    {group.alternatives.find((a) => a.id === selection.alternativeId)?.title}
                  </strong>{' '}
                  — {selection.reason}{' '}
                  <span className="intent-muted">
                    {new Date(selection.createdAt).toLocaleString()}
                  </span>
                </p>
              ))}
          </details>
        );
      })}
    </Surface>
  );
}
