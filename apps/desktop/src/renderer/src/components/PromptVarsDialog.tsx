import React, { useState, useRef, useEffect } from 'react';
import type { PromptVar } from '../lib/libraryTemplate';

interface Props {
  title: string;
  vars: PromptVar[];
  onSubmit: (values: Record<string, string>) => void;
  onCancel: () => void;
}

/**
 * The form a library item shows before it runs: one control per {{?…}} field,
 * picked by the field's type — paragraph (textarea), single-line (input),
 * dropdown (select), or checkbox (toggle). The collected values are keyed by
 * the field token and substituted into the prompt by applyTemplate().
 */
const PromptVarsDialog: React.FC<Props> = ({ title, vars, onSubmit, onCancel }) => {
  const [values, setValues] = useState<Record<string, string>>(() => {
    const init: Record<string, string> = {};
    for (const v of vars) init[v.token] = v.default;
    return init;
  });
  // Focus the first control on open (input / textarea / select all accept focus()).
  const firstRef = useRef<HTMLElement>(null);
  useEffect(() => { setTimeout(() => firstRef.current?.focus(), 0); }, []);

  const setValue = (token: string, value: string) => setValues((prev) => ({ ...prev, [token]: value }));
  const submit = () => onSubmit(values);

  return (
    <div
      style={{ position: 'fixed', inset: 0, backgroundColor: 'var(--wks-overlay)', display: 'flex', justifyContent: 'center', paddingTop: '14vh', zIndex: 2200 }}
      onClick={onCancel}
    >
      <div
        style={{ backgroundColor: 'var(--wks-bg-raised)', border: '1px solid var(--wks-border-input)', borderRadius: 8, width: 'min(480px, 92vw)', boxSizing: 'border-box', maxHeight: '72vh', overflow: 'auto', boxShadow: '0 8px 32px var(--wks-shadow)', padding: 16 }}
        onClick={(e) => e.stopPropagation()}
        onKeyDown={(e) => {
          if (e.key === 'Escape') { e.preventDefault(); onCancel(); }
          if (e.key === 'Enter' && (e.metaKey || e.ctrlKey)) { e.preventDefault(); submit(); }
        }}
      >
        <div style={{ fontSize: '0.8rem', fontWeight: 600, color: 'var(--wks-text-primary)', marginBottom: 4 }}>{title}</div>
        <div style={{ fontSize: '0.65rem', color: 'var(--wks-text-faint)', marginBottom: 12 }}>Fill in the fields — ⌘/Ctrl+Enter to apply.</div>
        {vars.map((v, i) => (
          <Field key={v.token} v={v} value={values[v.token] ?? ''} onChange={(val) => setValue(v.token, val)} fieldRef={i === 0 ? firstRef : undefined} />
        ))}
        <div style={{ display: 'flex', justifyContent: 'flex-end', gap: 8, marginTop: 8 }}>
          <button onClick={onCancel} style={btn(false)}>Cancel</button>
          <button onClick={submit} style={btn(true)}>Apply</button>
        </div>
      </div>
    </div>
  );
};

const Field: React.FC<{
  v: PromptVar;
  value: string;
  onChange: (value: string) => void;
  fieldRef?: React.Ref<HTMLElement>;
}> = ({ v, value, onChange, fieldRef }) => {
  // A toggle is a single labelled row; everything else is label-over-control.
  if (v.type === 'toggle') {
    const checked = value === v.onValue;
    return (
      <label style={{ display: 'flex', alignItems: 'center', gap: 8, marginBottom: 12, cursor: 'pointer', fontSize: '0.72rem', color: 'var(--wks-text-secondary)' }}>
        <input
          ref={fieldRef as React.Ref<HTMLInputElement>}
          type="checkbox"
          checked={checked}
          onChange={(e) => onChange(e.target.checked ? (v.onValue ?? '') : (v.offValue ?? ''))}
          style={{ width: 15, height: 15, accentColor: 'var(--wks-accent)', cursor: 'pointer' }}
        />
        {v.label}
      </label>
    );
  }

  return (
    <div style={{ marginBottom: 10 }}>
      <div style={{ fontSize: '0.68rem', color: 'var(--wks-text-secondary)', marginBottom: 4 }}>{v.label}</div>
      {v.type === 'select' ? (
        <select
          ref={fieldRef as React.Ref<HTMLSelectElement>}
          value={value}
          onChange={(e) => onChange(e.target.value)}
          style={{ ...controlStyle, height: 32, cursor: 'pointer' }}
        >
          {(v.options ?? []).map((opt) => <option key={opt} value={opt}>{opt}</option>)}
        </select>
      ) : v.type === 'text' ? (
        <input
          ref={fieldRef as React.Ref<HTMLInputElement>}
          value={value}
          placeholder={v.default || ''}
          onChange={(e) => onChange(e.target.value)}
          style={{ ...controlStyle, height: 32 }}
        />
      ) : (
        <textarea
          ref={fieldRef as React.Ref<HTMLTextAreaElement>}
          value={value}
          placeholder={v.default || ''}
          onChange={(e) => onChange(e.target.value)}
          style={{ ...controlStyle, minHeight: 96, resize: 'vertical', lineHeight: 1.5 }}
        />
      )}
    </div>
  );
};

const controlStyle: React.CSSProperties = {
  width: '100%', fontFamily: 'inherit', fontSize: '0.75rem',
  padding: '6px 8px', borderRadius: 5, outline: 'none', boxSizing: 'border-box',
  backgroundColor: 'var(--wks-bg-input)', color: 'var(--wks-text-primary)', border: '1px solid var(--wks-border-input)',
};

function btn(primary: boolean): React.CSSProperties {
  return {
    fontSize: '0.72rem', fontFamily: 'inherit', fontWeight: 600, cursor: 'pointer',
    padding: '6px 14px', borderRadius: 5,
    border: primary ? 'none' : '1px solid var(--wks-border-input)',
    background: primary ? 'var(--wks-accent)' : 'transparent',
    color: primary ? 'var(--wks-text-on-accent, #fff)' : 'var(--wks-text-secondary)',
  };
}

export default PromptVarsDialog;
