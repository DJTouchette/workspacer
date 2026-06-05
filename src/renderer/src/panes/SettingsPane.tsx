import React from 'react';
import { useConfig } from '../hooks/useConfig';
import AppearanceSection from '../components/settings/AppearanceSection';
import LayoutSection from '../components/settings/LayoutSection';
import KeybindingsSection from '../components/settings/KeybindingsSection';
import NotificationsSection from '../components/settings/NotificationsSection';
import SessionSection from '../components/settings/SessionSection';
import BrowserSection from '../components/settings/BrowserSection';
import AppsSection from '../components/settings/AppsSection';
import ClaudeProfilesSection from '../components/settings/ClaudeProfilesSection';

interface SettingsPaneProps {
  title: string;
}

const SettingsPane: React.FC<SettingsPaneProps> = ({ title }) => {
  const { config, save } = useConfig();

  return (
    <div
      style={{
        display: 'flex',
        flexDirection: 'column',
        height: '100%',
        backgroundColor: 'var(--wks-bg-base)',
        color: 'var(--wks-text-secondary)',
        fontFamily: 'Inter, system-ui, sans-serif',
        fontSize: '12px',
        overflow: 'auto',
        padding: '16px 24px',
      }}
    >
      <h2 style={{ fontSize: '0.9rem', fontWeight: 600, margin: '0 0 16px 0', color: 'var(--wks-text-primary)' }}>
        Settings
      </h2>

      {/* Appearance section */}
      <AppearanceSection config={config} save={save} />

      {/* Layout section */}
      <LayoutSection config={config} save={save} />

      {/* Keybindings section */}
      <KeybindingsSection config={config} save={save} />

      {/* Notifications section */}
      <NotificationsSection config={config} save={save} />

      {/* Session section */}
      <SessionSection config={config} save={save} />

      {/* Browser section */}
      <BrowserSection config={config} save={save} />

      {/* Apps section */}
      <AppsSection config={config} save={save} />

      {/* Claude Profiles section */}
      <ClaudeProfilesSection />
    </div>
  );
};

export default SettingsPane;
