import React from 'react';
import type { AgentProvider } from '../types/pane';
import { fmtTokens } from '../lib/sessionStats';

export interface ContextChoice {
  value: number | null;
  label: string;
}

interface Props {
  provider?: AgentProvider;
  requested?: number | null;
  effective?: number;
  providerDefault?: number;
  advertisedMaximum?: number;
  choices?: ContextChoice[];
  onChange?: (value: number | null) => void;
  allowNumeric?: boolean;
}

const tokens = (value: number) => `${fmtTokens(value)} tokens`;

/** Shared context disclosure for every harness. Configuration and runtime
 * truth deliberately occupy different labelled rows; only Claude/Codex pass
 * choices, while other installed harnesses remain provider-managed. */
export const ModelContextPopover: React.FC<Props> = ({
  provider = 'claude',
  requested,
  effective,
  providerDefault,
  advertisedMaximum,
  choices,
  onChange,
  allowNumeric,
}) => {
  const editable = !!onChange && !!choices?.length;
  const summary = effective
    ? `Context · ${fmtTokens(effective)} effective`
    : requested
      ? `Context · ${fmtTokens(requested)} requested`
      : 'Context · provider-managed';
  return (
    <details
      className="wks-context-popover"
      style={{ position: 'relative', display: 'inline-block' }}
    >
      <summary
        style={{ cursor: 'pointer', listStyle: 'none', fontSize: 12 }}
        aria-label="Context settings"
      >
        {summary}
      </summary>
      <div
        role="dialog"
        aria-label="Context"
        style={{
          position: 'absolute',
          zIndex: 30,
          top: 'calc(100% + 6px)',
          right: 0,
          width: 260,
          padding: 12,
          border: '1px solid var(--wks-border-subtle)',
          borderRadius: 8,
          background: 'var(--wks-bg-elevated)',
          boxShadow: '0 8px 28px var(--wks-shadow)',
          fontSize: 12,
        }}
      >
        <div style={{ fontWeight: 600, marginBottom: 8 }}>Context · {provider}</div>
        {effective !== undefined && <div>Effective (runtime confirmed): {tokens(effective)}</div>}
        {requested != null && (
          <div>
            Requested{effective === undefined ? ' (provisional)' : ''}: {tokens(requested)}
          </div>
        )}
        {providerDefault !== undefined && <div>Provider default: {tokens(providerDefault)}</div>}
        {advertisedMaximum !== undefined && (
          <div>Advertised maximum: {tokens(advertisedMaximum)}</div>
        )}
        {!editable && requested == null && (
          <div>Provider-managed; this installed harness exposes no validated request control.</div>
        )}
        {editable && (
          <div style={{ display: 'grid', gap: 6, marginTop: 10 }}>
            {choices!.map((choice) => (
              <button
                key={choice.value ?? 'managed'}
                type="button"
                aria-pressed={(requested ?? null) === choice.value}
                onClick={() => onChange!(choice.value)}
              >
                {choice.label}
              </button>
            ))}
            {allowNumeric && (
              <label>
                Custom tokens
                <input
                  aria-label="Custom context tokens"
                  type="number"
                  min={1}
                  step={1000}
                  value={requested ?? ''}
                  onChange={(event) => {
                    const value = Number(event.target.value);
                    if (Number.isSafeInteger(value) && value > 0) onChange!(value);
                  }}
                  style={{ width: '100%', marginTop: 4 }}
                />
              </label>
            )}
          </div>
        )}
      </div>
    </details>
  );
};

export default ModelContextPopover;
