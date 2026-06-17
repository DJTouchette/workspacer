import React, { useState, useCallback } from 'react';
import { Config } from '../../hooks/useConfig';
import { Section, Row } from './primitives';

function ChromeCookieSyncRow() {
  const isWindows = window.electronAPI?.platform === 'win32';
  const [syncing, setSyncing] = useState(false);
  const [lastResult, setLastResult] = useState<string | null>(null);
  const [restrictDomains, setRestrictDomains] = useState(true);
  const [browser, setBrowser] = useState<'chrome' | 'edge'>('chrome');

  // Hosts we'll import when "restrict" is checked. Anything you visit in
  // Chrome that's not in this list stays in Chrome.
  const defaultDomains = [
    'atlassian.com',
    'atlassian.net',
    'microsoftonline.com',
    'microsoft.com',
    'live.com',
    'office.com',
    'office365.com',
    'google.com',
    'github.com',
  ];

  const onSync = useCallback(async () => {
    setSyncing(true);
    setLastResult(null);
    try {
      const res = await window.electronAPI.importChromeCookies(
        restrictDomains ? defaultDomains : undefined,
        'cdp',
        browser,
      );
      const diag = (res as any).diagnostics ?? {};
      const diagStr = Object.keys(diag).length
        ? ' — ' + Object.entries(diag).map(([k, v]) => `${k}=${v}`).join(', ')
        : '';
      const msg = `Imported ${res.imported}, skipped ${res.skipped}` + diagStr +
        (res.errors.length ? `\nFirst error: ${res.errors[0]}` : '');
      setLastResult(msg);
    } catch (err: any) {
      setLastResult(`Failed: ${err?.message ?? err}`);
    } finally {
      setSyncing(false);
    }
  }, [restrictDomains, browser]);

  if (!isWindows) {
    return (
      <div style={{ fontSize: '0.55rem', color: 'var(--wks-text-disabled)' }}>
        Cookie import from Chrome/Edge is available on Windows only.
      </div>
    );
  }

  return (
    <>
      <Row label="Source browser">
        <select
          value={browser}
          onChange={(e) => setBrowser(e.target.value as 'chrome' | 'edge')}
          style={{
            height: '24px',
            padding: '0 8px',
            fontSize: '0.65rem',
            backgroundColor: 'var(--wks-bg-input)',
            color: 'var(--wks-text-secondary)',
            border: '1px solid var(--wks-border)',
            borderRadius: '3px',
            outline: 'none',
            fontFamily: 'inherit',
          }}
        >
          <option value="chrome">Google Chrome</option>
          <option value="edge">Microsoft Edge</option>
        </select>
      </Row>
      <Row label={`Sync cookies from ${browser === 'edge' ? 'Edge' : 'Chrome'}`}>
        <button
          onClick={onSync}
          disabled={syncing}
          style={{
            height: '24px',
            padding: '0 12px',
            fontSize: '0.65rem',
            fontWeight: 600,
            backgroundColor: syncing ? 'var(--wks-bg-input)' : 'var(--wks-accent-bg)',
            color: syncing ? 'var(--wks-text-disabled)' : 'var(--wks-accent-text)',
            border: '1px solid var(--wks-border)',
            borderRadius: '3px',
            cursor: syncing ? 'default' : 'pointer',
            fontFamily: 'inherit',
          }}
        >
          {syncing ? 'Syncing…' : 'Sync now'}
        </button>
      </Row>
      <Row label="Restrict to login-related domains">
        <input
          type="checkbox"
          checked={restrictDomains}
          onChange={(e) => setRestrictDomains(e.target.checked)}
        />
      </Row>
      <div style={{ fontSize: '0.55rem', color: 'var(--wks-text-disabled)' }}>
        Reads Chrome's local cookie store and copies into Workspacer's browser session — useful when OAuth (e.g. Microsoft sign-in) won't complete inside an embedded webview. Run while Chrome is closed for best results.
        {lastResult && (
          <div style={{ marginTop: 4, color: 'var(--wks-text-secondary)' }}>{lastResult}</div>
        )}
      </div>
    </>
  );
}

interface BrowserSectionProps {
  config: Config;
  save: (partial: Partial<Config>) => Promise<Config>;
}

const BrowserSection: React.FC<BrowserSectionProps> = ({ config, save }) => {
  return (
    <Section title="Browser">
      <Row label="Hibernate after (seconds)">
        <input
          type="number"
          min={0}
          step={30}
          value={config.browser?.hibernateAfter ?? 300}
          onChange={(e) => {
            const val = parseInt(e.target.value, 10);
            if (!isNaN(val) && val >= 0) {
              save({ browser: { ...config.browser, hibernateAfter: val } });
            }
          }}
          style={{
            width: '80px',
            height: '24px',
            padding: '0 8px',
            fontSize: '0.65rem',
            backgroundColor: 'var(--wks-bg-input)',
            color: 'var(--wks-text-secondary)',
            border: '1px solid var(--wks-border)',
            borderRadius: '3px',
            outline: 'none',
            fontFamily: 'monospace',
            textAlign: 'right',
          }}
        />
      </Row>
      <div style={{ fontSize: '0.55rem', color: 'var(--wks-text-disabled)' }}>
        Browser panes hibernate after being out of view. 0 = disabled.
      </div>

      <ChromeCookieSyncRow />
    </Section>
  );
};

export default BrowserSection;
