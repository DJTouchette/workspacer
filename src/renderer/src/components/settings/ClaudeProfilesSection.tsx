import React, { useState, useCallback, useEffect } from 'react';
import { Section, SmallButton, inputStyle } from './primitives';

interface ClaudeProfile {
  id: string;
  name: string;
  configDir: string;
  extraArgs: string[];
  isDefault: boolean;
}

const ProfileEditForm: React.FC<{
  name: string; configDir: string; args: string;
  onNameChange: (v: string) => void; onConfigDirChange: (v: string) => void; onArgsChange: (v: string) => void;
  onSave: () => void; onCancel: () => void;
}> = ({ name, configDir, args, onNameChange, onConfigDirChange, onArgsChange, onSave, onCancel }) => (
  <div style={{
    display: 'flex', flexDirection: 'column', gap: '4px', padding: '8px',
    backgroundColor: 'var(--wks-bg-surface)', borderRadius: '4px', border: '1px solid var(--wks-border-input)',
  }}>
    <input value={name} onChange={e => onNameChange(e.target.value)} placeholder="Profile name" style={inputStyle} autoFocus />
    <input value={configDir} onChange={e => onConfigDirChange(e.target.value)} placeholder="Config dir (e.g. ~/.claude-work, blank = default)" style={{ ...inputStyle, fontFamily: 'monospace' }} />
    <input
      value={args} onChange={e => onArgsChange(e.target.value)}
      placeholder="Extra args (e.g. --dangerously-skip-permissions)"
      style={{ ...inputStyle, fontFamily: 'monospace' }}
      onKeyDown={e => { if (e.key === 'Enter') onSave(); }}
    />
    <div style={{ display: 'flex', gap: '4px', justifyContent: 'flex-end' }}>
      <SmallButton label="Cancel" onClick={onCancel} />
      <SmallButton label="Save" onClick={onSave} primary />
    </div>
  </div>
);

const ClaudeProfilesSection: React.FC = () => {
  const [profiles, setProfiles] = useState<ClaudeProfile[]>([]);
  const [editing, setEditing] = useState<string | null>(null); // profile id or 'new'
  const [editName, setEditName] = useState('');
  const [editConfigDir, setEditConfigDir] = useState('');
  const [editArgs, setEditArgs] = useState('');

  const load = useCallback(() => {
    window.electronAPI.claudeProfilesList().then(p => setProfiles(p as ClaudeProfile[]));
  }, []);

  useEffect(() => { load(); }, [load]);

  const startEdit = (profile?: ClaudeProfile) => {
    if (profile) {
      setEditing(profile.id);
      setEditName(profile.name);
      setEditConfigDir(profile.configDir);
      setEditArgs(profile.extraArgs.join(' '));
    } else {
      setEditing('new');
      setEditName('');
      setEditConfigDir('');
      setEditArgs('');
    }
  };

  const cancelEdit = () => setEditing(null);

  const saveEdit = async () => {
    const args = editArgs.trim() ? editArgs.trim().split(/\s+/) : [];
    if (editing === 'new') {
      await window.electronAPI.claudeProfilesAdd(editName || 'Profile', editConfigDir, args);
    } else if (editing) {
      await window.electronAPI.claudeProfilesUpdate(editing, { name: editName, configDir: editConfigDir, extraArgs: args });
    }
    setEditing(null);
    load();
  };

  const setDefault = async (id: string) => {
    await window.electronAPI.claudeProfilesUpdate(id, { isDefault: true });
    load();
  };

  const remove = async (id: string) => {
    await window.electronAPI.claudeProfilesRemove(id);
    load();
  };

  return (
    <Section title="Claude Code">
      <div style={{ display: 'flex', flexDirection: 'column', gap: '4px' }}>
        {profiles.map(p => (
          <div key={p.id}>
            {editing === p.id ? (
              <ProfileEditForm
                name={editName} configDir={editConfigDir} args={editArgs}
                onNameChange={setEditName} onConfigDirChange={setEditConfigDir} onArgsChange={setEditArgs}
                onSave={saveEdit} onCancel={cancelEdit}
              />
            ) : (
              <div
                style={{
                  display: 'flex', alignItems: 'center', gap: '8px',
                  padding: '4px 8px', borderRadius: '4px',
                }}
                onMouseEnter={e => { (e.currentTarget as HTMLElement).style.backgroundColor = 'var(--wks-bg-hover)'; }}
                onMouseLeave={e => { (e.currentTarget as HTMLElement).style.backgroundColor = 'transparent'; }}
              >
                <span style={{ fontSize: '0.75rem', width: '16px', textAlign: 'center', color: p.isDefault ? 'var(--wks-accent)' : 'var(--wks-text-disabled)' }}>
                  {p.isDefault ? '♦' : '○'}
                </span>
                <div style={{ flex: 1, minWidth: 0 }}>
                  <div style={{ fontSize: '0.7rem', color: 'var(--wks-text-secondary)', fontWeight: 500 }}>
                    {p.name}
                    {p.isDefault && <span style={{ fontSize: '0.55rem', color: 'var(--wks-accent)', marginLeft: 6 }}>default</span>}
                  </div>
                  {p.configDir && (
                    <div style={{ fontSize: '0.58rem', color: 'var(--wks-text-faint)', fontFamily: 'monospace', overflow: 'hidden', textOverflow: 'ellipsis', whiteSpace: 'nowrap' }}>
                      {p.configDir}
                    </div>
                  )}
                  {p.extraArgs.length > 0 && (
                    <div style={{ fontSize: '0.55rem', color: 'var(--wks-text-disabled)', fontFamily: 'monospace' }}>
                      {p.extraArgs.join(' ')}
                    </div>
                  )}
                </div>
                {!p.isDefault && <SmallButton label="Default" onClick={() => setDefault(p.id)} />}
                <SmallButton label="Edit" onClick={() => startEdit(p)} />
                {p.id !== 'default' && <SmallButton label="✕" onClick={() => remove(p.id)} danger />}
              </div>
            )}
          </div>
        ))}

        {editing === 'new' ? (
          <ProfileEditForm
            name={editName} configDir={editConfigDir} args={editArgs}
            onNameChange={setEditName} onConfigDirChange={setEditConfigDir} onArgsChange={setEditArgs}
            onSave={saveEdit} onCancel={cancelEdit}
          />
        ) : (
          <button
            onClick={() => startEdit()}
            style={{
              padding: '6px 12px', fontSize: '0.65rem', fontFamily: 'inherit', fontWeight: 500,
              backgroundColor: 'transparent', color: 'var(--wks-text-muted)',
              border: '1px dashed var(--wks-border-input)', borderRadius: '4px',
              cursor: 'pointer', height: 'auto', lineHeight: '1.4', margin: '4px 0 0', width: '100%',
            }}
            onMouseEnter={e => { (e.currentTarget as HTMLElement).style.borderColor = 'var(--wks-accent)'; (e.currentTarget as HTMLElement).style.color = 'var(--wks-text-secondary)'; }}
            onMouseLeave={e => { (e.currentTarget as HTMLElement).style.borderColor = 'var(--wks-border-input)'; (e.currentTarget as HTMLElement).style.color = 'var(--wks-text-muted)'; }}
          >
            + Add Profile
          </button>
        )}
      </div>
    </Section>
  );
};

export default ClaudeProfilesSection;
