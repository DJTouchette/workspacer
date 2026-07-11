import React, { useState } from 'react';
import { Section, Row, SmallButton } from './primitives';

/**
 * Command line — install the bundled `workspacer` CLI onto PATH. The install
 * itself is the CLI's own `install-cli` subcommand (spawned in main via
 * `cli:install`); this row just triggers it and shows what it printed —
 * the install destination, or PATH instructions when the target dir isn't
 * on PATH yet. Unavailable on the web client (no host filesystem).
 */
const CliSection: React.FC = () => {
  const [busy, setBusy] = useState(false);
  const [result, setResult] = useState<{ ok: boolean; message: string } | null>(null);

  const install = async () => {
    if (busy) return;
    setBusy(true);
    setResult(null);
    try {
      const res = await window.electronAPI.installCli?.();
      setResult(res ?? { ok: false, message: 'not available in this client' });
    } catch (err) {
      setResult({ ok: false, message: err instanceof Error ? err.message : String(err) });
    } finally {
      setBusy(false);
    }
  };

  return (
    <Section title="Command Line">
      <div style={{ fontSize: '0.72rem', color: 'var(--wks-text-disabled)' }}>
        The app bundles the <code>workspacer</code> CLI — a headless-server launcher:{' '}
        <code>workspacer serve</code> runs the same daemons the app uses, with no window, and{' '}
        <code>workspacer status</code> reports what&apos;s running. Installing symlinks/copies it
        onto your PATH (/usr/local/bin → ~/.local/bin; a per-user dir on Windows).
      </div>
      <Row label="Install workspacer command">
        <SmallButton label={busy ? 'Installing…' : 'Install'} onClick={() => void install()} />
      </Row>
      {result && (
        <div
          style={{
            fontSize: '0.72rem',
            fontFamily: 'var(--wks-font-mono, monospace)',
            whiteSpace: 'pre-wrap',
            wordBreak: 'break-word',
            color: result.ok ? 'var(--wks-text-muted)' : 'var(--wks-error, #e05555)',
          }}
        >
          {result.message}
        </div>
      )}
    </Section>
  );
};

export default CliSection;
