import React from 'react';
import { Config } from '../../hooks/useConfig';
import { useAppVersion } from '../../hooks/useAppVersion';
import { Section, CheckRow } from './primitives';
import { ReleaseNotes } from '../ReleaseNotes';

const UPDATES_DEFAULTS = { enabled: true, channel: 'latest' };

/**
 * The changelog version a running build corresponds to. A nightly is stamped
 * `X.Y.Z-nightly.<stamp>` and its notes are whatever `X.Y.Z` says plus whatever
 * has landed since — the changelog cannot have a row per nightly, so the base
 * version is the honest match rather than no match at all.
 */
function releaseVersion(v: string): string {
  return v.split('-')[0];
}

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
        {isNightly &&
          ' Nightly builds update from the rolling nightly prerelease, never stable — to return to stable, reinstall a release build from the website.'}
      </div>

      {/* The notes live beside the version they describe, which is the one
          question anybody opening this section actually has. Same markdown the
          GitHub release body is cut from — see lib/changelog.generated.ts. */}
      <div style={{ marginTop: 6, display: 'flex', flexDirection: 'column', gap: 8 }}>
        <div style={{ fontSize: '0.72rem', color: 'var(--wks-text-secondary)' }}>Release notes</div>
        <ReleaseNotes highlightVersion={releaseVersion(version)} />
      </div>
    </Section>
  );
};

export default UpdatesSection;
