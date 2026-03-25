import React, { useState, useCallback, useRef } from 'react';
import { useConfig } from '../hooks/useConfig';

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
  const l = leader || 'Ctrl+Space';
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
  const kbConfig = config.keybindings ?? { mode: 'default' as const, leader: 'ctrl+space' };

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
    // Skip if only a modifier was pressed
    if (['control', 'alt', 'shift', 'meta'].includes(key)) return;

    parts.push(key);
    const combo = parts.join('+');

    setLeader(combo);
    setCapturingLeader(false);
    save({ keybindings: { mode, leader: combo } });
  }, [mode, save]);

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
    </div>
  );
};

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
