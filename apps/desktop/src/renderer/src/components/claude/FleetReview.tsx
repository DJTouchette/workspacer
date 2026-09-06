import React, { useContext, useState } from 'react';
import { ChevronDown, ChevronRight } from 'lucide-react';
import type { FleetReviewEvidence, FleetReviewFile } from '../../../../main/shared/fleetReview';
import type { HtmlCardHost } from '../../lib/htmlCard/cardActions';
import { HtmlCardHostContext } from './HtmlResponseCard';
import DiffViewer from '../review/DiffViewer';
import { parseUnifiedDiff } from '../../lib/diff/parseDiff';
import { claudeColors as colors } from '../claude-shared';

const buttonStyle: React.CSSProperties = {
  background: 'transparent',
  color: colors.accent,
  border: `1px solid ${colors.borderSubtle}`,
  borderRadius: 'var(--wks-radius-sm)',
  padding: '4px 7px',
  cursor: 'pointer',
  fontSize: '0.7rem',
  display: 'inline-flex',
  gap: 4,
  alignItems: 'center',
};
/** The owner comes from the containing chat pane, never from transcript JSON. */
export function FleetReview({
  evidenceId,
  workerSessionId,
}: {
  evidenceId?: string;
  workerSessionId: string;
}): React.ReactElement {
  const host = useContext(HtmlCardHostContext);
  return (
    <FleetReviewBody
      key={`${host?.sessionId}:${workerSessionId}:${evidenceId}`}
      host={host}
      evidenceId={evidenceId}
      workerSessionId={workerSessionId}
    />
  );
}
function FleetReviewBody({
  host,
  evidenceId,
  workerSessionId,
}: {
  host: HtmlCardHost | null;
  evidenceId?: string;
  workerSessionId: string;
}): React.ReactElement {
  const [open, setOpen] = useState(false);
  const [evidence, setEvidence] = useState<FleetReviewEvidence>();
  const [selected, setSelected] = useState<FleetReviewFile>();
  const [error, setError] = useState('');
  const [busy, setBusy] = useState(false);
  const [forgotten, setForgotten] = useState(false);
  const unsupported = !window.electronAPI?.fleetReviewRead;
  const unavailable = forgotten
    ? 'Review data forgotten.'
    : unsupported
      ? 'Review evidence is available only on the originating local desktop. Remote/headless review is not supported.'
      : !host?.sessionId
        ? 'The owning manager session is unavailable.'
        : !evidenceId
          ? 'No host-captured range. In-place, remote, or older dispatches cannot be reviewed here.'
          : '';
  const request = {
    ownerSessionId: host?.sessionId ?? '',
    workerSessionId,
    evidenceId: evidenceId ?? '',
  };
  async function load(file?: string): Promise<void> {
    setBusy(true);
    setError('');
    try {
      const result = await window.electronAPI.fleetReviewRead!(
        file === undefined ? request : { ...request, file },
      );
      if (host?.isCurrent?.() === false) return;
      if (!result.ok) {
        setError(result.error);
        setSelected(undefined);
        return;
      }
      if (file === undefined) {
        setEvidence(result.evidence);
        if (result.evidence.availability === 'captured' && result.evidence.files[0])
          await load(result.evidence.files[0].path);
      } else setSelected(result.evidence.files[0]);
    } catch {
      setError('Review evidence is unavailable on this backend.');
    } finally {
      setBusy(false);
    }
  }
  async function forget(): Promise<void> {
    setBusy(true);
    try {
      const result = await window.electronAPI.fleetReviewForget?.(request);
      if (!result?.ok) {
        setError('Could not forget review data.');
        return;
      }
      setEvidence(undefined);
      setSelected(undefined);
      setForgotten(true);
      setOpen(false);
    } catch {
      setError('Could not forget review data.');
    } finally {
      setBusy(false);
    }
  }
  const diff = selected?.diff === undefined ? undefined : parseUnifiedDiff(selected.diff);
  return (
    <section aria-label="Worker review" style={{ marginTop: 8, minWidth: 0, color: colors.text }}>
      <button
        style={buttonStyle}
        disabled={!!unavailable || busy}
        aria-expanded={open}
        onClick={() => {
          setOpen(!open);
          if (!open) {
            setSelected(undefined);
            void load();
          }
        }}
      >
        {open ? <ChevronDown size={12} /> : <ChevronRight size={12} />}Review changes
      </button>
      {unavailable && <p style={{ fontSize: '0.7rem', color: colors.muted }}>{unavailable}</p>}
      {open && (
        <div
          data-testid="fleet-inline-review"
          style={{ marginTop: 8, minWidth: 0, overflow: 'hidden' }}
        >
          {busy && <p role="status">Loading captured review…</p>}
          {error && <p role="alert">{error}</p>}
          {evidence && (
            <>
              <div style={{ fontSize: '0.7rem', overflowWrap: 'anywhere', lineHeight: 1.6 }}>
                <strong>{evidence.projectRoot.split(/[/\\]/).pop()}</strong> · {evidence.branch}
                <div>
                  {evidence.baseCommit} → {evidence.headCommit ?? 'unresolved'}
                </div>
                <div>
                  Source worker: {evidence.workerSessionId} · local · {evidence.availability}
                </div>
                <div>
                  Captured {evidence.capturedAt} ·{' '}
                  {evidence.lifecycle === 'turn-ended'
                    ? 'turn ended; writer may still be running'
                    : evidence.lifecycle === 'session-ended'
                      ? 'session ended; descendant writers not verified'
                      : 'before worktree removal; writer termination not verified'}
                </div>
                <div title={evidence.projectRoot}>{evidence.allocatedCwd}</div>
              </div>
              {evidence.reason && <p style={{ fontSize: '0.7rem' }}>{evidence.reason}</p>}
              {evidence.availability === 'captured' && (
                <>
                  <p style={{ fontSize: '0.7rem', color: colors.muted }}>
                    Immutable committed range · read only · checks are worker-reported, without host
                    verification.
                  </p>
                  {evidence.files.length === 0 && <p>No committed changes in this range.</p>}
                  <div
                    aria-label="Changed files"
                    style={{
                      maxHeight: 200,
                      overflow: 'auto',
                      display: 'flex',
                      flexDirection: 'column',
                      gap: 4,
                    }}
                  >
                    {evidence.files.map((file) => (
                      <button
                        key={file.path}
                        style={{ ...buttonStyle, textAlign: 'left', overflowWrap: 'anywhere' }}
                        disabled={busy}
                        aria-pressed={selected?.path === file.path}
                        onClick={() => void load(file.path)}
                      >
                        {file.status} · {file.oldPath ? `${file.oldPath} → ` : ''}
                        {file.path}
                      </button>
                    ))}
                  </div>
                  {selected && diff && (
                    <div style={{ marginTop: 8 }}>
                      <div style={{ fontSize: '0.7rem', overflowWrap: 'anywhere' }}>
                        {selected.path} · +{diff.additions} / −{diff.deletions}
                      </div>
                      {diff.binary ? (
                        <p>Binary file changed; contents are not retained.</p>
                      ) : diff.hunks.length === 0 ? (
                        <pre style={{ whiteSpace: 'pre-wrap', fontSize: '0.7rem' }}>
                          {selected.diff}
                        </pre>
                      ) : (
                        <div style={{ height: 320 }}>
                          <DiffViewer diff={diff} path={selected.path} />
                        </div>
                      )}
                    </div>
                  )}
                </>
              )}
              <button
                style={{ ...buttonStyle, marginTop: 8 }}
                disabled={busy}
                onClick={() => void forget()}
              >
                Forget review data
              </button>
            </>
          )}
        </div>
      )}
    </section>
  );
}
