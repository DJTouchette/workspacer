import React, { useEffect, useState } from 'react';
import { deriveAgentName } from '../hooks/useAgentManager';

interface SpawnAgentDialogProps {
  defaultCwd: string;
  onSpawn: (opts: { cwd: string; name?: string; profileId?: string; model?: string; skipPermissions?: boolean; resumeSessionId?: string }) => void;
  onCancel: () => void;
}

const CUSTOM = '__custom__';

const SpawnAgentDialog: React.FC<SpawnAgentDialogProps> = ({ defaultCwd, onSpawn, onCancel }) => {
  const [cwd, setCwd] = useState(defaultCwd);
  const [name, setName] = useState('');
  const [profiles, setProfiles] = useState<Array<{ id: string; name: string }>>([]);
  const [profileId, setProfileId] = useState<string>('');

  // Model selection. `modelSel` is the dropdown value (''=Default, an alias/id,
  // or the CUSTOM sentinel); `customModel` holds the free-text id when CUSTOM.
  const [aliases, setAliases] = useState<Array<{ value: string; label: string }>>([]);
  const [seen, setSeen] = useState<string[]>([]);
  const [modelSel, setModelSel] = useState<string>('');
  const [customModel, setCustomModel] = useState('');
  const [skipPermissions, setSkipPermissions] = useState(false);

  // Resume an existing Claude session in this cwd. ''=start fresh.
  const [sessions, setSessions] = useState<Array<{ sessionId: string; timestamp: string; summary: string }>>([]);
  const [resumeSessionId, setResumeSessionId] = useState('');

  useEffect(() => { setCwd(defaultCwd); }, [defaultCwd]);

  // Discover resumable sessions whenever the directory settles (debounced).
  useEffect(() => {
    const dir = cwd.trim();
    if (!dir) { setSessions([]); return; }
    let cancelled = false;
    const handle = setTimeout(() => {
      window.electronAPI.claudeListSessionsForDir?.(dir)
        .then((list) => { if (!cancelled) setSessions(list ?? []); })
        .catch(() => { if (!cancelled) setSessions([]); });
    }, 250);
    return () => { cancelled = true; clearTimeout(handle); };
  }, [cwd]);

  // If the picked session disappears from the list (cwd changed), reset to fresh.
  useEffect(() => {
    if (resumeSessionId && !sessions.some((s) => s.sessionId === resumeSessionId)) setResumeSessionId('');
  }, [sessions, resumeSessionId]);

  useEffect(() => {
    window.electronAPI.claudeProfilesList?.()
      .then((list: any[]) => setProfiles(list ?? []))
      .catch(() => {});
    window.electronAPI.claudeListModels?.()
      .then((res) => {
        if (!res) return;
        setAliases(res.aliases ?? []);
        setSeen(res.seen ?? []);
        setSkipPermissions(res.skipPermissionsDefault === true);
        // Pre-select the saved default. If it's a concrete id we don't have in
        // a list, keep it as a custom entry so the saved value isn't dropped.
        const d = res.defaultModel ?? '';
        const known = d === '' || (res.aliases ?? []).some((a) => a.value === d) || (res.seen ?? []).includes(d);
        if (known) { setModelSel(d); } else { setModelSel(CUSTOM); setCustomModel(d); }
      })
      .catch(() => {});
  }, []);

  const resolvedModel = modelSel === CUSTOM ? customModel.trim() : modelSel;

  const browse = async () => {
    const picked = await window.electronAPI.pickFolder?.(cwd || undefined);
    if (picked) setCwd(picked);
  };

  const submit = () => {
    if (!cwd.trim()) return;
    onSpawn({ cwd: cwd.trim(), name: name.trim() || undefined, profileId: profileId || undefined, model: resolvedModel || undefined, skipPermissions, resumeSessionId: resumeSessionId || undefined });
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
          backgroundColor: 'var(--wks-glass-strong)',
          backdropFilter: 'blur(var(--wks-glass-blur)) saturate(170%)',
          WebkitBackdropFilter: 'blur(var(--wks-glass-blur)) saturate(170%)',
          border: '1px solid var(--wks-glass-border)',
          borderRadius: 'var(--wks-radius-lg)',
          padding: 20,
          boxShadow: '0 16px 48px var(--wks-glass-shadow), inset 0 0 0 1.5px var(--wks-glass-highlight)',
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

        {sessions.length > 0 && (
          <Field label="Resume session (optional)">
            <select value={resumeSessionId} onChange={(e) => setResumeSessionId(e.target.value)} style={inputStyle}>
              <option value="">Start fresh</option>
              {sessions.map((s) => (
                <option key={s.sessionId} value={s.sessionId}>
                  {relTime(s.timestamp)} — {s.summary}
                </option>
              ))}
            </select>
          </Field>
        )}

        <Field label="Model">
          <select value={modelSel} onChange={(e) => setModelSel(e.target.value)} style={inputStyle}>
            <option value="">Default (Claude Code setting)</option>
            {aliases.length > 0 && (
              <optgroup label="Latest">
                {aliases.map((a) => (
                  <option key={a.value} value={a.value}>{a.label}</option>
                ))}
              </optgroup>
            )}
            {seen.length > 0 && (
              <optgroup label="Seen in sessions">
                {seen.map((m) => (
                  <option key={m} value={m}>{m}</option>
                ))}
              </optgroup>
            )}
            <option value={CUSTOM}>Custom…</option>
          </select>
          {modelSel === CUSTOM && (
            <input
              value={customModel}
              onChange={(e) => setCustomModel(e.target.value)}
              onKeyDown={(e) => { if (e.key === 'Enter') submit(); if (e.key === 'Escape') onCancel(); }}
              placeholder="claude-opus-4-8  or  opus"
              style={{ ...inputStyle, marginTop: 6 }}
            />
          )}
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

        <label style={{
          display: 'flex', alignItems: 'flex-start', gap: 8, marginTop: 4, cursor: 'pointer',
        }}>
          <input
            type="checkbox"
            checked={skipPermissions}
            onChange={(e) => setSkipPermissions(e.target.checked)}
            style={{ marginTop: 2, cursor: 'pointer' }}
          />
          <span style={{ fontSize: '0.72rem', lineHeight: 1.4 }}>
            <span style={{ color: 'var(--wks-text-primary)' }}>Skip permissions</span>
            <span style={{ color: 'var(--wks-danger, #e05555)', marginLeft: 6 }}>dangerous</span>
            <div style={{ color: 'var(--wks-text-faint)', fontSize: '0.65rem', marginTop: 1 }}>
              Bypasses all approval prompts (--dangerously-skip-permissions).
            </div>
          </span>
        </label>

        <div style={{ display: 'flex', justifyContent: 'flex-end', gap: 8, marginTop: 20 }}>
          <button onClick={onCancel} style={secondaryBtnStyle}>Cancel</button>
          <button onClick={submit} disabled={!cwd.trim()} style={primaryBtnStyle(!cwd.trim())}>Spawn</button>
        </div>
      </div>
    </div>
  );
};

/** Compact relative time for the resume picker, e.g. "2h ago", "3d ago". */
function relTime(iso: string): string {
  const t = new Date(iso).getTime();
  if (!Number.isFinite(t)) return '';
  const s = Math.max(0, (Date.now() - t) / 1000);
  if (s < 60) return 'just now';
  const m = s / 60;
  if (m < 60) return `${Math.floor(m)}m ago`;
  const h = m / 60;
  if (h < 24) return `${Math.floor(h)}h ago`;
  return `${Math.floor(h / 24)}d ago`;
}

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
