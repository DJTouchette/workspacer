import React, { useState, useEffect, useCallback, useRef } from 'react';
import { useConfig, AppEntry } from '../hooks/useConfig';
import { themes } from '../themes';

interface ClaudeProfile {
  id: string;
  name: string;
  configDir: string;
  extraArgs: string[];
  isDefault: boolean;
}

interface SettingsPaneProps {
  title: string;
}

const defaultShortcuts: [string, string][] = [
  ['Ctrl+1-9', 'Jump to pane'],
  ['Alt+Left/Right', 'Previous/Next pane'],
  ['Ctrl+T', 'New terminal'],
  ['Ctrl+B', 'New browser'],
  ['Ctrl+W', 'Close active pane'],
  ['Ctrl+Shift+Left/Right', 'Shrink/Grow pane'],
  ['Ctrl+Shift+0', 'Reset pane width'],
  ['Ctrl+Shift+1-9', 'Move pane to position'],
  ['F2', 'Rename active pane'],
  ['Ctrl+,', 'Open settings'],
  ['Ctrl+/', 'Toggle help'],
];

function vimShortcuts(leader: string): [string, string][] {
  const l = leader || 'Ctrl';
  return [
    [`${l} then 1-9`, 'Jump to pane'],
    [`${l} then h / l`, 'Prev / next pane'],
    [`${l} then H / L`, 'Move pane left / right'],
    [`${l} then n`, 'New terminal'],
    [`${l} then b`, 'New browser'],
    [`${l} then q`, 'Close pane'],
    [`${l} then r`, 'Rename pane'],
    [`${l} then + / -`, 'Grow / shrink pane'],
    [`${l} then =`, 'Reset pane width'],
    [`${l} then ?`, 'Toggle help'],
    ['', ''],
    ['Direct shortcuts (always active):', ''],
    ['Ctrl+T', 'New terminal'],
    ['Ctrl+B', 'New browser'],
    ['Ctrl+W', 'Close pane'],
    ['Ctrl+,', 'Open settings'],
    ['Ctrl+/', 'Toggle help'],
  ];
}

const SettingsPane: React.FC<SettingsPaneProps> = ({ title }) => {
  const { config, save } = useConfig();
  const kbConfig = config.keybindings ?? { mode: 'default' as const, leader: 'ctrl' };

  const [mode, setMode] = useState<'default' | 'vim'>(kbConfig.mode);
  const [leader, setLeader] = useState(kbConfig.leader);
  const [capturingLeader, setCapturingLeader] = useState(false);
  const leaderInputRef = useRef<HTMLInputElement>(null);

  const handleModeChange = useCallback((newMode: 'default' | 'vim') => {
    setMode(newMode);
    save({ keybindings: { mode: newMode, leader } });
  }, [leader, save]);

  const handleLeaderCapture = useCallback((e: React.KeyboardEvent) => {
    e.preventDefault();
    e.stopPropagation();

    // Build combo string from the event
    const parts: string[] = [];
    if (e.ctrlKey) parts.push('ctrl');
    if (e.altKey) parts.push('alt');
    if (e.shiftKey) parts.push('shift');
    if (e.metaKey) parts.push('meta');

    const key = e.key === ' ' ? 'space' : e.key.toLowerCase();

    // Allow bare modifier as leader (e.g. just "ctrl")
    if (['control', 'alt', 'shift', 'meta'].includes(key)) {
      const modMap: Record<string, string> = { control: 'ctrl', alt: 'alt', shift: 'shift', meta: 'meta' };
      const combo = modMap[key] || key;
      setLeader(combo);
      setCapturingLeader(false);
      save({ keybindings: { mode, leader: combo } });
      return;
    }

    parts.push(key);
    const combo = parts.join('+');

    setLeader(combo);
    setCapturingLeader(false);
    save({ keybindings: { mode, leader: combo } });
  }, [mode, save]);

  // --- Apps management ---
  const [apps, setApps] = useState<AppEntry[]>(config.apps ?? []);
  const [editingAppIndex, setEditingAppIndex] = useState<number | null>(null);
  const [editName, setEditName] = useState('');
  const [editUrl, setEditUrl] = useState('');
  const [editIcon, setEditIcon] = useState('');

  const saveApps = useCallback((newApps: AppEntry[]) => {
    setApps(newApps);
    save({ apps: newApps });
  }, [save]);

  const handleAddApp = useCallback(() => {
    const newApps = [...apps, { name: 'New App', url: 'https://', icon: '\u{1F310}' }];
    saveApps(newApps);
    setEditingAppIndex(newApps.length - 1);
    setEditName('New App');
    setEditUrl('https://');
    setEditIcon('\u{1F310}');
  }, [apps, saveApps]);

  const handleEditApp = useCallback((index: number) => {
    const app = apps[index];
    setEditingAppIndex(index);
    setEditName(app.name);
    setEditUrl(app.url);
    setEditIcon(app.icon || '');
  }, [apps]);

  const handleSaveApp = useCallback(() => {
    if (editingAppIndex === null) return;
    const newApps = [...apps];
    newApps[editingAppIndex] = { name: editName.trim() || 'App', url: editUrl.trim() || 'https://', icon: editIcon.trim() || undefined };
    saveApps(newApps);
    setEditingAppIndex(null);
  }, [editingAppIndex, editName, editUrl, editIcon, apps, saveApps]);

  const handleDeleteApp = useCallback((index: number) => {
    const newApps = apps.filter((_, i) => i !== index);
    saveApps(newApps);
    if (editingAppIndex === index) setEditingAppIndex(null);
  }, [apps, saveApps, editingAppIndex]);

  const shortcuts = mode === 'vim' ? vimShortcuts(leader) : defaultShortcuts;

  return (
    <div
      style={{
        display: 'flex',
        flexDirection: 'column',
        height: '100%',
        backgroundColor: 'var(--wks-bg-base)',
        color: 'var(--wks-text-secondary)',
        fontFamily: 'Inter, system-ui, sans-serif',
        fontSize: '12px',
        overflow: 'auto',
        padding: '16px 24px',
      }}
    >
      <h2 style={{ fontSize: '0.9rem', fontWeight: 600, margin: '0 0 16px 0', color: 'var(--wks-text-primary)' }}>
        Settings
      </h2>

      {/* Appearance section */}
      <Section title="Appearance">
        <Row label="Theme">
          <div style={{ display: 'flex', gap: '4px' }}>
            {Object.keys(themes).map((themeName) => (
              <ModeButton
                key={themeName}
                label={themeName.charAt(0).toUpperCase() + themeName.slice(1)}
                active={config.ui.theme === themeName}
                onClick={() => save({ ui: { ...config.ui, theme: themeName } })}
              />
            ))}
          </div>
        </Row>
      </Section>

      {/* Keybindings section */}
      <Section title="Keybindings">
        <Row label="Mode">
          <div style={{ display: 'flex', gap: '4px' }}>
            <ModeButton
              label="Default"
              active={mode === 'default'}
              onClick={() => handleModeChange('default')}
            />
            <ModeButton
              label="Vim"
              active={mode === 'vim'}
              onClick={() => handleModeChange('vim')}
            />
          </div>
        </Row>

        {mode === 'vim' && (
          <Row label="Leader Key">
            <div style={{ display: 'flex', alignItems: 'center', gap: '8px' }}>
              <input
                ref={leaderInputRef}
                data-leader-capture="true"
                readOnly={!capturingLeader}
                value={capturingLeader ? 'Press key combo...' : leader}
                onKeyDown={capturingLeader ? handleLeaderCapture : undefined}
                onBlur={() => setCapturingLeader(false)}
                onClick={() => {
                  setCapturingLeader(true);
                  setTimeout(() => leaderInputRef.current?.focus(), 0);
                }}
                style={{
                  width: '160px',
                  height: '24px',
                  padding: '0 8px',
                  fontSize: '11px',
                  fontFamily: 'monospace',
                  backgroundColor: capturingLeader ? 'var(--wks-bg-selected)' : 'var(--wks-bg-input)',
                  color: capturingLeader ? 'var(--wks-accent-text)' : 'var(--wks-text-secondary)',
                  border: capturingLeader ? '1px solid var(--wks-accent)' : '1px solid var(--wks-border)',
                  borderRadius: '3px',
                  outline: 'none',
                  cursor: 'pointer',
                }}
              />
            </div>
          </Row>
        )}

        {/* Binding reference */}
        <div style={{ marginTop: '12px' }}>
          <div style={{ fontSize: '0.65rem', color: 'var(--wks-text-faint)', textTransform: 'uppercase', letterSpacing: '0.05em', marginBottom: '6px' }}>
            {mode === 'vim' ? 'Vim Bindings' : 'Default Bindings'}
          </div>
          <table style={{ width: '100%', borderCollapse: 'collapse', fontSize: '0.7rem' }}>
            <tbody>
              {shortcuts.map(([key, desc], i) => {
                if (!key && !desc) {
                  return <tr key={i}><td colSpan={2} style={{ height: '8px' }} /></tr>;
                }
                if (!desc) {
                  return (
                    <tr key={i}>
                      <td colSpan={2} style={{ padding: '4px 0 2px', color: 'var(--wks-text-faint)', fontSize: '0.6rem', fontWeight: 600 }}>
                        {key}
                      </td>
                    </tr>
                  );
                }
                return (
                  <tr key={i}>
                    <td style={{ padding: '2px 16px 2px 0', color: 'var(--wks-text-tertiary)', fontFamily: 'monospace', fontSize: '0.65rem', whiteSpace: 'nowrap' }}>
                      {key}
                    </td>
                    <td style={{ padding: '2px 0', color: 'var(--wks-text-muted)' }}>
                      {desc}
                    </td>
                  </tr>
                );
              })}
            </tbody>
          </table>
        </div>
      </Section>

      {/* Browser section */}
      <Section title="Browser">
        <Row label="Hibernate after (seconds)">
          <input
            type="number"
            min={0}
            step={30}
            value={config.browser?.hibernateAfter ?? 300}
            onChange={(e) => {
              const val = parseInt(e.target.value, 10);
              if (!isNaN(val) && val >= 0) {
                save({ browser: { ...config.browser, hibernateAfter: val } });
              }
            }}
            style={{
              width: '80px',
              height: '24px',
              padding: '0 8px',
              fontSize: '0.65rem',
              backgroundColor: 'var(--wks-bg-input)',
              color: 'var(--wks-text-secondary)',
              border: '1px solid var(--wks-border)',
              borderRadius: '3px',
              outline: 'none',
              fontFamily: 'monospace',
              textAlign: 'right',
            }}
          />
        </Row>
        <div style={{ fontSize: '0.55rem', color: 'var(--wks-text-disabled)' }}>
          Browser panes hibernate after being out of view. 0 = disabled.
        </div>
      </Section>

      {/* Apps section */}
      <Section title="Apps (Ctrl+K)">
        <div style={{ display: 'flex', flexDirection: 'column', gap: '4px' }}>
          {apps.map((app, i) => (
            <div key={i}>
              {editingAppIndex === i ? (
                /* Edit mode */
                <div style={{
                  display: 'flex',
                  flexDirection: 'column',
                  gap: '4px',
                  padding: '8px',
                  backgroundColor: 'var(--wks-bg-surface)',
                  borderRadius: '4px',
                  border: '1px solid var(--wks-border-input)',
                }}>
                  <div style={{ display: 'flex', gap: '4px' }}>
                    <input
                      value={editIcon}
                      onChange={(e) => setEditIcon(e.target.value)}
                      placeholder="Icon"
                      style={{ ...inputStyle, width: '40px', textAlign: 'center' }}
                    />
                    <input
                      value={editName}
                      onChange={(e) => setEditName(e.target.value)}
                      placeholder="Name"
                      style={{ ...inputStyle, flex: 1 }}
                      autoFocus
                    />
                  </div>
                  <input
                    value={editUrl}
                    onChange={(e) => setEditUrl(e.target.value)}
                    placeholder="https://..."
                    style={{ ...inputStyle, fontFamily: 'monospace' }}
                    onKeyDown={(e) => { if (e.key === 'Enter') handleSaveApp(); }}
                  />
                  <div style={{ display: 'flex', gap: '4px', justifyContent: 'flex-end' }}>
                    <SmallButton label="Cancel" onClick={() => setEditingAppIndex(null)} />
                    <SmallButton label="Save" onClick={handleSaveApp} primary />
                  </div>
                </div>
              ) : (
                /* Display mode */
                <div
                  style={{
                    display: 'flex',
                    alignItems: 'center',
                    gap: '8px',
                    padding: '4px 8px',
                    borderRadius: '4px',
                  }}
                  onMouseEnter={(e) => { (e.currentTarget as HTMLElement).style.backgroundColor = 'var(--wks-bg-hover)'; }}
                  onMouseLeave={(e) => { (e.currentTarget as HTMLElement).style.backgroundColor = 'transparent'; }}
                >
                  <span style={{ fontSize: '0.85rem', width: '20px', textAlign: 'center' }}>
                    {app.icon || '\u{1F310}'}
                  </span>
                  <div style={{ flex: 1, minWidth: 0 }}>
                    <div style={{ fontSize: '0.7rem', color: 'var(--wks-text-secondary)', fontWeight: 500 }}>{app.name}</div>
                    <div style={{ fontSize: '0.6rem', color: 'var(--wks-text-faint)', overflow: 'hidden', textOverflow: 'ellipsis', whiteSpace: 'nowrap' }}>{app.url}</div>
                  </div>
                  <SmallButton label="Edit" onClick={() => handleEditApp(i)} />
                  <SmallButton label="\u2715" onClick={() => handleDeleteApp(i)} danger />
                </div>
              )}
            </div>
          ))}

          <button
            onClick={handleAddApp}
            style={{
              padding: '6px 12px',
              fontSize: '0.65rem',
              fontFamily: 'inherit',
              fontWeight: 500,
              backgroundColor: 'var(--wks-bg-elevated)',
              color: 'var(--wks-text-muted)',
              border: '1px dashed var(--wks-border-input)',
              borderRadius: '4px',
              cursor: 'pointer',
              height: 'auto',
              lineHeight: '1.4',
              margin: '4px 0 0',
              width: '100%',
            }}
            onMouseEnter={(e) => { (e.currentTarget as HTMLElement).style.borderColor = 'var(--wks-accent)'; (e.currentTarget as HTMLElement).style.color = 'var(--wks-text-secondary)'; }}
            onMouseLeave={(e) => { (e.currentTarget as HTMLElement).style.borderColor = 'var(--wks-border-input)'; (e.currentTarget as HTMLElement).style.color = 'var(--wks-text-muted)'; }}
          >
            + Add App
          </button>
        </div>
      </Section>

      {/* Claude Profiles section */}
      <ClaudeProfilesSection />
    </div>
  );
};

// ── Claude Profiles Section ──

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
                  {p.isDefault ? '\u2666' : '\u25CB'}
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
                {p.id !== 'default' && <SmallButton label="\u2715" onClick={() => remove(p.id)} danger />}
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

const inputStyle: React.CSSProperties = {
  height: '24px',
  padding: '0 8px',
  fontSize: '0.65rem',
  backgroundColor: 'var(--wks-bg-input)',
  color: 'var(--wks-text-secondary)',
  border: '1px solid var(--wks-border)',
  borderRadius: '3px',
  outline: 'none',
  fontFamily: 'inherit',
  boxSizing: 'border-box' as const,
};

function SmallButton({ label, onClick, primary, danger }: { label: string; onClick: () => void; primary?: boolean; danger?: boolean }) {
  return (
    <button
      onClick={(e) => { e.stopPropagation(); onClick(); }}
      style={{
        padding: '2px 8px',
        fontSize: '0.6rem',
        fontFamily: 'inherit',
        fontWeight: 500,
        backgroundColor: primary ? 'var(--wks-accent)' : 'transparent',
        color: danger ? 'var(--wks-error)' : primary ? '#fff' : 'var(--wks-text-muted)',
        border: primary ? '1px solid var(--wks-accent)' : '1px solid var(--wks-border)',
        borderRadius: '3px',
        cursor: 'pointer',
        height: 'auto',
        lineHeight: '1.4',
        margin: 0,
        width: 'auto',
      }}
    >
      {label}
    </button>
  );
}

function Section({ title, children }: { title: string; children: React.ReactNode }) {
  return (
    <div style={{ marginBottom: '20px' }}>
      <div style={{
        fontSize: '0.7rem',
        fontWeight: 600,
        color: 'var(--wks-text-tertiary)',
        marginBottom: '8px',
        paddingBottom: '4px',
        borderBottom: '1px solid var(--wks-border)',
      }}>
        {title}
      </div>
      <div style={{ display: 'flex', flexDirection: 'column', gap: '8px' }}>
        {children}
      </div>
    </div>
  );
}

function Row({ label, children }: { label: string; children: React.ReactNode }) {
  return (
    <div style={{ display: 'flex', alignItems: 'center', justifyContent: 'space-between' }}>
      <span style={{ fontSize: '0.7rem', color: 'var(--wks-text-muted)' }}>{label}</span>
      {children}
    </div>
  );
}

function ModeButton({ label, active, onClick }: { label: string; active: boolean; onClick: () => void }) {
  return (
    <button
      onClick={onClick}
      style={{
        padding: '3px 12px',
        fontSize: '0.65rem',
        fontFamily: 'inherit',
        fontWeight: active ? 600 : 400,
        backgroundColor: active ? 'var(--wks-accent)' : 'var(--wks-bg-elevated)',
        color: active ? '#fff' : 'var(--wks-text-muted)',
        border: active ? '1px solid var(--wks-accent)' : '1px solid var(--wks-border)',
        borderRadius: '3px',
        cursor: 'pointer',
        height: '24px',
        lineHeight: '1',
        margin: 0,
        width: 'auto',
      }}
    >
      {label}
    </button>
  );
}

export default SettingsPane;
