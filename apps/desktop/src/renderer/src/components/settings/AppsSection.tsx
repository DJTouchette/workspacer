import React, { useState, useCallback, useEffect } from 'react';
import { X } from 'lucide-react';
import { Config, AppEntry } from '../../hooks/useConfig';
import { Section, SmallButton, inputStyle } from './primitives';

interface AppsSectionProps {
  config: Config;
  save: (partial: Partial<Config>) => Promise<Config>;
}

const AppsSection: React.FC<AppsSectionProps> = ({ config, save }) => {
  const [apps, setApps] = useState<AppEntry[]>(config.apps ?? []);
  useEffect(() => {
    setApps(config.apps ?? []);
  }, [config.apps]);
  const [editingAppIndex, setEditingAppIndex] = useState<number | null>(null);
  const [editName, setEditName] = useState('');
  const [editUrl, setEditUrl] = useState('');
  const [editIcon, setEditIcon] = useState('');

  const saveApps = useCallback(
    (newApps: AppEntry[]) => {
      setApps(newApps);
      save({ apps: newApps });
    },
    [save],
  );

  const handleAddApp = useCallback(() => {
    const newApps = [...apps, { name: 'New App', url: 'https://', icon: '\u{1F310}' }];
    saveApps(newApps);
    setEditingAppIndex(newApps.length - 1);
    setEditName('New App');
    setEditUrl('https://');
    setEditIcon('\u{1F310}');
  }, [apps, saveApps]);

  const handleEditApp = useCallback(
    (index: number) => {
      const app = apps[index];
      setEditingAppIndex(index);
      setEditName(app.name);
      setEditUrl(app.url);
      setEditIcon(app.icon || '');
    },
    [apps],
  );

  const handleSaveApp = useCallback(() => {
    if (editingAppIndex === null) return;
    const newApps = [...apps];
    newApps[editingAppIndex] = {
      name: editName.trim() || 'App',
      url: editUrl.trim() || 'https://',
      icon: editIcon.trim() || undefined,
    };
    saveApps(newApps);
    setEditingAppIndex(null);
  }, [editingAppIndex, editName, editUrl, editIcon, apps, saveApps]);

  const handleDeleteApp = useCallback(
    (index: number) => {
      const newApps = apps.filter((_, i) => i !== index);
      saveApps(newApps);
      if (editingAppIndex === index) setEditingAppIndex(null);
    },
    [apps, saveApps, editingAppIndex],
  );

  return (
    <Section title="Apps (Ctrl+K)">
      <div style={{ display: 'flex', flexDirection: 'column', gap: '4px' }}>
        {apps.map((app, i) => (
          <div key={i}>
            {editingAppIndex === i ? (
              /* Edit mode */
              <div
                style={{
                  display: 'flex',
                  flexDirection: 'column',
                  gap: '4px',
                  padding: '8px',
                  backgroundColor: 'var(--wks-bg-surface)',
                  borderRadius: '4px',
                  border: '1px solid var(--wks-border-input)',
                }}
              >
                <div style={{ display: 'flex', gap: '4px' }}>
                  <input
                    value={editIcon}
                    onChange={(e) => setEditIcon(e.target.value)}
                    placeholder="Icon"
                    style={{ ...inputStyle, width: '40px', textAlign: 'center' }}
                  />
                  <input
                    value={editName}
                    onChange={(e) => setEditName(e.target.value)}
                    placeholder="Name"
                    style={{ ...inputStyle, flex: 1 }}
                    autoFocus
                  />
                </div>
                <input
                  value={editUrl}
                  onChange={(e) => setEditUrl(e.target.value)}
                  placeholder="https://..."
                  style={{ ...inputStyle, fontFamily: 'var(--wks-font-mono)' }}
                  onKeyDown={(e) => {
                    if (e.key === 'Enter') handleSaveApp();
                  }}
                />
                <div style={{ display: 'flex', gap: '4px', justifyContent: 'flex-end' }}>
                  <SmallButton label="Cancel" onClick={() => setEditingAppIndex(null)} />
                  <SmallButton label="Save" onClick={handleSaveApp} primary />
                </div>
              </div>
            ) : (
              /* Display mode */
              <div
                style={{
                  display: 'flex',
                  alignItems: 'center',
                  gap: '8px',
                  padding: '4px 8px',
                  borderRadius: '4px',
                }}
                onMouseEnter={(e) => {
                  (e.currentTarget as HTMLElement).style.backgroundColor = 'var(--wks-bg-hover)';
                }}
                onMouseLeave={(e) => {
                  (e.currentTarget as HTMLElement).style.backgroundColor = 'transparent';
                }}
              >
                <span style={{ fontSize: '0.85rem', width: '20px', textAlign: 'center' }}>
                  {app.icon || '\u{1F310}'}
                </span>
                <div style={{ flex: 1, minWidth: 0 }}>
                  <div
                    style={{
                      fontSize: '0.7rem',
                      color: 'var(--wks-text-secondary)',
                      fontWeight: 500,
                    }}
                  >
                    {app.name}
                  </div>
                  <div
                    style={{
                      fontSize: '0.64rem',
                      color: 'var(--wks-text-faint)',
                      overflow: 'hidden',
                      textOverflow: 'ellipsis',
                      whiteSpace: 'nowrap',
                    }}
                  >
                    {app.url}
                  </div>
                </div>
                <SmallButton label="Edit" onClick={() => handleEditApp(i)} />
                <SmallButton
                  label={<X size={11} strokeWidth={2.25} />}
                  onClick={() => handleDeleteApp(i)}
                  danger
                />
              </div>
            )}
          </div>
        ))}

        <button
          onClick={handleAddApp}
          style={{
            padding: '6px 12px',
            fontSize: '0.68rem',
            fontFamily: 'inherit',
            fontWeight: 500,
            backgroundColor: 'var(--wks-bg-elevated)',
            color: 'var(--wks-text-muted)',
            border: '1px dashed var(--wks-border-input)',
            borderRadius: '4px',
            cursor: 'pointer',
            height: 'auto',
            lineHeight: 1.4,
            margin: '4px 0 0',
            width: '100%',
          }}
          onMouseEnter={(e) => {
            (e.currentTarget as HTMLElement).style.borderColor = 'var(--wks-accent)';
            (e.currentTarget as HTMLElement).style.color = 'var(--wks-text-secondary)';
          }}
          onMouseLeave={(e) => {
            (e.currentTarget as HTMLElement).style.borderColor = 'var(--wks-border-input)';
            (e.currentTarget as HTMLElement).style.color = 'var(--wks-text-muted)';
          }}
        >
          + Add App
        </button>
      </div>
    </Section>
  );
};

export default AppsSection;
