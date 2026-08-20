import React, { useState, useCallback, useEffect, useMemo, useRef } from 'react';
import { RotateCcw } from 'lucide-react';
import { Config, DEFAULT_CONFIG } from '../../hooks/useConfig';
import {
  findBindingConflicts,
  formatBinding,
  formatCombo,
  LAYER_ACTIONS,
  ACTION_SECTIONS,
  DIGIT_RANGE_ACTIONS,
  DIGIT_RANGE_TOKEN,
  resolveLeader,
} from '../../lib/shortcuts';
import {
  KEYBINDING_PRESETS,
  PRESET_ORDER,
  presetConfigPatch,
  isPresetId,
} from '../../lib/keybindingPresets';
import { Section, Row, ModeButton } from './primitives';

// ── Shortcut Editor ──

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

const ShortcutEditor: React.FC<{
  config: Config;
  save: (partial: Partial<Config>) => Promise<Config>;
}> = ({ config, save }) => {
  // Defaults merged under the saved map so actions added after the user's
  // config was written (e.g. the fleet/inbox bindings) still show their
  // default combo instead of "—".
  const currentShortcuts = {
    ...(DEFAULT_CONFIG.keybindings.shortcuts ?? {}),
    ...(config.keybindings?.shortcuts ?? {}),
  };
  const prefix = resolveLeader(config.keybindings?.prefix ?? 'ctrl+space');
  // Live conflict check over the SAME merged map the dispatcher walks (layer
  // verbs included only while the layer is enabled — mirroring App). Warnings,
  // not blocks: the dispatcher degrades, the user just deserves to know.
  const bindingWarnings = useMemo(() => {
    const merged: Record<string, string> = { ...currentShortcuts };
    if (config.keybindings?.commandLayer?.enabled !== true) {
      for (const a of LAYER_ACTIONS) delete merged[a];
    }
    return findBindingConflicts(merged);
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [config.keybindings]);
  const [capturing, setCapturing] = useState<string | null>(null);
  // True once the prefix has been pressed mid-capture: the following keys
  // become the chord (stored as "prefix <step> [<step>…]").
  const [chordPending, setChordPending] = useState(false);
  // Multi-step chord capture: steps accumulate ('prefix g g', shifted steps
  // like 'prefix shift+k' included) and COMMIT after a beat of silence — the
  // only rule that works for paths the current tree doesn't know yet.
  const [chordSteps, setChordSteps] = useState<string[]>([]);
  const chordCommitRef = useRef<ReturnType<typeof setTimeout> | null>(null);

  const handleCapture = useCallback(
    (action: string, e: React.KeyboardEvent) => {
      e.preventDefault();
      e.stopPropagation();

      // Wait through bare modifier presses.
      if (['Control', 'Alt', 'Shift', 'Meta'].includes(e.key)) return;

      const combo = comboFromEvent(e);

      // Digit-range actions (jump-tab/move-tab) bind to a modifier + any of 1–9.
      // Capture the modifiers off the pressed digit, then normalize to "…+1-9".
      if (DIGIT_RANGE_ACTIONS.has(action)) {
        const m = /(\d)$/.exec(combo);
        if (!m) return; // wait until a digit is pressed
        const value = combo.slice(0, m.index) + DIGIT_RANGE_TOKEN;
        save({
          keybindings: {
            ...config.keybindings,
            shortcuts: { ...currentShortcuts, [action]: value },
          },
        });
        setCapturing(null);
        setChordPending(false);
        return;
      }

      // First press matches the prefix → arm chord capture (don't save yet).
      if (!chordPending && combo === prefix.toLowerCase().trim()) {
        setChordPending(true);
        return;
      }

      if (chordPending) {
        // Accumulate steps; a 750ms pause commits the whole path. This is what
        // lets the editor express 'prefix g g' and shifted steps — committing
        // on the first key could never record a multi-step chord.
        const steps = [...chordSteps, combo];
        setChordSteps(steps);
        if (chordCommitRef.current) clearTimeout(chordCommitRef.current);
        chordCommitRef.current = setTimeout(() => {
          save({
            keybindings: {
              ...config.keybindings,
              shortcuts: { ...currentShortcuts, [action]: `prefix ${steps.join(' ')}` },
            },
          });
          setCapturing(null);
          setChordPending(false);
          setChordSteps([]);
        }, 750);
        return;
      }

      save({
        keybindings: { ...config.keybindings, shortcuts: { ...currentShortcuts, [action]: combo } },
      });
      setCapturing(null);
      setChordPending(false);
      setChordSteps([]);
    },
    [config.keybindings, currentShortcuts, prefix, chordPending, chordSteps, save],
  );

  const handleReset = useCallback(
    (action: string) => {
      const defaults = DEFAULT_CONFIG.keybindings.shortcuts ?? {};
      const updated = { ...currentShortcuts, [action]: defaults[action] ?? '' };
      save({ keybindings: { ...config.keybindings, shortcuts: updated } });
    },
    [config.keybindings, currentShortcuts, save],
  );

  const startCapture = (action: string) => {
    if (chordCommitRef.current) clearTimeout(chordCommitRef.current);
    setCapturing(action);
    setChordPending(false);
    setChordSteps([]);
  };
  const stopCapture = () => {
    // Cancel any pending multi-step commit — a blur mid-sequence must not
    // save half a chord 750ms later.
    if (chordCommitRef.current) clearTimeout(chordCommitRef.current);
    setCapturing(null);
    setChordPending(false);
    setChordSteps([]);
  };

  return (
    <div style={{ marginTop: '12px' }}>
      <div
        style={{
          fontSize: '0.68rem',
          color: 'var(--wks-text-faint)',
          textTransform: 'uppercase',
          letterSpacing: '0.05em',
          marginBottom: '4px',
        }}
      >
        Shortcuts (click to rebind)
      </div>
      <div style={{ fontSize: '0.72rem', color: 'var(--wks-text-disabled)', marginBottom: '8px' }}>
        Press a key combo for a direct binding, or press the prefix first then a key for a chord.
        Chords may be multi-step (prefix, then several keys — a pause commits).
      </div>
      {bindingWarnings.length > 0 && (
        <div
          role="alert"
          style={{
            marginBottom: '8px',
            padding: '6px 9px',
            borderRadius: 'var(--wks-radius-md)',
            border: '1px solid color-mix(in srgb, var(--wks-warning) 45%, transparent)',
            backgroundColor: 'color-mix(in srgb, var(--wks-warning) 10%, transparent)',
            fontSize: '0.7rem',
            color: 'var(--wks-warning)',
            display: 'flex',
            flexDirection: 'column',
            gap: '3px',
          }}
        >
          {bindingWarnings.map((w, i) => (
            <div key={i}>⚠ {w}</div>
          ))}
        </div>
      )}
      <div style={{ display: 'flex', flexDirection: 'column', gap: '2px' }}>
        {ACTION_SECTIONS.flatMap((section) => [
          <div
            key={`hdr-${section.section}`}
            style={{
              fontSize: '0.62rem',
              fontWeight: 600,
              color: 'var(--wks-text-faint)',
              textTransform: 'uppercase',
              letterSpacing: '0.04em',
              padding: '8px 0 2px',
            }}
          >
            {section.section}
          </div>,
          ...section.items.map(({ action, label }) => {
            const isCapturing = capturing === action;
            const combo = currentShortcuts[action] ?? '';
            const display = isCapturing
              ? DIGIT_RANGE_ACTIONS.has(action)
                ? 'Press modifier + digit…'
                : chordPending
                  ? chordSteps.length
                    ? `${formatBinding(prefix)} ${chordSteps.map((s) => formatCombo(s)).join(' ')} …`
                    : `${formatBinding(prefix)} then…`
                  : 'Press keys…'
              : combo
                ? formatBinding(combo, prefix)
                : '—';
            return (
              <div
                key={action}
                style={{
                  display: 'flex',
                  alignItems: 'center',
                  justifyContent: 'space-between',
                  padding: '4px 8px',
                  borderRadius: '4px',
                  backgroundColor: isCapturing ? 'var(--wks-bg-selected)' : 'transparent',
                }}
              >
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
                      width: '150px',
                      height: '22px',
                      padding: '0 6px',
                      fontSize: '0.68rem',
                      fontFamily: 'var(--wks-font-mono)',
                      textAlign: 'center',
                      backgroundColor: isCapturing ? 'var(--wks-bg-input)' : 'transparent',
                      color: isCapturing ? 'var(--wks-accent-text)' : 'var(--wks-text-tertiary)',
                      border: isCapturing
                        ? '1px solid var(--wks-accent)'
                        : '1px solid var(--wks-border)',
                      borderRadius: '3px',
                      outline: 'none',
                      cursor: 'pointer',
                    }}
                  />
                  <button
                    onClick={() => handleReset(action)}
                    style={{
                      display: 'inline-flex',
                      alignItems: 'center',
                      padding: '2px 6px',
                      borderRadius: '3px',
                      border: '1px solid var(--wks-border)',
                      backgroundColor: 'transparent',
                      color: 'var(--wks-text-faint)',
                      cursor: 'pointer',
                    }}
                    title="Reset to default"
                  >
                    <RotateCcw size={11} strokeWidth={2} />
                  </button>
                </div>
              </div>
            );
          }),
        ])}
      </div>
    </div>
  );
};

interface KeybindingsSectionProps {
  config: Config;
  save: (partial: Partial<Config>) => Promise<Config>;
}

/** Modifier-only prefix shown while the combo is still being held, e.g. "Ctrl+…". */
function pendingModifiers(e: KeyboardEvent): string {
  const parts: string[] = [];
  if (e.ctrlKey) parts.push('ctrl');
  if (e.altKey) parts.push('alt');
  if (e.shiftKey) parts.push('shift');
  if (e.metaKey) parts.push('meta');
  return parts.join('+');
}

const KeybindingsSection: React.FC<KeybindingsSectionProps> = ({ config, save }) => {
  const prefix = resolveLeader(config.keybindings?.prefix ?? 'ctrl+space');
  const chordHints = config.keybindings?.chordHints ?? true;
  const commandLayerEnabled = config.keybindings?.commandLayer?.enabled === true;
  const [capturingPrefix, setCapturingPrefix] = useState(false);
  // While held-down modifiers are being typed, show them so the combo forming is
  // visible (e.g. "Ctrl+…"); '' means nothing pressed yet.
  const [pendingPrefix, setPendingPrefix] = useState('');
  const prefixInputRef = useRef<HTMLInputElement>(null);

  // Capture the leader via a document-level keydown listener rather than an
  // input's focus. The old focus-then-setTimeout dance raced with the input's
  // blur and usually cancelled itself before a key ever landed. A single
  // un-modified key (bare "`" / space) is a valid leader here.
  useEffect(() => {
    if (!capturingPrefix) return;
    // Keep the field focused so the global nav handler (useKeyboardNav) sees the
    // data-leader-capture target and bails instead of consuming the combo before
    // our listener runs. Focus is synchronous here — no setTimeout race.
    prefixInputRef.current?.focus();

    const onKeyDown = (e: KeyboardEvent) => {
      e.preventDefault();
      e.stopPropagation();
      if (e.key === 'Escape') {
        setCapturingPrefix(false);
        setPendingPrefix('');
        return;
      }
      // Bare modifier: reflect it in the pending display and keep waiting.
      if (['Control', 'Alt', 'Shift', 'Meta'].includes(e.key)) {
        setPendingPrefix(pendingModifiers(e));
        return;
      }
      const combo = comboFromEvent(e as unknown as React.KeyboardEvent);
      setCapturingPrefix(false);
      setPendingPrefix('');
      save({ keybindings: { ...config.keybindings, prefix: combo } });
    };
    // A click anywhere but the capture field itself cancels.
    const onMouseDown = (e: MouseEvent) => {
      if (e.target === prefixInputRef.current) return;
      setCapturingPrefix(false);
      setPendingPrefix('');
    };

    document.addEventListener('keydown', onKeyDown, true);
    document.addEventListener('mousedown', onMouseDown, true);
    return () => {
      document.removeEventListener('keydown', onKeyDown, true);
      document.removeEventListener('mousedown', onMouseDown, true);
    };
  }, [capturingPrefix, config.keybindings, save]);

  const prefixDisplay = capturingPrefix
    ? pendingPrefix
      ? `${formatBinding(pendingPrefix)}+…`
      : 'Press key combo…'
    : formatBinding(prefix);

  const activePreset = isPresetId(config.keybindings?.presetId)
    ? config.keybindings.presetId
    : undefined;

  return (
    <Section title="Keybindings">
      <Row label="Preset">
        <div style={{ display: 'flex', gap: 4, flexWrap: 'wrap' }}>
          {PRESET_ORDER.map((id) => (
            <ModeButton
              key={id}
              label={KEYBINDING_PRESETS[id].label}
              active={activePreset === id}
              onClick={() => save(presetConfigPatch(id, config))}
            />
          ))}
        </div>
      </Row>
      <div style={{ fontSize: '0.72rem', color: 'var(--wks-text-disabled)' }}>
        {activePreset ? KEYBINDING_PRESETS[activePreset].description : 'Custom bindings.'} Switching
        a preset keeps any keys you&rsquo;ve personally rebound.
        {activePreset && (
          <>
            {' '}
            <button
              onClick={() => save(presetConfigPatch(activePreset, config, true))}
              style={{
                fontSize: '0.7rem',
                padding: '1px 6px',
                borderRadius: '3px',
                border: '1px solid var(--wks-border)',
                background: 'transparent',
                color: 'var(--wks-text-faint)',
                cursor: 'pointer',
              }}
              title={`Discard every override and restore the ${KEYBINDING_PRESETS[activePreset].label} defaults`}
            >
              Reset all to {KEYBINDING_PRESETS[activePreset].label}
            </button>
          </>
        )}
      </div>

      <Row label="Prefix">
        <input
          ref={prefixInputRef}
          data-leader-capture="true"
          readOnly
          value={prefixDisplay}
          onMouseDown={() => {
            setPendingPrefix('');
            setCapturingPrefix(true);
          }}
          style={{
            width: '160px',
            height: '24px',
            padding: '0 8px',
            fontSize: '0.7rem',
            fontFamily: 'var(--wks-font-mono)',
            backgroundColor: capturingPrefix ? 'var(--wks-bg-selected)' : 'var(--wks-bg-input)',
            color: capturingPrefix ? 'var(--wks-accent-text)' : 'var(--wks-text-secondary)',
            border: capturingPrefix ? '1px solid var(--wks-accent)' : '1px solid var(--wks-border)',
            borderRadius: '3px',
            outline: 'none',
            cursor: 'pointer',
          }}
        />
      </Row>
      <div style={{ fontSize: '0.72rem', color: 'var(--wks-text-disabled)' }}>
        Structural commands (new tab, split, navigate panes) fire as{' '}
        <strong>{formatBinding(prefix)}</strong> then a key. Direct combos are reserved for
        terminal-safe keys so a focused terminal keeps Ctrl+C / Ctrl+L / etc.
      </div>

      <Row label="Chord hints">
        <div style={{ display: 'flex', gap: 4 }}>
          <ModeButton
            label="On"
            active={chordHints}
            onClick={() => save({ keybindings: { ...config.keybindings, chordHints: true } })}
          />
          <ModeButton
            label="Off"
            active={!chordHints}
            onClick={() => save({ keybindings: { ...config.keybindings, chordHints: false } })}
          />
        </div>
      </Row>
      <div style={{ fontSize: '0.72rem', color: 'var(--wks-text-disabled)' }}>
        When the prefix is pressed, show a cheatsheet of the available chord keys in the bottom
        corner. Off keeps just the minimal prefix indicator.
      </div>

      <Row label="Command layer">
        <div style={{ display: 'flex', gap: 4 }}>
          <ModeButton
            label="On"
            active={commandLayerEnabled}
            onClick={() =>
              save({
                keybindings: {
                  ...config.keybindings,
                  commandLayer: { ...config.keybindings?.commandLayer, enabled: true },
                },
              })
            }
          />
          <ModeButton
            label="Off"
            active={!commandLayerEnabled}
            onClick={() =>
              save({
                keybindings: {
                  ...config.keybindings,
                  commandLayer: { ...config.keybindings?.commandLayer, enabled: false },
                },
              })
            }
          />
        </div>
      </Row>
      <div style={{ fontSize: '0.72rem', color: 'var(--wks-text-disabled)' }}>
        tmux-style keyboard mode: the prefix arms a transient key layer — pane zoom (z), swap
        ({'{'} {'}'}), harpoon pins (m, 1–9), chat paging (Shift+K/J), approve/deny (y/n), and
        repeat groups so prefix h h l walks panes. Chords stay armed until resolved; Esc, a
        click, or an unknown key stands down. Prefix twice sends the literal prefix to the
        focused terminal (nested tmux).
      </div>

      <ShortcutEditor config={config} save={save} />
    </Section>
  );
};

export default KeybindingsSection;
