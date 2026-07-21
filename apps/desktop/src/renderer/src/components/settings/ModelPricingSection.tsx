import React, { useCallback, useEffect, useMemo, useState } from 'react';
import { ChevronDown, ChevronRight } from 'lucide-react';
import { Section, SmallButton, inputStyle } from './primitives';

// Shapes mirror electron.d.ts pricingGetRates.
interface DefaultRate {
  input: number;
  output: number;
  contextLimit: number;
}
interface OverrideRate {
  input: number;
  output: number;
  cached_input?: number;
  context_limit?: number;
}
type Editable = { input: string; output: string; context: string };

/** Parse a trimmed numeric field; undefined when blank/invalid. */
function num(s: string): number | undefined {
  const t = s.trim();
  if (!t) return undefined;
  const n = Number(t);
  return Number.isFinite(n) && n >= 0 ? n : undefined;
}

/** "claude-opus-4-1-" → "Opus 4.1 (legacy)"-ish; keep it readable but exact. */
function prettyPrefix(prefix: string): string {
  return prefix.replace(/^claude-/, '').replace(/-$/, '');
}

/**
 * Model pricing overrides editor. Writes ~/.workspacer/model-rates.json — the
 * same file the claudemon Rust engine reads — so an edit re-rates both costing
 * paths with no restart. Collapsed by default (an advanced knob).
 */
const ModelPricingSection: React.FC = () => {
  const [defaults, setDefaults] = useState<Record<string, DefaultRate>>({});
  const [overrides, setOverrides] = useState<Record<string, OverrideRate>>({});
  const [rows, setRows] = useState<Record<string, Editable>>({});
  const [open, setOpen] = useState(false); // collapsed by default
  const [status, setStatus] = useState<'idle' | 'saving' | 'saved'>('idle');

  const load = useCallback(async () => {
    const res = await window.electronAPI.pricingGetRates?.();
    if (!res) return;
    setDefaults(res.defaults as Record<string, DefaultRate>);
    setOverrides(res.overrides as Record<string, OverrideRate>);
    const init: Record<string, Editable> = {};
    for (const [prefix, ov] of Object.entries(res.overrides as Record<string, OverrideRate>)) {
      init[prefix] = {
        input: String(ov.input),
        output: String(ov.output),
        context: ov.context_limit != null ? String(ov.context_limit) : '',
      };
    }
    setRows(init);
  }, []);

  useEffect(() => {
    void load();
  }, [load]);

  // Union of built-in models and any override-only prefixes (e.g. codex rates
  // added to the file directly), so nothing the user set is hidden.
  const prefixes = useMemo(() => {
    const set = new Set<string>([...Object.keys(defaults), ...Object.keys(overrides)]);
    return Array.from(set);
  }, [defaults, overrides]);

  const overrideCount = Object.keys(overrides).length;

  const setField = (prefix: string, field: keyof Editable, value: string) => {
    setRows((r) => ({ ...r, [prefix]: { ...(r[prefix] ?? blank()), [field]: value } }));
    setStatus('idle');
  };

  const save = useCallback(async () => {
    setStatus('saving');
    const next: Record<string, OverrideRate> = {};
    for (const prefix of prefixes) {
      const row = rows[prefix];
      if (!row) continue;
      const touched = row.input.trim() || row.output.trim() || row.context.trim();
      if (!touched) continue;
      const def = defaults[prefix];
      const input = num(row.input) ?? def?.input;
      const output = num(row.output) ?? def?.output;
      if (input == null || output == null) continue; // can't form a valid entry
      const contextLimit = num(row.context) ?? def?.contextLimit;
      const entry: OverrideRate = { input, output };
      if (contextLimit != null) entry.context_limit = contextLimit;
      // Preserve any advanced cached_input set via the file directly.
      const prevCached = overrides[prefix]?.cached_input;
      if (prevCached != null) entry.cached_input = prevCached;
      next[prefix] = entry;
    }
    await window.electronAPI.pricingSaveOverrides?.(next);
    await load();
    setStatus('saved');
    setTimeout(() => setStatus('idle'), 1500);
  }, [prefixes, rows, defaults, overrides, load]);

  const resetAll = useCallback(async () => {
    setStatus('saving');
    await window.electronAPI.pricingSaveOverrides?.({});
    setRows({});
    await load();
    setStatus('saved');
    setTimeout(() => setStatus('idle'), 1500);
  }, [load]);

  const cell: React.CSSProperties = { ...inputStyle, width: 92, textAlign: 'right' };
  const head: React.CSSProperties = {
    fontSize: '0.64rem',
    fontWeight: 700,
    textTransform: 'uppercase',
    letterSpacing: '0.04em',
    color: 'var(--wks-text-faint)',
  };

  return (
    <Section title="Model pricing">
      <div style={{ fontSize: '0.78rem', color: 'var(--wks-text-muted)', lineHeight: 1.6 }}>
        Override the per-model rates used for cost and context-window readouts. Values are USD per
        million tokens; context is in tokens. Blank fields use the built-in default (shown as the
        placeholder). Edits are written to{' '}
        <code style={{ fontFamily: 'var(--wks-font-mono)', fontSize: '0.72rem' }}>
          ~/.workspacer/model-rates.json
        </code>{' '}
        and apply to both costing engines immediately — no restart.
      </div>

      <button
        onClick={() => setOpen((o) => !o)}
        style={{
          alignSelf: 'flex-start',
          marginTop: 12,
          display: 'inline-flex',
          alignItems: 'center',
          gap: 8,
          padding: '6px 4px',
          background: 'transparent',
          border: 'none',
          cursor: 'pointer',
          color: 'var(--wks-text-secondary)',
          fontFamily: 'inherit',
          fontSize: '0.8rem',
          fontWeight: 600,
        }}
      >
        <span
          style={{ color: 'var(--wks-text-faint)', display: 'inline-flex', alignItems: 'center' }}
        >
          {open ? (
            <ChevronDown size={11} strokeWidth={2} />
          ) : (
            <ChevronRight size={11} strokeWidth={2} />
          )}
        </span>
        Edit rate table
        {overrideCount > 0 && (
          <span
            style={{
              fontSize: '0.66rem',
              fontWeight: 700,
              padding: '1px 7px',
              borderRadius: 'var(--wks-radius-pill)',
              background: 'var(--wks-accent-bg)',
              color: 'var(--wks-accent-text)',
            }}
          >
            {overrideCount} override{overrideCount === 1 ? '' : 's'}
          </span>
        )}
      </button>

      {open && (
        <div style={{ marginTop: 10 }}>
          <div
            style={{
              display: 'grid',
              gridTemplateColumns: '1fr 92px 92px 110px',
              gap: '8px 12px',
              alignItems: 'center',
            }}
          >
            <span style={head}>Model prefix</span>
            <span style={{ ...head, textAlign: 'right' }}>Input $/M</span>
            <span style={{ ...head, textAlign: 'right' }}>Output $/M</span>
            <span style={{ ...head, textAlign: 'right' }}>Context (tok)</span>
            {prefixes.map((prefix) => {
              const def = defaults[prefix];
              const row = rows[prefix] ?? blank();
              return (
                <React.Fragment key={prefix}>
                  <span
                    style={{
                      fontFamily: 'var(--wks-font-mono)',
                      fontSize: '0.72rem',
                      color: 'var(--wks-text-secondary)',
                      overflow: 'hidden',
                      textOverflow: 'ellipsis',
                    }}
                    title={prefix}
                  >
                    {prettyPrefix(prefix)}
                  </span>
                  <input
                    style={cell}
                    inputMode="decimal"
                    value={row.input}
                    placeholder={def ? String(def.input) : '—'}
                    onChange={(e) => setField(prefix, 'input', e.target.value)}
                  />
                  <input
                    style={cell}
                    inputMode="decimal"
                    value={row.output}
                    placeholder={def ? String(def.output) : '—'}
                    onChange={(e) => setField(prefix, 'output', e.target.value)}
                  />
                  <input
                    style={{ ...cell, width: 110 }}
                    inputMode="numeric"
                    value={row.context}
                    placeholder={def ? String(def.contextLimit) : '—'}
                    onChange={(e) => setField(prefix, 'context', e.target.value)}
                  />
                </React.Fragment>
              );
            })}
          </div>

          <div style={{ display: 'flex', alignItems: 'center', gap: 12, marginTop: 16 }}>
            <SmallButton
              label={status === 'saving' ? 'Saving…' : 'Save overrides'}
              onClick={save}
              primary
            />
            <SmallButton label="Reset all to defaults" onClick={resetAll} />
            {status === 'saved' && (
              <span style={{ fontSize: '0.72rem', color: 'var(--wks-success)' }}>Saved</span>
            )}
          </div>
        </div>
      )}
    </Section>
  );
};

function blank(): Editable {
  return { input: '', output: '', context: '' };
}

export default ModelPricingSection;
