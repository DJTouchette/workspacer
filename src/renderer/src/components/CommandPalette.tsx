import React, { useState, useEffect, useRef, useCallback } from 'react';
import { AppEntry } from '../hooks/useConfig';

interface CommandPaletteProps {
  visible: boolean;
  apps: AppEntry[];
  onClose: () => void;
  onLaunchApp: (app: AppEntry) => void;
}

const CommandPalette: React.FC<CommandPaletteProps> = ({ visible, apps, onClose, onLaunchApp }) => {
  const [query, setQuery] = useState('');
  const [selectedIndex, setSelectedIndex] = useState(0);
  const inputRef = useRef<HTMLInputElement>(null);

  // Focus input and reset state when opening
  useEffect(() => {
    if (visible) {
      setQuery('');
      setSelectedIndex(0);
      setTimeout(() => inputRef.current?.focus(), 0);
    }
  }, [visible]);

  const filtered = apps.filter((app) =>
    app.name.toLowerCase().includes(query.toLowerCase()) ||
    app.url.toLowerCase().includes(query.toLowerCase())
  );

  // Clamp selected index when results change
  useEffect(() => {
    if (selectedIndex >= filtered.length) {
      setSelectedIndex(Math.max(0, filtered.length - 1));
    }
  }, [filtered.length, selectedIndex]);

  const handleKeyDown = useCallback((e: React.KeyboardEvent) => {
    if (e.key === 'Escape') {
      e.preventDefault();
      e.stopPropagation();
      onClose();
    } else if (e.key === 'ArrowDown') {
      e.preventDefault();
      setSelectedIndex((i) => Math.min(i + 1, filtered.length - 1));
    } else if (e.key === 'ArrowUp') {
      e.preventDefault();
      setSelectedIndex((i) => Math.max(i - 1, 0));
    } else if (e.key === 'Enter') {
      e.preventDefault();
      if (filtered[selectedIndex]) {
        onLaunchApp(filtered[selectedIndex]);
        onClose();
      }
    }
  }, [filtered, selectedIndex, onClose, onLaunchApp]);

  if (!visible) return null;

  return (
    <div
      style={{
        position: 'fixed',
        top: 0,
        left: 0,
        right: 0,
        bottom: 0,
        backgroundColor: 'var(--wks-overlay)',
        display: 'flex',
        justifyContent: 'center',
        paddingTop: '15vh',
        zIndex: 2000,
      }}
      onClick={onClose}
    >
      <div
        style={{
          backgroundColor: 'var(--wks-bg-raised)',
          border: '1px solid var(--wks-border-input)',
          borderRadius: '8px',
          width: '400px',
          maxHeight: '400px',
          display: 'flex',
          flexDirection: 'column',
          overflow: 'hidden',
          boxShadow: '0 8px 32px var(--wks-shadow)',
        }}
        onClick={(e) => e.stopPropagation()}
      >
        {/* Search input */}
        <div style={{ padding: '12px 12px 8px' }}>
          <input
            ref={inputRef}
            value={query}
            onChange={(e) => { setQuery(e.target.value); setSelectedIndex(0); }}
            onKeyDown={handleKeyDown}
            placeholder="Launch app..."
            spellCheck={false}
            style={{
              width: '100%',
              height: '32px',
              padding: '0 12px',
              fontSize: '0.8rem',
              fontFamily: 'inherit',
              backgroundColor: 'var(--wks-bg-input)',
              color: 'var(--wks-text-primary)',
              border: '1px solid var(--wks-border-input)',
              borderRadius: '5px',
              outline: 'none',
              boxSizing: 'border-box',
            }}
            onFocus={(e) => { e.currentTarget.style.borderColor = 'var(--wks-accent)'; }}
            onBlur={(e) => { e.currentTarget.style.borderColor = 'var(--wks-border-input)'; }}
          />
        </div>

        {/* Results */}
        <div style={{ overflow: 'auto', padding: '0 4px 8px' }}>
          {filtered.length === 0 && (
            <div style={{ padding: '12px', fontSize: '0.7rem', color: 'var(--wks-text-faint)', textAlign: 'center' }}>
              No apps found
            </div>
          )}
          {filtered.map((app, i) => (
            <div
              key={`${app.name}-${app.url}`}
              onClick={() => {
                onLaunchApp(app);
                onClose();
              }}
              style={{
                display: 'flex',
                alignItems: 'center',
                gap: '10px',
                padding: '8px 12px',
                margin: '0 4px',
                borderRadius: '5px',
                cursor: 'pointer',
                backgroundColor: i === selectedIndex ? 'var(--wks-bg-selected)' : 'transparent',
              }}
              onMouseEnter={() => setSelectedIndex(i)}
            >
              <span style={{ fontSize: '1rem', width: '20px', textAlign: 'center' }}>
                {app.icon || '\u{1F310}'}
              </span>
              <div style={{ flex: 1, minWidth: 0 }}>
                <div style={{ fontSize: '0.75rem', color: 'var(--wks-text-primary)', fontWeight: 500 }}>
                  {app.name}
                </div>
                <div style={{
                  fontSize: '0.6rem',
                  color: 'var(--wks-text-faint)',
                  overflow: 'hidden',
                  textOverflow: 'ellipsis',
                  whiteSpace: 'nowrap',
                }}>
                  {app.url}
                </div>
              </div>
            </div>
          ))}
        </div>
      </div>
    </div>
  );
};

export default CommandPalette;
