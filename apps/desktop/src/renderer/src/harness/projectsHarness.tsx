/**
 * Standalone Projects harness — the settings section and the mark, against a
 * fabricated config, with no Electron and no live app.
 *
 * The interesting states here are not "does it render" but the MIGRATION ones:
 * a config with only the legacy `directories` arrays, only the new `projects`
 * map, or both disagreeing. Those are invisible in a screenshot of the real app
 * (which has exactly one config), so they get their own columns.
 *
 * Open http://localhost:5173/projects-harness.html with the dev server running.
 */
import React from 'react';
import { createRoot } from 'react-dom/client';
import '../App.css';
import { ProjectMark } from '../components/ProjectMark';
import { listProjects } from '../lib/projectRegistry';
import type { Config } from '../hooks/useConfig';

const { default: ProjectsSection } = await import('../components/settings/ProjectsSection');
const { PluginsContext } = await import('../contexts/PluginsContext');

/** Two plugins declaring per-project settings — the case the real app can only
 *  show you if you happen to have those plugins installed. */
const FAKE_PLUGINS = [
  {
    id: 'djtouchette.jira',
    name: 'Jira',
    apiVersion: '1',
    settings: [
      { key: 'prefix', label: 'Prefix', type: 'string', scope: 'project', help: 'e.g. HVMS' },
    ],
  },
  {
    id: 'djtouchette.shiplight',
    name: 'Shiplight',
    apiVersion: '1',
    settings: [
      { key: 'repo', label: 'Repo', type: 'string', scope: 'project' },
      { key: 'token', label: 'Token', type: 'string', secret: true },
    ],
  },
] as any;

/** A config with only the OLD arrays — what an existing user's file looks like. */
const legacyOnly: Partial<Config> = {
  directories: {
    favourites: ['/home/you/work/workspacer'],
    recent: ['/home/you/work/api-gateway', '/home/you/work/api-worker', '/home/you/work/docs-site'],
  },
};

/** A config fully on the new shape, with every identity variation. */
const migrated: Partial<Config> = {
  projects: {
    '/home/you/work/workspacer': {
      label: 'work{spacer}',
      icon: '🚀',
      favourite: true,
      lastOpened: Date.now(),
    },
    '/home/you/work/api-gateway': { color: '#2dd4bf', lastOpened: Date.now() - 3600_000 },
    '/home/you/work/api-worker': { lastOpened: Date.now() - 86_400_000 },
    '/home/you/work/hvms': { label: 'HVMS Platform', icon: 'HV', favourite: true },
    // A downloaded icon that will 404 here — proves the fallback to initials.
    '/home/you/work/docs-site': {
      favicon: 'https://example.invalid/none.png',
      iconFile: 'missing.png',
    },
    '/home/you/work/billing': { plugins: { 'djtouchette.jira': { prefix: 'BILL' } } },
  },
  scripts: { '/home/you/work/only-scripts': [] },
};

/** Both present and DISAGREEING: the legacy array pins it, the map unpins it. */
const conflicting: Partial<Config> = {
  directories: { favourites: ['/home/you/work/was-pinned'], recent: ['/home/you/work/was-pinned'] },
  projects: { '/home/you/work/was-pinned': { favourite: false, label: 'Unpinned since' } },
};

const CASES: Array<[string, Partial<Config>]> = [
  ['legacy arrays only', legacyOnly],
  ['migrated', migrated],
  ['conflicting (explicit wins)', conflicting],
];

const box: React.CSSProperties = {
  border: '1px solid var(--wks-border-subtle)',
  borderRadius: 'var(--wks-radius-lg)',
  padding: 12,
  minWidth: 0,
};
const h: React.CSSProperties = {
  fontSize: '0.66rem',
  textTransform: 'uppercase',
  letterSpacing: '0.06em',
  color: 'var(--wks-text-faint)',
  margin: '0 0 8px',
};

function Harness() {
  const [config, setConfig] = React.useState<Partial<Config>>(migrated);
  // A real save round-trip, in memory — so pinning and editing in the section
  // below actually behave, including the wholesale-replace semantics.
  const save = async (partial: Partial<Config>) => {
    const next = { ...config, ...partial } as Config;
    setConfig(next);
    return next;
  };

  return (
    <div
      style={{ padding: 20, color: 'var(--wks-text-primary)', fontFamily: 'var(--wks-font-sans)' }}
    >
      <h1 style={{ fontSize: '1.05rem', margin: '0 0 4px' }}>Projects harness</h1>
      <p style={{ fontSize: '0.72rem', color: 'var(--wks-text-muted)', margin: '0 0 20px' }}>
        Marks, and what <code>listProjects</code> resolves for each config shape. The editable
        section at the bottom runs against a live in-memory config.
      </p>

      <div
        style={{
          display: 'grid',
          gridTemplateColumns: 'repeat(3, 1fr)',
          gap: 12,
          marginBottom: 24,
        }}
      >
        {CASES.map(([name, cfg]) => (
          <div key={name} style={box}>
            <p style={h}>{name}</p>
            {listProjects(cfg).map((p) => (
              <div
                key={p.dir}
                style={{
                  display: 'flex',
                  alignItems: 'center',
                  gap: 8,
                  padding: '3px 0',
                  fontSize: '0.72rem',
                }}
              >
                <ProjectMark cwd={p.dir} projects={cfg.projects} size={16} />
                <span
                  style={{
                    minWidth: 0,
                    overflow: 'hidden',
                    textOverflow: 'ellipsis',
                    whiteSpace: 'nowrap',
                  }}
                >
                  {p.dir.split('/').pop()}
                </span>
                <span
                  style={{ marginLeft: 'auto', color: 'var(--wks-text-faint)', fontSize: '0.6rem' }}
                >
                  {p.favourite ? 'pinned' : ''}
                </span>
              </div>
            ))}
          </div>
        ))}
      </div>

      <div style={box}>
        <p style={h}>marks at every size</p>
        <div style={{ display: 'flex', alignItems: 'center', gap: 16 }}>
          {[12, 14, 16, 22, 32].map((s) => (
            <ProjectMark
              key={s}
              cwd="/home/you/work/api-gateway"
              projects={migrated.projects}
              size={s}
            />
          ))}
          <ProjectMark cwd="/home/you/work/workspacer" projects={migrated.projects} size={22} />
          <ProjectMark cwd="/home/you/work/hvms" projects={migrated.projects} size={22} />
          <ProjectMark cwd="/home/you/work/docs-site" projects={migrated.projects} size={22} />
          <ProjectMark
            cwd="/home/you/work/api-gateway"
            projects={migrated.projects}
            size={22}
            withLabel
          />
        </div>
      </div>

      <div style={{ marginTop: 24 }}>
        <PluginsContext.Provider
          value={{ plugins: FAKE_PLUGINS, loading: false, error: null, refresh: () => {} } as any}
        >
          <ProjectsSection config={config as Config} save={save} />
        </PluginsContext.Provider>
      </div>
    </div>
  );
}

createRoot(document.getElementById('root')!).render(<Harness />);
