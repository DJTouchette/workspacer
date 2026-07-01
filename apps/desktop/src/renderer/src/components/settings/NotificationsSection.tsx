import React from 'react';
import { Config } from '../../hooks/useConfig';
import { Section, CheckRow } from './primitives';

const NOTIF_DEFAULTS = { enabled: true, notifyDone: true, onlyWhenUnwatched: true, sound: false };

interface NotificationsSectionProps {
  config: Config;
  save: (partial: Partial<Config>) => Promise<Config>;
}

const NotificationsSection: React.FC<NotificationsSectionProps> = ({ config, save }) => {
  const notif = config.notifications ?? NOTIF_DEFAULTS;
  const set = (patch: Partial<typeof notif>) => save({ notifications: { ...notif, ...patch } });

  return (
    <Section title="Notifications">
      <CheckRow
        label="Desktop notifications"
        checked={notif.enabled}
        onChange={(v) => set({ enabled: v })}
      />
      <CheckRow
        label="Notify when an agent finishes"
        checked={notif.notifyDone}
        disabled={!notif.enabled}
        onChange={(v) => set({ notifyDone: v })}
      />
      <CheckRow
        label="Only when I'm not watching that agent"
        checked={notif.onlyWhenUnwatched}
        disabled={!notif.enabled}
        onChange={(v) => set({ onlyWhenUnwatched: v })}
      />
      <CheckRow
        label="Play a sound"
        checked={notif.sound}
        disabled={!notif.enabled}
        onChange={(v) => set({ sound: v })}
      />
      <div style={{ fontSize: '0.72rem', color: 'var(--wks-text-disabled)' }}>
        Alerts when an agent needs approval/input or finishes. Ctrl+Alt+→ jumps to the next agent that needs you.
      </div>
    </Section>
  );
};

export default NotificationsSection;
