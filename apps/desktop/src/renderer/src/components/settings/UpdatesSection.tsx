import React from 'react';
import { Config } from '../../hooks/useConfig';
import { useAppVersion } from '../../hooks/useAppVersion';
import { Section, CheckRow } from './primitives';

const UPDATES_DEFAULTS = { enabled: true, channel: 'latest' };

interface UpdatesSectionProps {
  config: Config;
  save: (partial: Partial<Config>) => Promise<Config>;
}

const UpdatesSection: React.FC<UpdatesSectionProps> = ({ config, save }) => {
  const { version, isNightly } = useAppVersion();
  const updates = config.updates ?? UPDATES_DEFAULTS;
  const set = (patch: Partial<typeof updates>) =>
    save({ updates: { ...UPDATES_DEFAULTS, ...updates, ...patch } });

  return (
    <Section title="Updates">
      {version && (
        <div
          style={{
            fontSize: '0.72rem',
            color: 'var(--wks-text-secondary)',
            fontFamily: 'var(--wks-font-mono)',
          }}
        >
          Workspacer v{version}
          {isNightly && <span style={{ color: 'var(--wks-warning)' }}> (nightly build)</span>}
        </div>
      )}
      <CheckRow
        label="Automatically check for and install updates"
        checked={updates.enabled !== false}
        onChange={(v) => set({ enabled: v })}
      />
      <div style={{ fontSize: '0.72rem', color: 'var(--wks-text-disabled)' }}>
        Checks the GitHub release feed on launch and every few hours, downloads a newer build in the
        background, and asks before restarting to install. Only active in the packaged app.
      </div>
    </Section>
  );
};

export default UpdatesSection;
