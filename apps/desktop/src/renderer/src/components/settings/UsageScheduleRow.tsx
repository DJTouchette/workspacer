import React, { useCallback, useEffect, useRef, useState } from 'react';
import { Row, ModeButton } from './primitives';
import { refreshUsageReport } from '../../hooks/useUsageReport';
import type {
  UsagePacingSchedule,
  UsagePacingScheduleWire,
} from '../../../../main/shared/usageReport';

/**
 * Settings → "Usage schedule": which week the Overview's SEVEN-DAY usage pacing
 * tick is drawn against.
 *
 * The value is HUB-OWNED, not a config.yaml key, and that is deliberate — see
 * `services/hub/internal/usageprefs`. Three consequences the UI has to honour:
 *
 *  1. An older hub does not know the methods, so the read answers `null`. The
 *     control renders dimmed and inert with a reason, rather than vanishing
 *     (which reads as "removed") or showing a default nobody chose (which reads
 *     as a setting that silently reverted).
 *  2. A save can genuinely FAIL — refused, or the hub went away mid-click. The
 *     selection only moves once the hub has confirmed what it stored; a failure
 *     shows the error and leaves the previous choice selected.
 *  3. The choice changes what `usage.report` computes, so the Overview is
 *     re-read immediately on success. Without that, a card can keep showing the
 *     old curve for up to a minute and the setting looks like it did nothing.
 *
 * Nothing here does date arithmetic. Every weekday/weekend/timezone/DST fact is
 * the hub's, and this file only ever names one of two words.
 */

const OPTIONS: { value: UsagePacingSchedule; label: string }[] = [
  { value: 'five_day', label: 'Work week (Mon–Fri)' },
  { value: 'seven_day', label: 'Every day (7 days)' },
];

const HINT: Record<UsagePacingSchedule | '', string> = {
  five_day:
    'Expected weekly usage climbs Monday to Friday and stays flat over the weekend, in the hub machine’s own timezone. Weekend work still counts against the week — only the expectation pauses.',
  seven_day:
    'Expected weekly usage rises evenly across all seven days, which is how the cards shipped.',
  '': 'No schedule chosen, so the hub’s routing.yaml decides the weekly curve. Pick one to override it.',
};

export default function UsageScheduleRow(): React.ReactElement {
  const [state, setState] = useState<UsagePacingScheduleWire | null>(null);
  const [loaded, setLoaded] = useState(false);
  const [saving, setSaving] = useState(false);
  const [error, setError] = useState<string | null>(null);
  // Replies from a click the user has already superseded must not repaint the
  // control; only the newest one may.
  const seq = useRef(0);

  useEffect(() => {
    let alive = true;
    const read = window.electronAPI?.usagePacingSchedule;
    if (typeof read !== 'function') {
      setLoaded(true);
      return;
    }
    void read()
      .then((s) => {
        if (alive) setState(s);
      })
      .catch(() => {
        if (alive) setState(null);
      })
      .finally(() => {
        if (alive) setLoaded(true);
      });
    return () => {
      alive = false;
    };
  }, []);

  const choose = useCallback(
    (value: UsagePacingSchedule) => {
      const write = window.electronAPI?.setUsagePacingSchedule;
      if (typeof write !== 'function' || state?.schedule === value) return;
      const mine = ++seq.current;
      setSaving(true);
      setError(null);
      void write(value)
        .then((res) => {
          if (mine !== seq.current) return;
          if (!res.ok) {
            setError(res.error || 'the hub refused the change');
            return;
          }
          // The hub's own answer, not the value we sent: the two agree today
          // and a control that assumed so would stop reporting the truth the
          // day they did not.
          setState(res.state);
          // The pacing tick is computed hub-side from this, so re-read now.
          void refreshUsageReport();
        })
        .catch((err: unknown) => {
          if (mine !== seq.current) return;
          setError(err instanceof Error ? err.message : String(err));
        })
        .finally(() => {
          if (mine === seq.current) setSaving(false);
        });
    },
    [state?.schedule],
  );

  const unavailable = loaded && (state === null || !state.configurable);
  const reason =
    state === null
      ? 'This server’s hub does not offer the usage-schedule setting (it predates it, or is not reachable). The weekly curve stays as its routing.yaml sets it.'
      : 'This hub was started without a preference file, so the schedule cannot be stored here.';

  return (
    <>
      <Row label="Usage schedule">
        <div style={{ display: 'flex', gap: 4 }}>
          {OPTIONS.map((o) => (
            <ModeButton
              key={o.value}
              label={o.label}
              active={state?.schedule === o.value}
              disabled={!loaded || unavailable || saving}
              title={unavailable ? reason : undefined}
              onClick={() => choose(o.value)}
            />
          ))}
        </div>
      </Row>
      <div style={{ fontSize: '0.72rem', color: 'var(--wks-text-disabled)' }}>
        {unavailable ? reason : HINT[state?.schedule ?? '']} Applies to the 7-day usage cards on
        Overview only — the 5-hour and monthly windows are unchanged either way.
      </div>
      {error && (
        <div style={{ fontSize: '0.72rem', color: 'var(--wks-error)' }} role="alert">
          Could not save the usage schedule: {error}
        </div>
      )}
    </>
  );
}
