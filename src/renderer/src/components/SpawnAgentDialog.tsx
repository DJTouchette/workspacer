import React, { useEffect, useState } from 'react';
import { deriveAgentName } from '../hooks/useAgentManager';

interface SpawnAgentDialogProps {
  defaultCwd: string;
  onSpawn: (opts: { cwd: string; name?: string; profileId?: string }) => void;
  onCancel: () => void;
}

const SpawnAgentDialog: React.FC<SpawnAgentDialogProps> = ({ defaultCwd, onSpawn, onCancel }) => {
  const [cwd, setCwd] = useState(defaultCwd);
  const [name, setName] = useState('');
  const [profiles, setProfiles] = useState<Array<{ id: string; name: string }>>([]);
  const [profileId, setProfileId] = useState<string>('');

  useEffect(() => { setCwd(defaultCwd); }, [defaultCwd]);

  useEffect(() => {
    window.electronAPI.claudeProfilesList?.()
      .then((list: any[]) => setProfiles(list ?? []))
      .catch(() => {});
  }, []);

  const browse = async () => {
    const picked = await window.electronAPI.pickFolder?.(cwd || undefined);
    if (picked) setCwd(picked);
  };

  const submit = () => {
    if (!cwd.trim()) return;
    onSpawn({ cwd: cwd.trim(), name: name.trim() || undefined, profileId: profileId || undefined });
  };

  const placeholderName = cwd.trim() ? deriveAgentName(cwd.trim()) : 'agent';

  return (
    <div
      onMouseDown={onCancel}
      style={{
        position: 'fixed', inset: 0, zIndex: 20000,
        backgroundColor: 'var(--wks-overlay, rgba(0,0,0,0.5))',
        display: 'flex', alignItems: 'center', justifyContent: 'center',
      }}
    >
      <div
        onMouseDown={(e) => e.stopPropagation()}
        style={{
          width: 420, maxWidth: '90vw',
          backgroundColor: 'var(--wks-bg-surface)',
          border: '1px solid var(--wks-border-input)',
          borderRadius: 8,
          padding: 20,
          boxShadow: '0 8px 32px var(--wks-shadow)',
          fontFamily: 'inherit',
        }}
      >
        <div style={{ fontSize: '0.9rem', fontWeight: 600, color: 'var(--wks-text-primary)', marginBottom: 16 }}>
          Spawn agent
        </div>

        <Field label="Working directory">
          <div style={{ display: 'flex', gap: 6 }}>
            <input
              autoFocus
              value={cwd}
              onChange={(e) => setCwd(e.target.value)}
              onKeyDown={(e) => { if (e.key === 'Enter') submit(); if (e.key === 'Escape') onCancel(); }}
              placeholder="/path/to/project"
              style={inputStyle}
            />
            <button onClick={browse} style={browseBtnStyle}>Browse…</button>
          </div>
        </Field>

        <Field label="Name (optional)">
          <input
            value={name}
            onChange={(e) => setName(e.target.value)}
            onKeyDown={(e) => { if (e.key === 'Enter') submit(); if (e.key === 'Escape') onCancel(); }}
            placeholder={placeholderName}
            style={inputStyle}
          />
        </Field>

        {profiles.length > 0 && (
          <Field label="Profile (optional)">
            <select value={profileId} onChange={(e) => setProfileId(e.target.value)} style={inputStyle}>
              <option value="">Default</option>
              {profiles.map((p) => (
                <option key={p.id} value={p.id}>{p.name}</option>
              ))}
            </select>
          </Field>
        )}

        <div style={{ display: 'flex', justifyContent: 'flex-end', gap: 8, marginTop: 20 }}>
          <button onClick={onCancel} style={secondaryBtnStyle}>Cancel</button>
          <button onClick={submit} disabled={!cwd.trim()} style={primaryBtnStyle(!cwd.trim())}>Spawn</button>
        </div>
      </div>
    </div>
  );
};

function Field({ label, children }: { label: string; children: React.ReactNode }) {
  return (
    <div style={{ marginBottom: 12 }}>
      <div style={{ fontSize: '0.65rem', color: 'var(--wks-text-muted)', marginBottom: 4 }}>{label}</div>
      {children}
    </div>
  );
}

const inputStyle: React.CSSProperties = {
  flex: 1, width: '100%', boxSizing: 'border-box',
  fontSize: '0.8rem', fontFamily: 'inherit',
  background: 'var(--wks-bg-base)', color: 'var(--wks-text-primary)',
  border: '1px solid var(--wks-border-input)', borderRadius: 4, padding: '6px 8px',
};

const browseBtnStyle: React.CSSProperties = {
  fontSize: '0.75rem', fontFamily: 'inherit', cursor: 'pointer',
  background: 'var(--wks-bg-input)', color: 'var(--wks-text-tertiary)',
  border: '1px solid var(--wks-border-input)', borderRadius: 4, padding: '0 10px', whiteSpace: 'nowrap',
};

const secondaryBtnStyle: React.CSSProperties = {
  fontSize: '0.78rem', fontFamily: 'inherit', cursor: 'pointer',
  background: 'transparent', color: 'var(--wks-text-tertiary)',
  border: '1px solid var(--wks-border-input)', borderRadius: 4, padding: '6px 14px',
};

const primaryBtnStyle = (disabled: boolean): React.CSSProperties => ({
  fontSize: '0.78rem', fontFamily: 'inherit', cursor: disabled ? 'default' : 'pointer',
  background: disabled ? 'var(--wks-bg-input)' : 'var(--wks-accent)',
  color: disabled ? 'var(--wks-text-faint)' : 'var(--wks-text-on-accent, #fff)',
  border: 'none', borderRadius: 4, padding: '6px 14px', fontWeight: 600,
});

export default SpawnAgentDialog;
