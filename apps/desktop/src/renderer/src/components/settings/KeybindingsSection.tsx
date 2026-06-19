import React, { useState, useCallback, useRef } from 'react';
import { Config, DEFAULT_CONFIG } from '../../hooks/useConfig';
import { formatBinding } from '../../lib/shortcuts';
import { Section, Row, ModeButton } from './primitives';

// ── Shortcut Editor ──

const SHORTCUT_LABELS: Record<string, string> = {
  // Agents
  'prev-agent': 'Previous Agent',
  'next-agent': 'Next Agent',
  'next-attention': 'Jump to Agent Needing You',
  'spawn-agent': 'Spawn Agent',
  // Navigation
  'prev-tab': 'Previous Tab',
  'next-tab': 'Next Tab',
  'move-tab-left': 'Move Tab Left',
  'move-tab-right': 'Move Tab Right',
  'cycle-view': 'Cycle View Mode',
  'nav-left': 'Focus Pane Left',
  'nav-right': 'Focus Pane Right',
  'nav-up': 'Focus Pane Up',
  'nav-down': 'Focus Pane Down',
  // Tabs & panes
  'new-terminal': 'New Terminal',
  'new-browser': 'New Browser',
  'new-claude': 'New Claude',
  'split': 'Split Pane',
  'quick-split': 'Quick Split (clone)',
  'close-pane': 'Close Pane',
  'open-file': 'Open File (editor)',
  'rename-tab': 'Rename Tab',
  // Panels & tools
  'toggle-sidebar': 'Toggle Sidebar',
  'toggle-terminal': 'Toggle Terminal Panel',
  'toggle-inbox': 'Toggle Inbox',
  'toggle-fleet': 'Toggle Fleet Deck',
  'toggle-inspector': 'Toggle Inspector (Claude pane)',
  'library-picker': 'Library Picker',
  'command-palette': 'Command Palette',
  'save-session': 'Save Session',
  'settings': 'Settings',
  'toggle-help': 'Toggle Help',
};

/** Build a combo string from a keyboard event (e.g. "ctrl+shift+p"). */
function comboFromEvent(e: React.KeyboardEvent): string {
  const parts: string[] = [];
  if (e.ctrlKey) parts.push('ctrl');
  if (e.altKey) parts.push('alt');
  if (e.shiftKey) parts.push('shift');
  if (e.metaKey) parts.push('meta');
  parts.push(e.key === ' ' ? 'space' : e.key.toLowerCase());
  return parts.join('+');
}

const ShortcutEditor: React.FC<{ config: Config; save: (partial: Partial<Config>) => Promise<Config> }> = ({ config, save }) => {
  const currentShortcuts = config.keybindings?.shortcuts ?? {};
  const prefix = config.keybindings?.prefix ?? 'ctrl+space';
  const [capturing, setCapturing] = useState<string | null>(null);
  // True once the prefix has been pressed mid-capture: the next key becomes the
  // chord (stored as "prefix <combo>").
  const [chordPending, setChordPending] = useState(false);

  const handleCapture = useCallback((action: string, e: React.KeyboardEvent) => {
    e.preventDefault();
    e.stopPropagation();

    // Wait through bare modifier presses.
    if (['Control', 'Alt', 'Shift', 'Meta'].includes(e.key)) return;

    const combo = comboFromEvent(e);

    // First press matches the prefix → arm chord capture (don't save yet).
    if (!chordPending && combo === prefix.toLowerCase().trim()) {
      setChordPending(true);
      return;
    }

    const value = chordPending ? `prefix ${combo}` : combo;
    save({ keybindings: { ...config.keybindings, shortcuts: { ...currentShortcuts, [action]: value } } });
    setCapturing(null);
    setChordPending(false);
  }, [config.keybindings, currentShortcuts, prefix, chordPending, save]);

  const handleReset = useCallback((action: string) => {
    const defaults = DEFAULT_CONFIG.keybindings.shortcuts ?? {};
    const updated = { ...currentShortcuts, [action]: defaults[action] ?? '' };
    save({ keybindings: { ...config.keybindings, shortcuts: updated } });
  }, [config.keybindings, currentShortcuts, save]);

  const startCapture = (action: string) => { setCapturing(action); setChordPending(false); };
  const stopCapture = () => { setCapturing(null); setChordPending(false); };

  return (
    <div style={{ marginTop: '12px' }}>
      <div style={{ fontSize: '0.65rem', color: 'var(--wks-text-faint)', textTransform: 'uppercase', letterSpacing: '0.05em', marginBottom: '4px' }}>
        Shortcuts (click to rebind)
      </div>
      <div style={{ fontSize: '0.58rem', color: 'var(--wks-text-disabled)', marginBottom: '8px' }}>
        Press a key combo for a direct binding, or press the prefix first then a key for a chord.
      </div>
      <div style={{ display: 'flex', flexDirection: 'column', gap: '2px' }}>
        {Object.entries(SHORTCUT_LABELS).map(([action, label]) => {
          const isCapturing = capturing === action;
          const combo = currentShortcuts[action] ?? '';
          const display = isCapturing
            ? (chordPending ? `${formatBinding(prefix)} then…` : 'Press keys…')
            : (combo ? formatBinding(combo, prefix) : '—');
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
                  value={display}
                  onClick={() => startCapture(action)}
                  onKeyDown={isCapturing ? (e) => handleCapture(action, e) : undefined}
                  onBlur={stopCapture}
                  style={{
                    width: '150px', height: '22px', padding: '0 6px',
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
  const prefix = config.keybindings?.prefix ?? 'ctrl+space';
  const chordHints = config.keybindings?.chordHints ?? true;
  const [capturingPrefix, setCapturingPrefix] = useState(false);
  const prefixInputRef = useRef<HTMLInputElement>(null);

  const handlePrefixCapture = useCallback((e: React.KeyboardEvent) => {
    e.preventDefault();
    e.stopPropagation();
    // The prefix must be a real combo, not a bare modifier — a bare-modifier
    // prefix would arm a chord on every modifier tap.
    if (['Control', 'Alt', 'Shift', 'Meta'].includes(e.key)) return;
    const combo = comboFromEvent(e);
    setCapturingPrefix(false);
    save({ keybindings: { ...config.keybindings, prefix: combo } });
  }, [config.keybindings, save]);

  return (
    <Section title="Keybindings">
      <Row label="Prefix">
        <input
          ref={prefixInputRef}
          data-leader-capture="true"
          readOnly
          value={capturingPrefix ? 'Press key combo…' : formatBinding(prefix)}
          onKeyDown={capturingPrefix ? handlePrefixCapture : undefined}
          onBlur={() => setCapturingPrefix(false)}
          onClick={() => {
            setCapturingPrefix(true);
            setTimeout(() => prefixInputRef.current?.focus(), 0);
          }}
          style={{
            width: '160px', height: '24px', padding: '0 8px',
            fontSize: '11px', fontFamily: 'monospace',
            backgroundColor: capturingPrefix ? 'var(--wks-bg-selected)' : 'var(--wks-bg-input)',
            color: capturingPrefix ? 'var(--wks-accent-text)' : 'var(--wks-text-secondary)',
            border: capturingPrefix ? '1px solid var(--wks-accent)' : '1px solid var(--wks-border)',
            borderRadius: '3px', outline: 'none', cursor: 'pointer',
          }}
        />
      </Row>
      <div style={{ fontSize: '0.58rem', color: 'var(--wks-text-disabled)' }}>
        Structural commands (new tab, split, navigate panes) fire as <strong>{formatBinding(prefix)}</strong> then a
        key. Direct combos are reserved for terminal-safe keys so a focused terminal keeps Ctrl+C / Ctrl+L / etc.
      </div>

      <Row label="Chord hints">
        <div style={{ display: 'flex', gap: 4 }}>
          <ModeButton label="On" active={chordHints} onClick={() => save({ keybindings: { ...config.keybindings, chordHints: true } })} />
          <ModeButton label="Off" active={!chordHints} onClick={() => save({ keybindings: { ...config.keybindings, chordHints: false } })} />
        </div>
      </Row>
      <div style={{ fontSize: '0.58rem', color: 'var(--wks-text-disabled)' }}>
        When the prefix is pressed, show a cheatsheet of the available chord keys in the bottom corner. Off keeps just
        the minimal prefix indicator.
      </div>

      <ShortcutEditor config={config} save={save} />
    </Section>
  );
};

export default KeybindingsSection;
