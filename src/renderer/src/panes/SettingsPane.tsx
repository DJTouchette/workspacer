import React, { useState, useCallback, useRef } from 'react';
import { useConfig, AppEntry } from '../hooks/useConfig';

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
        backgroundColor: 'rgb(24, 24, 27)',
        color: 'rgb(200, 200, 210)',
        fontFamily: 'Inter, system-ui, sans-serif',
        fontSize: '12px',
        overflow: 'auto',
        padding: '16px 24px',
      }}
    >
      <h2 style={{ fontSize: '0.9rem', fontWeight: 600, margin: '0 0 16px 0', color: 'rgb(220, 220, 235)' }}>
        Settings
      </h2>

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
                  backgroundColor: capturingLeader ? 'rgb(40, 50, 70)' : 'rgb(18, 18, 20)',
                  color: capturingLeader ? 'rgb(147, 197, 253)' : 'rgb(200, 200, 210)',
                  border: capturingLeader ? '1px solid rgb(80, 120, 200)' : '1px solid rgb(50, 50, 55)',
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
          <div style={{ fontSize: '0.65rem', color: 'rgb(120, 120, 135)', textTransform: 'uppercase', letterSpacing: '0.05em', marginBottom: '6px' }}>
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
                      <td colSpan={2} style={{ padding: '4px 0 2px', color: 'rgb(120, 120, 135)', fontSize: '0.6rem', fontWeight: 600 }}>
                        {key}
                      </td>
                    </tr>
                  );
                }
                return (
                  <tr key={i}>
                    <td style={{ padding: '2px 16px 2px 0', color: 'rgb(170, 170, 185)', fontFamily: 'monospace', fontSize: '0.65rem', whiteSpace: 'nowrap' }}>
                      {key}
                    </td>
                    <td style={{ padding: '2px 0', color: 'rgb(140, 140, 155)' }}>
                      {desc}
                    </td>
                  </tr>
                );
              })}
            </tbody>
          </table>
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
                  backgroundColor: 'rgb(30, 30, 35)',
                  borderRadius: '4px',
                  border: '1px solid rgb(60, 60, 70)',
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
                  onMouseEnter={(e) => { (e.currentTarget as HTMLElement).style.backgroundColor = 'rgb(32, 32, 38)'; }}
                  onMouseLeave={(e) => { (e.currentTarget as HTMLElement).style.backgroundColor = 'transparent'; }}
                >
                  <span style={{ fontSize: '0.85rem', width: '20px', textAlign: 'center' }}>
                    {app.icon || '\u{1F310}'}
                  </span>
                  <div style={{ flex: 1, minWidth: 0 }}>
                    <div style={{ fontSize: '0.7rem', color: 'rgb(200, 200, 210)', fontWeight: 500 }}>{app.name}</div>
                    <div style={{ fontSize: '0.6rem', color: 'rgb(100, 100, 115)', overflow: 'hidden', textOverflow: 'ellipsis', whiteSpace: 'nowrap' }}>{app.url}</div>
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
              backgroundColor: 'rgb(35, 35, 40)',
              color: 'rgb(160, 160, 175)',
              border: '1px dashed rgb(55, 55, 65)',
              borderRadius: '4px',
              cursor: 'pointer',
              height: 'auto',
              lineHeight: '1.4',
              margin: '4px 0 0',
              width: '100%',
            }}
            onMouseEnter={(e) => { (e.currentTarget as HTMLElement).style.borderColor = 'rgb(80, 120, 200)'; (e.currentTarget as HTMLElement).style.color = 'rgb(200, 200, 210)'; }}
            onMouseLeave={(e) => { (e.currentTarget as HTMLElement).style.borderColor = 'rgb(55, 55, 65)'; (e.currentTarget as HTMLElement).style.color = 'rgb(160, 160, 175)'; }}
          >
            + Add App
          </button>
        </div>
      </Section>
    </div>
  );
};

const inputStyle: React.CSSProperties = {
  height: '24px',
  padding: '0 8px',
  fontSize: '0.65rem',
  backgroundColor: 'rgb(18, 18, 20)',
  color: 'rgb(200, 200, 210)',
  border: '1px solid rgb(50, 50, 55)',
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
        backgroundColor: primary ? 'rgb(80, 120, 200)' : 'transparent',
        color: danger ? 'rgb(248, 113, 113)' : primary ? '#fff' : 'rgb(140, 140, 155)',
        border: primary ? '1px solid rgb(80, 120, 200)' : '1px solid rgb(50, 50, 55)',
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
        color: 'rgb(180, 180, 195)',
        marginBottom: '8px',
        paddingBottom: '4px',
        borderBottom: '1px solid rgb(40, 40, 45)',
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
      <span style={{ fontSize: '0.7rem', color: 'rgb(160, 160, 175)' }}>{label}</span>
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
        backgroundColor: active ? 'rgb(80, 120, 200)' : 'rgb(35, 35, 40)',
        color: active ? '#fff' : 'rgb(160, 160, 175)',
        border: active ? '1px solid rgb(80, 120, 200)' : '1px solid rgb(50, 50, 55)',
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
