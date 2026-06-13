import React, { useState, useCallback, useRef } from 'react';
import { Config } from '../../hooks/useConfig';
import { Section, Row, ModeButton } from './primitives';

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
  'open-file': 'Open File (editor)',
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
  'toggle-inspector': 'Toggle Inspector (Claude pane)',
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
      'command-palette': 'ctrl+k', 'open-file': 'ctrl+e', 'settings': 'ctrl+,', 'save-session': 'ctrl+s',
      'rename-tab': 'f2', 'toggle-help': 'ctrl+?', 'prev-tab': 'ctrl+[',
      'next-tab': 'ctrl+]', 'nav-left': 'ctrl+h', 'nav-right': 'ctrl+l',
      'nav-up': 'ctrl+shift+k', 'nav-down': 'ctrl+shift+j',
      'prev-agent': 'ctrl+alt+arrowup', 'next-agent': 'ctrl+alt+arrowdown',
      'spawn-agent': 'ctrl+alt+n', 'toggle-inspector': 'ctrl+shift+e',
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

interface KeybindingsSectionProps {
  config: Config;
  save: (partial: Partial<Config>) => Promise<Config>;
}

const KeybindingsSection: React.FC<KeybindingsSectionProps> = ({ config, save }) => {
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

  const shortcuts = mode === 'vim' ? vimShortcuts(leader) : defaultShortcuts;

  return (
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
  );
};

export default KeybindingsSection;
