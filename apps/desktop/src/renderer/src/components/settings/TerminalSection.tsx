import React from 'react';
import { Config } from '../../hooks/useConfig';
import { Section, Row, SearchableSelect, SelectOption } from './primitives';

interface TerminalSectionProps {
  config: Config;
  save: (partial: Partial<Config>) => Promise<Config>;
}

const TerminalSection: React.FC<TerminalSectionProps> = ({ config, save }) => {
  const termCfg = config.terminal;
  const shells = termCfg?.shells ?? [];
  const current = termCfg?.shell ?? '';

  // The empty string means "let the OS/backend pick" — same convention the
  // TerminalPane uses when no per-tab shell is given (shell || termCfg.shell).
  const options: SelectOption[] = [
    { value: '', label: 'System default' },
    ...shells.map((s) => ({ value: s.path, label: s.label })),
  ];

  return (
    <Section title="Terminal">
      <Row label="Default shell">
        <SearchableSelect
          value={current}
          options={options}
          onChange={(v) => save({ terminal: { ...termCfg, shell: v } })}
          placeholder="System default"
        />
      </Row>
      <div style={{ fontSize: '0.55rem', color: 'var(--wks-text-disabled)' }}>
        New terminals (the <strong>+</strong> button, <strong>Ctrl+Space&nbsp;N</strong>) open with this shell.
        <strong> System default</strong> uses your OS default. Pick a specific shell from the menu next to <strong>+</strong> to
        override per-tab.
      </div>
    </Section>
  );
};

export default TerminalSection;
