/**
 * A reasoning-effort picker that follows the HARNESS it is picking for — the
 * effort twin of HarnessModelSelect, and it exists for the same reason.
 *
 * The effort ladders are per-CLI and do not overlap: Claude has low..max, Codex
 * minimal..xhigh (plus whatever its live catalog reports), Copilot its own
 * seven, and OpenCode/Pi have no such knob at all. So a single shared list would
 * offer levels the selected CLI has never heard of, and a fixed row would show
 * a control that does nothing for the harnesses without the knob.
 *
 *  - Levels come from `capsFor(provider).effort`, the same source the spawn
 *    dialog and the composer pill read.
 *  - A harness with `effort: null` renders NOTHING — no dead row, no
 *    always-disabled select implying the setting exists.
 *  - A saved level the harness doesn't list stays visible and flagged, never
 *    silently blanked (same rule as the model picker).
 */
import React from 'react';
import type { AgentProvider } from '../../types/pane';
import { capsFor, effortLevelLabel } from '../../lib/providerCaps';
import { Row, SearchableSelect, SelectOption } from './primitives';

interface HarnessEffortSelectProps {
  /** The harness this row picks an effort level for. */
  provider: AgentProvider;
  label: string;
  value: string;
  onChange: (value: string) => void;
  /** Explanatory copy under the row. */
  hint?: React.ReactNode;
}

/** The option list and stale-value verdict for one harness. Pure, and exported
 *  because that is the part worth pinning in a test. */
export function harnessEffortOptions(
  provider: AgentProvider,
  value: string,
): { options: SelectOption[]; unknown: boolean } | null {
  const levels = capsFor(provider).effort?.levels;
  if (!levels?.length) return null;
  const known = levels.some((l) => l.id === value);
  return {
    options: [
      { value: '', label: `${provider} default` },
      ...levels.map((l) => ({ value: l.id, label: effortLevelLabel(l.id) })),
      ...(value && !known ? [{ value, label: `${value} (not a ${provider} level)` }] : []),
    ],
    unknown: !!value && !known,
  };
}

const HarnessEffortSelect: React.FC<HarnessEffortSelectProps> = ({
  provider,
  label,
  value,
  onChange,
  hint,
}) => {
  const resolved = harnessEffortOptions(provider, value);
  if (!resolved) return null;
  return (
    <>
      <Row label={label}>
        <SearchableSelect
          value={value}
          options={resolved.options}
          onChange={onChange}
          placeholder={`${provider} default`}
        />
      </Row>
      {hint && <div style={{ fontSize: '0.72rem', color: 'var(--wks-text-disabled)' }}>{hint}</div>}
      {resolved.unknown && (
        <div style={{ fontSize: '0.72rem', color: 'var(--wks-warning)' }}>
          <strong>{value}</strong> is not one of {provider}’s effort levels, so a {provider} spawn
          may be refused it. Pick one above, or leave it on the {provider} default.
        </div>
      )}
    </>
  );
};

export default HarnessEffortSelect;
