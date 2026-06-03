import React, { useState, useEffect, useCallback, useRef } from 'react';
import { useConfig, Config, AppEntry } from '../hooks/useConfig';
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
  ['Ctrl+Alt+Up/Down', 'Previous/Next agent'],
  ['Ctrl+Alt+N', 'Spawn agent'],
  ['Ctrl+1-9', 'Jump to tab'],
  ['Ctrl+[ / ]', 'Previous/Next tab'],
  ['Ctrl+H/L', 'Navigate panes'],
  ['Ctrl+T', 'New terminal'],
  ['Ctrl+B', 'New browser'],
  ['Ctrl+W', 'Close active pane'],
  ['Ctrl+Shift+1-9', 'Move tab to position'],
  ['F2', 'Rename active tab'],
  ['Ctrl+,', 'Open settings'],
  ['Ctrl+/', 'Toggle help'],
];

function vimShortcuts(leader: string): [string, string][] {
  const l = leader || 'Ctrl';
  return [
    [`${l} then k / j`, 'Prev / next agent'],
    [`${l} then a`, 'Spawn agent'],
    [`${l} then 1-9`, 'Jump to tab'],
    [`${l} then h / l`, 'Prev / next tab'],
    [`${l} then H / L`, 'Move tab left / right'],
    [`${l} then n`, 'New terminal'],
    [`${l} then b`, 'New browser'],
    [`${l} then q`, 'Close tab'],
    [`${l} then r`, 'Rename tab'],
    [`${l} then ?`, 'Toggle help'],
    ['', ''],
    ['Direct shortcuts (always active):', ''],
    ['Ctrl+Alt+Up/Down', 'Prev / next agent'],
    ['Ctrl+Alt+N', 'Spawn agent'],
    ['Ctrl+T', 'New terminal'],
    ['Ctrl+B', 'New browser'],
    ['Ctrl+W', 'Close pane'],
    ['Ctrl+,', 'Open settings'],
    ['Ctrl+/', 'Toggle help'],
  ];
}

// ── Shortcut Editor ──

const SHORTCUT_LABELS: Record<string, string> = {
  'new-terminal': 'New Terminal',
  'new-browser': 'New Browser',
  'new-claude': 'New Claude',
  'split': 'Split Pane',
  'quick-split': 'Quick Split (clone)',
  'close-pane': 'Close Pane',
  'command-palette': 'Command Palette',
  'settings': 'Settings',
  'save-session': 'Save Session',
  'rename-tab': 'Rename Tab',
  'toggle-help': 'Toggle Help',
  'prev-tab': 'Previous Tab',
  'next-tab': 'Next Tab',
  'nav-left': 'Navigate Left',
  'nav-right': 'Navigate Right',
  'nav-up': 'Navigate Up',
  'nav-down': 'Navigate Down',
  'prev-agent': 'Previous Agent',
  'next-agent': 'Next Agent',
  'spawn-agent': 'Spawn Agent',
};

const ShortcutEditor: React.FC<{ config: Config; save: (partial: Partial<Config>) => Promise<Config> }> = ({ config, save }) => {
  const currentShortcuts = config.keybindings?.shortcuts ?? {};
  const [capturing, setCapturing] = useState<string | null>(null);

  const handleCapture = useCallback((action: string, e: React.KeyboardEvent) => {
    e.preventDefault();
    e.stopPropagation();

    // Ignore bare modifier presses
    if (['Control', 'Alt', 'Shift', 'Meta'].includes(e.key)) return;

    const parts: string[] = [];
    if (e.ctrlKey) parts.push('ctrl');
    if (e.altKey) parts.push('alt');
    if (e.shiftKey) parts.push('shift');
    if (e.metaKey) parts.push('meta');

    const key = e.key === ' ' ? 'space' : e.key.length === 1 ? e.key.toLowerCase() : e.key.toLowerCase();
    parts.push(key);

    const combo = parts.join('+');
    const updated = { ...currentShortcuts, [action]: combo };
    save({ keybindings: { ...config.keybindings, shortcuts: updated } });
    setCapturing(null);
  }, [config.keybindings, currentShortcuts, save]);

  const handleReset = useCallback((action: string) => {
    const defaults: Record<string, string> = {
      'new-terminal': 'ctrl+t', 'new-browser': 'ctrl+n', 'new-claude': 'ctrl+j',
      'split': 'ctrl+d', 'quick-split': 'ctrl+shift+d', 'close-pane': 'ctrl+w',
      'command-palette': 'ctrl+k', 'settings': 'ctrl+,', 'save-session': 'ctrl+s',
      'rename-tab': 'f2', 'toggle-help': 'ctrl+?', 'prev-tab': 'ctrl+[',
      'next-tab': 'ctrl+]', 'nav-left': 'ctrl+h', 'nav-right': 'ctrl+l',
      'nav-up': 'ctrl+shift+k', 'nav-down': 'ctrl+shift+j',
      'prev-agent': 'ctrl+alt+arrowup', 'next-agent': 'ctrl+alt+arrowdown',
      'spawn-agent': 'ctrl+alt+n',
    };
    const updated = { ...currentShortcuts, [action]: defaults[action] ?? '' };
    save({ keybindings: { ...config.keybindings, shortcuts: updated } });
  }, [config.keybindings, currentShortcuts, save]);

  return (
    <div style={{ marginTop: '12px' }}>
      <div style={{ fontSize: '0.65rem', color: 'var(--wks-text-faint)', textTransform: 'uppercase', letterSpacing: '0.05em', marginBottom: '8px' }}>
        Shortcuts (click to rebind)
      </div>
      <div style={{ display: 'flex', flexDirection: 'column', gap: '2px' }}>
        {Object.entries(SHORTCUT_LABELS).map(([action, label]) => {
          const isCapturing = capturing === action;
          const combo = currentShortcuts[action] ?? '';
          return (
            <div key={action} style={{
              display: 'flex', alignItems: 'center', justifyContent: 'space-between',
              padding: '4px 8px', borderRadius: '4px',
              backgroundColor: isCapturing ? 'var(--wks-bg-selected)' : 'transparent',
            }}>
              <span style={{ fontSize: '0.7rem', color: 'var(--wks-text-muted)' }}>{label}</span>
              <div style={{ display: 'flex', alignItems: 'center', gap: '4px' }}>
                <input
                  data-leader-capture="true"
                  readOnly
                  value={isCapturing ? 'Press keys...' : combo}
                  onClick={() => setCapturing(action)}
                  onKeyDown={isCapturing ? (e) => handleCapture(action, e) : undefined}
                  onBlur={() => setCapturing(null)}
                  style={{
                    width: '140px', height: '22px', padding: '0 6px',
                    fontSize: '0.65rem', fontFamily: 'monospace', textAlign: 'center',
                    backgroundColor: isCapturing ? 'var(--wks-bg-input)' : 'transparent',
                    color: isCapturing ? 'var(--wks-accent-text)' : 'var(--wks-text-tertiary)',
                    border: isCapturing ? '1px solid var(--wks-accent)' : '1px solid var(--wks-border)',
                    borderRadius: '3px', outline: 'none', cursor: 'pointer',
                  }}
                />
                <button
                  onClick={() => handleReset(action)}
                  style={{
                    fontSize: '0.6rem', padding: '2px 6px', borderRadius: '3px',
                    border: '1px solid var(--wks-border)', backgroundColor: 'transparent',
                    color: 'var(--wks-text-faint)', cursor: 'pointer',
                  }}
                  title="Reset to default"
                >
                  ↺
                </button>
              </div>
            </div>
          );
        })}
      </div>
    </div>
  );
};

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

      {/* Layout section */}
      <Section title="Layout">
        <Row label="Peek">
          <div style={{ display: 'flex', alignItems: 'center', gap: '8px' }}>
            <input
              type="range"
              min={0}
              max={200}
              step={10}
              value={config.panes?.peek ?? 80}
              onChange={(e) => save({ panes: { ...config.panes, peek: parseInt(e.target.value) } })}
              style={{ width: '120px', accentColor: 'var(--wks-accent)' }}
            />
            <span style={{ fontSize: '0.7rem', fontFamily: 'monospace', color: 'var(--wks-text-tertiary)', minWidth: '32px' }}>
              {config.panes?.peek ?? 80}px
            </span>
          </div>
        </Row>
        <Row label="Gap">
          <div style={{ display: 'flex', alignItems: 'center', gap: '8px' }}>
            <input
              type="range"
              min={0}
              max={40}
              step={2}
              value={config.panes?.gap ?? 16}
              onChange={(e) => save({ panes: { ...config.panes, gap: parseInt(e.target.value) } })}
              style={{ width: '120px', accentColor: 'var(--wks-accent)' }}
            />
            <span style={{ fontSize: '0.7rem', fontFamily: 'monospace', color: 'var(--wks-text-tertiary)', minWidth: '32px' }}>
              {config.panes?.gap ?? 16}px
            </span>
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

        {/* Shortcut editor */}
        <ShortcutEditor config={config} save={save} />

        {/* Binding reference (vim chords) */}
        {mode === 'vim' && (
          <div style={{ marginTop: '12px' }}>
            <div style={{ fontSize: '0.65rem', color: 'var(--wks-text-faint)', textTransform: 'uppercase', letterSpacing: '0.05em', marginBottom: '6px' }}>
              Vim Chord Bindings (Leader + key)
            </div>
            <table style={{ width: '100%', borderCollapse: 'collapse', fontSize: '0.7rem' }}>
              <tbody>
                {shortcuts.filter(([_, desc]) => desc).map(([key, desc], i) => (
                  <tr key={i}>
                    <td style={{ padding: '2px 16px 2px 0', color: 'var(--wks-text-tertiary)', fontFamily: 'monospace', fontSize: '0.65rem', whiteSpace: 'nowrap' }}>
                      {key}
                    </td>
                    <td style={{ padding: '2px 0', color: 'var(--wks-text-muted)' }}>
                      {desc}
                    </td>
                  </tr>
                ))}
              </tbody>
            </table>
          </div>
        )}
      </Section>

      {/* Notifications section */}
      <NotificationsSection config={config} save={save} />

      {/* Session section */}
      <SessionSection config={config} save={save} />

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

        <ChromeCookieSyncRow />
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

// ── Notifications Section ──

const NOTIF_DEFAULTS = { enabled: true, notifyDone: true, onlyWhenUnwatched: true, sound: false };

const NotificationsSection: React.FC<{ config: Config; save: (partial: Partial<Config>) => Promise<Config> }> = ({ config, save }) => {
  const notif = config.notifications ?? NOTIF_DEFAULTS;
  const set = (patch: Partial<typeof notif>) => save({ notifications: { ...notif, ...patch } });

  return (
    <Section title="Notifications">
      <CheckRow
        label="Desktop notifications"
        checked={notif.enabled}
        onChange={(v) => set({ enabled: v })}
      />
      <CheckRow
        label="Notify when an agent finishes"
        checked={notif.notifyDone}
        disabled={!notif.enabled}
        onChange={(v) => set({ notifyDone: v })}
      />
      <CheckRow
        label="Only when I'm not watching that agent"
        checked={notif.onlyWhenUnwatched}
        disabled={!notif.enabled}
        onChange={(v) => set({ onlyWhenUnwatched: v })}
      />
      <CheckRow
        label="Play a sound"
        checked={notif.sound}
        disabled={!notif.enabled}
        onChange={(v) => set({ sound: v })}
      />
      <div style={{ fontSize: '0.55rem', color: 'var(--wks-text-disabled)' }}>
        Alerts when an agent needs approval/input or finishes. Ctrl+Alt+→ jumps to the next agent that needs you.
      </div>
    </Section>
  );
};

// ── Session Section ──

const SessionSection: React.FC<{ config: Config; save: (partial: Partial<Config>) => Promise<Config> }> = ({ config, save }) => {
  const autoResume = config.session?.autoResume ?? true;
  return (
    <Section title="Session">
      <CheckRow
        label="Restore my last session on launch"
        checked={autoResume}
        onChange={(v) => save({ session: { autoResume: v } })}
      />
      <div style={{ fontSize: '0.55rem', color: 'var(--wks-text-disabled)' }}>
        Reopens your agents and tabs automatically. Off shows the session picker at startup.
        Switch sessions any time from the command palette (Ctrl+K → Switch session).
      </div>
    </Section>
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

function ChromeCookieSyncRow() {
  const [syncing, setSyncing] = useState(false);
  const [lastResult, setLastResult] = useState<string | null>(null);
  const [restrictDomains, setRestrictDomains] = useState(true);
  const [browser, setBrowser] = useState<'chrome' | 'edge'>('chrome');

  // Hosts we'll import when "restrict" is checked. Anything you visit in
  // Chrome that's not in this list stays in Chrome.
  const defaultDomains = [
    'atlassian.com',
    'atlassian.net',
    'microsoftonline.com',
    'microsoft.com',
    'live.com',
    'office.com',
    'office365.com',
    'google.com',
    'github.com',
  ];

  const onSync = useCallback(async () => {
    setSyncing(true);
    setLastResult(null);
    try {
      const res = await window.electronAPI.importChromeCookies(
        restrictDomains ? defaultDomains : undefined,
        'cdp',
        browser,
      );
      const diag = (res as any).diagnostics ?? {};
      const diagStr = Object.keys(diag).length
        ? ' — ' + Object.entries(diag).map(([k, v]) => `${k}=${v}`).join(', ')
        : '';
      const msg = `Imported ${res.imported}, skipped ${res.skipped}` + diagStr +
        (res.errors.length ? `\nFirst error: ${res.errors[0]}` : '');
      setLastResult(msg);
    } catch (err: any) {
      setLastResult(`Failed: ${err?.message ?? err}`);
    } finally {
      setSyncing(false);
    }
  }, [restrictDomains, browser]);

  return (
    <>
      <Row label="Source browser">
        <select
          value={browser}
          onChange={(e) => setBrowser(e.target.value as 'chrome' | 'edge')}
          style={{
            height: '24px',
            padding: '0 8px',
            fontSize: '0.65rem',
            backgroundColor: 'var(--wks-bg-input)',
            color: 'var(--wks-text-secondary)',
            border: '1px solid var(--wks-border)',
            borderRadius: '3px',
            outline: 'none',
            fontFamily: 'inherit',
          }}
        >
          <option value="chrome">Google Chrome</option>
          <option value="edge">Microsoft Edge</option>
        </select>
      </Row>
      <Row label={`Sync cookies from ${browser === 'edge' ? 'Edge' : 'Chrome'}`}>
        <button
          onClick={onSync}
          disabled={syncing}
          style={{
            height: '24px',
            padding: '0 12px',
            fontSize: '0.65rem',
            fontWeight: 600,
            backgroundColor: syncing ? 'var(--wks-bg-input)' : 'var(--wks-accent-bg)',
            color: syncing ? 'var(--wks-text-disabled)' : 'var(--wks-accent-text)',
            border: '1px solid var(--wks-border)',
            borderRadius: '3px',
            cursor: syncing ? 'default' : 'pointer',
            fontFamily: 'inherit',
          }}
        >
          {syncing ? 'Syncing…' : 'Sync now'}
        </button>
      </Row>
      <Row label="Restrict to login-related domains">
        <input
          type="checkbox"
          checked={restrictDomains}
          onChange={(e) => setRestrictDomains(e.target.checked)}
        />
      </Row>
      <div style={{ fontSize: '0.55rem', color: 'var(--wks-text-disabled)' }}>
        Reads Chrome's local cookie store and copies into Workspacer's browser session — useful when OAuth (e.g. Microsoft sign-in) won't complete inside an embedded webview. Run while Chrome is closed for best results.
        {lastResult && (
          <div style={{ marginTop: 4, color: 'var(--wks-text-secondary)' }}>{lastResult}</div>
        )}
      </div>
    </>
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

function CheckRow({ label, checked, onChange, disabled }: { label: string; checked: boolean; onChange: (v: boolean) => void; disabled?: boolean }) {
  return (
    <label
      style={{
        display: 'flex', alignItems: 'center', justifyContent: 'space-between',
        cursor: disabled ? 'default' : 'pointer', opacity: disabled ? 0.45 : 1,
      }}
    >
      <span style={{ fontSize: '0.7rem', color: 'var(--wks-text-muted)' }}>{label}</span>
      <input
        type="checkbox"
        checked={checked}
        disabled={disabled}
        onChange={(e) => onChange(e.target.checked)}
        style={{ accentColor: 'var(--wks-accent)', cursor: disabled ? 'default' : 'pointer' }}
      />
    </label>
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
