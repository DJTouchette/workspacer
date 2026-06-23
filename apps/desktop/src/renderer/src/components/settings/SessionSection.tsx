import React from 'react';
import { Config } from '../../hooks/useConfig';
import { Section, CheckRow, Row, ModeButton } from './primitives';

interface SessionSectionProps {
  config: Config;
  save: (partial: Partial<Config>) => Promise<Config>;
}

const FONT_SCALES: { label: string; value: number }[] = [
  { label: 'Small', value: 1.0 },
  { label: 'Default', value: 1.15 },
  { label: 'Large', value: 1.3 },
  { label: 'XL', value: 1.5 },
];

const SessionSection: React.FC<SessionSectionProps> = ({ config, save }) => {
  const autoResume = config.session?.autoResume ?? true;
  const defaultView = config.claude?.defaultView ?? 'terminal';
  const guiFontScale = config.ui.guiFontScale ?? 1.15;
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

      <Row label="Default Claude view">
        <div style={{ display: 'flex', gap: 4 }}>
          <ModeButton
            label="GUI"
            active={defaultView === 'gui'}
            onClick={() => save({ claude: { ...config.claude, defaultView: 'gui' } })}
          />
          <ModeButton
            label="Terminal"
            active={defaultView === 'terminal'}
            onClick={() => save({ claude: { ...config.claude, defaultView: 'terminal' } })}
          />
        </div>
      </Row>
      <div style={{ fontSize: '0.55rem', color: 'var(--wks-text-disabled)' }}>
        Which view a Claude pane opens in. The rich GUI shows the conversation, work cards,
        and inspector; Terminal is the raw Claude Code TUI. Toggle any time from the pane's top bar.
      </div>

      <Row label="Chat text size">
        <div style={{ display: 'flex', gap: 4 }}>
          {FONT_SCALES.map((s) => (
            <ModeButton
              key={s.value}
              label={s.label}
              active={guiFontScale === s.value}
              onClick={() => save({ ui: { ...config.ui, guiFontScale: s.value } })}
            />
          ))}
        </div>
      </Row>
      <div style={{ fontSize: '0.55rem', color: 'var(--wks-text-disabled)' }}>
        Size of the conversation text in the GUI view (messages, markdown, code blocks).
        Doesn't affect the terminal view.
      </div>

      <CheckRow
        label="Show the composer send button"
        checked={config.ui.showComposerSend !== false}
        onChange={(v) => save({ ui: { ...config.ui, showComposerSend: v } })}
      />
      <div style={{ fontSize: '0.55rem', color: 'var(--wks-text-disabled)' }}>
        The ↑ button next to the message box. Off keeps the box clean — Enter still sends
        (Shift+Enter for a newline).
      </div>
    </Section>
  );
};

export default SessionSection;
