/**
 * The hub-owned "too many sessions" prompt (docs/session-roster.md §6).
 *
 * Nothing leaves the shared sidebar on its own, so the hub tells every client
 * when it needs a cleanup (`sessionOpen.cleanup.suggested`) and each renders the
 * same prompt: Hide all ended / Hide ended older than a day / Not now. A
 * cleanup hides (archives) ended sessions — never Paused or live ones — and
 * offers an Undo for a few seconds that restores exactly the ids it hid.
 * Hidden sessions stay in History.
 *
 * Shown only when the hub serves the cleanup methods: an older hub gets no
 * banner. Styling follows DESIGN_LANGUAGE.md: a fill tint, no border; bare
 * tokens; scale font sizes; lucide icons.
 */
import { useEffect, useRef, useState } from 'react';
import { RotateCcw, X } from 'lucide-react';
import { SmallButton } from './settings/primitives';
import type { SessionOpenDoc } from '../types/electron';

export const UNDO_MS = 8000;
export const DAY_MS = 24 * 60 * 60 * 1000;

interface Props {
  doc: SessionOpenDoc | null;
  /** Adopt a document a call returned (snooze), version-fenced. */
  onDocument?: (doc: unknown) => void;
}

type Undo = { hidden: string[]; timer: ReturnType<typeof setTimeout> };

export function SessionCleanupBanner({ doc, onDocument }: Props) {
  const api = window.electronAPI;
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [undo, setUndo] = useState<Undo | null>(null);
  const undoRef = useRef<Undo | null>(null);
  undoRef.current = undo;
  useEffect(() => () => clearTimeout(undoRef.current?.timer), []);

  const canClean = !!api.sessionOpenCleanup && !!api.sessionOpenSnoozeCleanup;
  const cleanup = doc?.cleanup;
  const showPrompt = canClean && !!cleanup?.suggested && !undo;

  const fail = (e: unknown) => setError(e instanceof Error ? e.message : String(e));

  const hide = async (olderThanMs?: number) => {
    setBusy(true);
    setError(null);
    try {
      const { hidden } = await api.sessionOpenCleanup!(
        olderThanMs === undefined ? {} : { olderThanMs },
      );
      const ids = Array.isArray(hidden) ? hidden : [];
      if (ids.length) {
        clearTimeout(undoRef.current?.timer);
        const timer = setTimeout(() => setUndo(null), UNDO_MS);
        setUndo({ hidden: ids, timer });
      }
    } catch (e) {
      fail(e);
    } finally {
      setBusy(false);
    }
  };

  const snooze = async () => {
    setBusy(true);
    setError(null);
    try {
      onDocument?.(await api.sessionOpenSnoozeCleanup!());
    } catch (e) {
      fail(e);
    } finally {
      setBusy(false);
    }
  };

  const restore = async () => {
    const current = undoRef.current;
    if (!current) return;
    clearTimeout(current.timer);
    setUndo(null);
    if (!api.sessionArchiveSetMany) return;
    try {
      await api.sessionArchiveSetMany(current.hidden, false);
    } catch (e) {
      fail(e);
    }
  };

  if (!showPrompt && !undo && !error) return null;

  const n = cleanup?.ended ?? 0;
  return (
    <div
      role={undo ? 'status' : 'region'}
      aria-label={undo ? 'Sessions hidden' : 'Sidebar cleanup'}
      data-testid="session-cleanup-banner"
      style={{
        margin: '0 12px',
        padding: '8px 10px',
        display: 'flex',
        flexDirection: 'column',
        gap: 6,
        borderRadius: 'var(--wks-radius-md)',
        background: 'color-mix(in srgb, var(--wks-accent) 10%, transparent)',
        fontSize: '0.72rem',
        color: 'var(--wks-text-secondary)',
      }}
    >
      {undo ? (
        <div style={{ display: 'flex', alignItems: 'center', gap: 8 }}>
          <span style={{ flex: 1 }}>
            Hid {undo.hidden.length} {undo.hidden.length === 1 ? 'session' : 'sessions'}. Still in
            History.
          </span>
          {api.sessionArchiveSetMany && (
            <SmallButton
              label={
                <span style={{ display: 'inline-flex', alignItems: 'center', gap: 4 }}>
                  <RotateCcw size={12} strokeWidth={2} />
                  Undo
                </span>
              }
              onClick={() => void restore()}
            />
          )}
        </div>
      ) : showPrompt ? (
        <>
          <div style={{ display: 'flex', alignItems: 'center', gap: 6 }}>
            <span style={{ flex: 1, fontWeight: 500, color: 'var(--wks-text-primary)' }}>
              {n} ended {n === 1 ? 'session' : 'sessions'} on your sidebar
            </span>
            <button
              type="button"
              aria-label="Not now"
              title="Not now"
              disabled={busy}
              onClick={() => void snooze()}
              style={{
                display: 'flex',
                alignItems: 'center',
                padding: 2,
                border: 'none',
                background: 'transparent',
                color: 'var(--wks-text-muted)',
                cursor: busy ? 'default' : 'pointer',
              }}
            >
              <X size={12} strokeWidth={2} />
            </button>
          </div>
          <div style={{ display: 'flex', flexWrap: 'wrap', gap: 4 }}>
            <SmallButton label="Hide all ended" primary disabled={busy} onClick={() => void hide()} />
            <SmallButton
              label="Hide ended older than a day"
              disabled={busy}
              onClick={() => void hide(DAY_MS)}
            />
            <SmallButton label="Not now" disabled={busy} onClick={() => void snooze()} />
          </div>
        </>
      ) : null}
      {error && (
        <div role="alert" style={{ color: 'var(--wks-error)', fontSize: '0.66rem' }}>
          {error}
        </div>
      )}
    </div>
  );
}

export default SessionCleanupBanner;
