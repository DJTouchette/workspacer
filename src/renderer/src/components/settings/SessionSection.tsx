import React from 'react';
import { Config } from '../../hooks/useConfig';
import { Section, CheckRow } from './primitives';

interface SessionSectionProps {
  config: Config;
  save: (partial: Partial<Config>) => Promise<Config>;
}

const SessionSection: React.FC<SessionSectionProps> = ({ config, save }) => {
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

export default SessionSection;
