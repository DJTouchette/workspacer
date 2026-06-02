import React, { useState, useRef, useEffect } from 'react';
import type { PromptVar } from '../lib/libraryTemplate';

interface Props {
  title: string;
  vars: PromptVar[];
  onSubmit: (values: Record<string, string>) => void;
  onCancel: () => void;
}

/** Collects the {{?…}} placeholder values for a library item before it runs. */
const PromptVarsDialog: React.FC<Props> = ({ title, vars, onSubmit, onCancel }) => {
  const [values, setValues] = useState<Record<string, string>>(() => {
    const init: Record<string, string> = {};
    for (const v of vars) init[v.token] = v.default;
    return init;
  });
  const firstRef = useRef<HTMLTextAreaElement>(null);
  useEffect(() => { setTimeout(() => firstRef.current?.focus(), 0); }, []);

  const submit = () => onSubmit(values);

  return (
    <div
      style={{ position: 'fixed', inset: 0, backgroundColor: 'var(--wks-overlay)', display: 'flex', justifyContent: 'center', paddingTop: '18vh', zIndex: 2200 }}
      onClick={onCancel}
    >
      <div
        style={{ backgroundColor: 'var(--wks-bg-raised)', border: '1px solid var(--wks-border-input)', borderRadius: 8, width: 460, maxHeight: '64vh', overflow: 'auto', boxShadow: '0 8px 32px var(--wks-shadow)', padding: 16 }}
        onClick={(e) => e.stopPropagation()}
        onKeyDown={(e) => {
          if (e.key === 'Escape') { e.preventDefault(); onCancel(); }
          if (e.key === 'Enter' && (e.metaKey || e.ctrlKey)) { e.preventDefault(); submit(); }
        }}
      >
        <div style={{ fontSize: '0.8rem', fontWeight: 600, color: 'var(--wks-text-primary)', marginBottom: 4 }}>{title}</div>
        <div style={{ fontSize: '0.65rem', color: 'var(--wks-text-faint)', marginBottom: 12 }}>Fill in the placeholders — ⌘/Ctrl+Enter to apply.</div>
        {vars.map((v, i) => (
          <div key={v.token} style={{ marginBottom: 10 }}>
            <div style={{ fontSize: '0.68rem', color: 'var(--wks-text-secondary)', marginBottom: 4 }}>{v.label}</div>
            <textarea
              ref={i === 0 ? firstRef : undefined}
              value={values[v.token] ?? ''}
              placeholder={v.default || ''}
              onChange={(e) => setValues((prev) => ({ ...prev, [v.token]: e.target.value }))}
              style={{
                width: '100%', minHeight: 44, resize: 'vertical', fontFamily: 'inherit', fontSize: '0.75rem',
                padding: '6px 8px', borderRadius: 5, outline: 'none',
                backgroundColor: 'var(--wks-bg-input)', color: 'var(--wks-text-primary)', border: '1px solid var(--wks-border-input)',
              }}
            />
          </div>
        ))}
        <div style={{ display: 'flex', justifyContent: 'flex-end', gap: 8, marginTop: 8 }}>
          <button onClick={onCancel} style={btn(false)}>Cancel</button>
          <button onClick={submit} style={btn(true)}>Apply</button>
        </div>
      </div>
    </div>
  );
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
