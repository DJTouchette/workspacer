import React, { useCallback, useEffect, useMemo, useState } from 'react';
import { RotateCcw, Check, AlertTriangle, Star } from 'lucide-react';
import { Config, ProjectIdentity } from '../../hooks/useConfig';
import { Section, SmallButton, inputStyle } from './primitives';
import { ProjectMark } from '../ProjectMark';
import { projectKey } from '../../lib/projectKey';
import {
  listProjects,
  patchProject,
  setFavourite,
  projectPluginSettings,
  setProjectPluginSettings,
} from '../../lib/projectRegistry';
import { usePluginsContext } from '../../contexts/PluginsContext';
import type { PluginManifest, PluginSettingDef } from '../../types/plugin';
import { basenameOf, resolveProject } from '../../lib/projectIdentity';

interface ProjectsSectionProps {
  config: Config;
  save: (partial: Partial<Config>) => Promise<Config>;
}

/**
 * Give each project directory a face.
 *
 * There is no "add a project" here on purpose: a project is a directory the app
 * already knows about, so the list is assembled from the ones it has seen —
 * favourites, recents, and anything that already has scripts or a widget board.
 * Asking someone to re-type a path they've already opened would be busywork,
 * and a project that isn't in any of those lists isn't one you're working in.
 *
 * Every row starts DERIVED rather than empty: initials from the name, colour
 * from the path. So this page is where you go to override something you didn't
 * like, not a form you must fill in before the feature does anything.
 */
const ProjectsSection: React.FC<ProjectsSectionProps> = ({ config, save }) => {
  const projects = useMemo(() => config.projects ?? {}, [config.projects]);

  // One source of truth — the registry unions the projects map with the legacy
  // directories arrays and orders them. This used to be a four-source union
  // inlined here, which was the smell that motivated collapsing them.
  const known = useMemo(() => listProjects(config), [config]);
  // Plugins that declare per-project settings. Three plugins invented this
  // privately before it existed here; now they declare it and the host stores
  // it beside the project's identity, so one page answers everything about a
  // project rather than one page per plugin.
  const { plugins } = usePluginsContext();
  const projectScoped = useMemo(
    () =>
      (plugins ?? [])
        .map((p: PluginManifest) => ({
          plugin: p,
          defs: (p.settings ?? []).filter((d: PluginSettingDef) => d.scope === 'project'),
        }))
        .filter((e) => e.defs.length > 0),
    [plugins],
  );
  const dirs = useMemo(() => known.map((p) => p.dir), [known]);

  const update = useCallback(
    (dir: string, patch: Partial<ProjectIdentity>) => {
      void save(patchProject(config, dir, patch));
    },
    [config, save],
  );

  /** Per-row download state, so a paste reports back instead of silently
   *  doing nothing (or worse, appearing to work while the URL 404s). */
  const [status, setStatus] = useState<Record<string, 'loading' | 'ok' | string>>({});

  /**
   * Which listed directories no longer exist. A project entry outlives the
   * directory it describes — a repo you deleted or moved keeps its row, its
   * icon and its plugin settings forever — and nothing else would ever notice.
   * Checked once on mount rather than on every render: it is one readDir per
   * project, and a directory does not come and go while a settings page is open.
   */
  const [missing, setMissing] = useState<Set<string>>(new Set());
  useEffect(() => {
    let cancelled = false;
    // Optional-chained through electronAPI itself, not just readDir: the seam is
    // assigned by the preload (desktop) or install.ts (web), and anything
    // rendering this component outside both — a harness, a test, the instant
    // before that assignment — has no object to reach through at all.
    const probe = window.electronAPI?.readDir;
    if (!probe) return; // web backend — no local filesystem to check against
    void Promise.all(
      dirs.map(async (d) => {
        try {
          await probe(d);
          return null;
        } catch {
          return d;
        }
      }),
    ).then((res) => {
      if (!cancelled) setMissing(new Set(res.filter(Boolean) as string[]));
    });
    return () => {
      cancelled = true;
    };
    // Deliberately keyed on the JOINED list, not the array identity, so a
    // re-render that produces an equal list does not re-probe the filesystem.
  }, [dirs.join('\u0000')]);

  /** Forget everything the app remembers about a directory that is gone. */
  const forget = useCallback(
    (dir: string) => {
      const key = projectKey(dir);
      const projectsNext = { ...(config.projects ?? {}) };
      delete projectsNext[key];
      const scripts = { ...(config.scripts ?? {}) };
      delete scripts[key];
      const widgets = { ...(config.widgets ?? {}) };
      delete widgets[key];
      // The legacy arrays too, or a forgotten directory reappears from them on
      // the next render — this is the one place that still writes them.
      void save({
        projects: projectsNext,
        scripts,
        widgets,
        directories: {
          recent: (config.directories?.recent ?? []).filter((d) => projectKey(d) !== key),
          favourites: (config.directories?.favourites ?? []).filter((d) => projectKey(d) !== key),
        },
      });
    },
    [config, save],
  );

  /**
   * Fetch the pasted URL and store the file. The URL is kept alongside as
   * provenance; the cached file is what renders, so the mark survives going
   * offline, the URL rotting, and re-renders that would otherwise re-request it.
   */
  const fetchIcon = useCallback(
    async (dir: string, url: string) => {
      const key = projectKey(dir);
      const trimmed = url.trim();
      if (!trimmed) {
        setStatus((s) => ({ ...s, [key]: '' }));
        update(dir, { favicon: '', iconFile: '' });
        return;
      }
      setStatus((s) => ({ ...s, [key]: 'loading' }));
      const res = await window.electronAPI.downloadProjectIcon?.(trimmed);
      if (!res) {
        setStatus((s) => ({ ...s, [key]: 'Downloading icons needs the desktop app' }));
        return;
      }
      if (!res.ok) {
        // Keep the URL in the field so the user can fix a typo rather than
        // retype it, but do not pretend an icon was stored.
        setStatus((s) => ({ ...s, [key]: res.error }));
        update(dir, { favicon: trimmed, iconFile: '' });
        return;
      }
      setStatus((s) => ({ ...s, [key]: 'ok' }));
      update(dir, { favicon: trimmed, iconFile: res.file });
    },
    [update],
  );

  const reset = useCallback(
    (dir: string) => update(dir, { label: '', color: '', icon: '', favicon: '', iconFile: '' }),
    [update],
  );

  return (
    <Section title="Projects">
      <div
        style={{
          fontSize: '0.72rem',
          color: 'var(--wks-text-muted)',
          lineHeight: 1.6,
          marginBottom: 10,
        }}
      >
        How each project reads at a glance. Every project already has a mark — initials from its
        name, colour from its path — so these are overrides, not a form to fill in. A favicon URL
        wins over an emoji when it loads.
      </div>
      {!dirs.length && (
        <div style={{ fontSize: '0.72rem', color: 'var(--wks-text-faint)', padding: '8px 0' }}>
          No projects yet. Open an agent in a directory and it will appear here.
        </div>
      )}
      {dirs.map((dir, i) => {
        const entry = projects[dir] ?? {};
        const customized = Object.keys(entry).length > 0;
        const st = status[dir];
        const failed = Boolean(st) && st !== 'ok' && st !== 'loading';
        return (
          <div key={dir} style={{ borderTop: '1px solid var(--wks-border-subtle)' }}>
            <div style={{ display: 'flex', alignItems: 'center', gap: 10, padding: '8px 0' }}>
              <button
                onClick={() => void save(setFavourite(config, dir, !known[i].favourite))}
                title={known[i].favourite ? 'Unpin from Overview' : 'Pin to Overview'}
                aria-label={known[i].favourite ? 'Unpin from Overview' : 'Pin to Overview'}
                style={{
                  appearance: 'none',
                  border: 'none',
                  background: 'transparent',
                  cursor: 'pointer',
                  padding: 0,
                  display: 'flex',
                  flex: 'none',
                  color: known[i].favourite ? 'var(--wks-warning)' : 'var(--wks-text-faint)',
                }}
              >
                <Star
                  size={13}
                  strokeWidth={1.75}
                  fill={known[i].favourite ? 'currentColor' : 'none'}
                />
              </button>
              <ProjectMark cwd={dir} projects={projects} size={22} />
              <div style={{ minWidth: 0, flex: 1 }}>
                <input
                  style={{ ...inputStyle, width: '100%' }}
                  value={entry.label ?? ''}
                  placeholder={basenameOf(dir)}
                  spellCheck={false}
                  aria-label={`Display name for ${dir}`}
                  onChange={(e) => update(dir, { label: e.target.value })}
                />
                <div
                  title={dir}
                  style={{
                    fontSize: '0.66rem',
                    color: 'var(--wks-text-faint)',
                    fontFamily: 'var(--wks-font-mono)',
                    overflow: 'hidden',
                    textOverflow: 'ellipsis',
                    whiteSpace: 'nowrap',
                    marginTop: 3,
                  }}
                >
                  {dir}
                </div>
              </div>
              <input
                style={{ ...inputStyle, width: 54, textAlign: 'center' }}
                value={entry.icon ?? ''}
                placeholder="🚀"
                spellCheck={false}
                title="An emoji, or one or two letters. Leave empty for initials."
                aria-label={`Icon for ${dir}`}
                onChange={(e) => update(dir, { icon: e.target.value.slice(0, 4) })}
              />
              {/* A native colour input rather than a hand-rolled picker: it is the
                one control the OS already does well, and this is a colour. */}
              <input
                type="color"
                // The RESOLVED colour, not a hardcoded default: an unconfigured
                // project has a derived colour, and showing blue beside a teal
                // mark tells the user something false about their own project.
                value={resolveProject(dir, projects)?.color ?? '#6b8afd'}
                title="Badge colour. Reset to fall back to the one derived from the path."
                aria-label={`Colour for ${dir}`}
                style={{
                  width: 28,
                  height: 26,
                  padding: 0,
                  border: '1px solid var(--wks-border-subtle)',
                  borderRadius: 'var(--wks-radius-sm)',
                  background: 'transparent',
                  cursor: 'pointer',
                }}
                onChange={(e) => update(dir, { color: e.target.value })}
              />
              {/* Committed on blur or Enter rather than per keystroke: this
                triggers a network fetch, and firing one per character typed
                would hammer whatever host the URL points at. */}
              <IconUrlField
                value={entry.favicon ?? ''}
                status={status[dir]}
                onCommit={(v) => fetchIcon(dir, v)}
                label={`Icon URL for ${dir}`}
              />
              {/* Only when there is something to undo — a permanently-visible
                disabled control on every row is noise. */}
              {customized ? (
                <span
                  style={{ width: 30, flex: 'none', display: 'flex', justifyContent: 'center' }}
                >
                  <SmallButton
                    onClick={() => reset(dir)}
                    label={<RotateCcw size={12} strokeWidth={2} />}
                  />
                </span>
              ) : (
                <span style={{ width: 30, flex: 'none' }} aria-hidden />
              )}
            </div>
            {missing.has(dir) && (
              <div
                style={{
                  display: 'flex',
                  alignItems: 'center',
                  gap: 8,
                  padding: '0 0 8px 32px',
                  fontSize: '0.66rem',
                  color: 'var(--wks-text-muted)',
                }}
              >
                <span>This directory no longer exists.</span>
                <SmallButton onClick={() => forget(dir)} label="Forget it" />
              </div>
            )}
            {/* Fleet Manager dispatch policy for THIS project — how it lands
                work (delivery mode) and whether its workers skip approvals
                (yolo). Read at dispatch and baked into each worker's brief. */}
            <div
              style={{
                display: 'flex',
                alignItems: 'center',
                gap: 10,
                padding: '0 0 8px 32px',
                flexWrap: 'wrap',
              }}
            >
              <span
                style={{
                  fontSize: '0.6rem',
                  fontWeight: 600,
                  textTransform: 'uppercase',
                  letterSpacing: '0.06em',
                  color: 'var(--wks-text-faint)',
                }}
              >
                Fleet
              </span>
              <label
                style={{
                  display: 'inline-flex',
                  alignItems: 'center',
                  gap: 5,
                  fontSize: '0.66rem',
                  color: 'var(--wks-text-muted)',
                }}
              >
                Delivery
                <select
                  value={entry.delivery ?? 'pr'}
                  aria-label={`Fleet delivery mode for ${dir}`}
                  title="How the Fleet Manager lands work here: open a PR for review, or land changes locally for an approved merge."
                  onChange={(e) => update(dir, { delivery: e.target.value as 'pr' | 'local' })}
                  style={{ ...inputStyle, width: 150 }}
                >
                  <option value="pr">Pull request (review)</option>
                  <option value="local">Local merge (approve)</option>
                </select>
              </label>
              <label
                style={{
                  display: 'inline-flex',
                  alignItems: 'center',
                  gap: 5,
                  fontSize: '0.66rem',
                  color: 'var(--wks-text-muted)',
                  cursor: 'pointer',
                }}
                title="Workers dispatched into this project run with permissions bypassed (no per-action approvals). Off by default."
              >
                <input
                  type="checkbox"
                  checked={entry.yolo === true}
                  aria-label={`Full access for workers in ${dir}`}
                  onChange={(e) => update(dir, { yolo: e.target.checked })}
                />
                Full access
              </label>
            </div>
            {/* What each plugin needs to know about THIS project. Rendered
                under the identity row rather than in a separate page, because
                "which Jira project is this repo" is a fact about the project,
                not about Jira. */}
            {projectScoped.map(({ plugin, defs }) => {
              const values = projectPluginSettings(config, dir, plugin.id);
              return (
                <div
                  key={plugin.id}
                  style={{
                    display: 'flex',
                    alignItems: 'center',
                    gap: 8,
                    padding: '0 0 8px 32px',
                    flexWrap: 'wrap',
                  }}
                >
                  <span
                    style={{
                      fontSize: '0.6rem',
                      fontWeight: 600,
                      textTransform: 'uppercase',
                      letterSpacing: '0.06em',
                      color: 'var(--wks-text-faint)',
                    }}
                  >
                    {plugin.name}
                  </span>
                  {defs.map((d) => (
                    <label
                      key={d.key}
                      style={{
                        display: 'inline-flex',
                        alignItems: 'center',
                        gap: 5,
                        fontSize: '0.66rem',
                        color: 'var(--wks-text-muted)',
                      }}
                    >
                      {d.label}
                      <input
                        style={{ ...inputStyle, width: 130 }}
                        value={String(values[d.key] ?? '')}
                        placeholder={String(d.default ?? '')}
                        spellCheck={false}
                        title={d.help}
                        onChange={(e) =>
                          void save(
                            setProjectPluginSettings(config, dir, plugin.id, {
                              ...values,
                              [d.key]: e.target.value,
                            }),
                          )
                        }
                      />
                    </label>
                  ))}
                </div>
              );
            })}
            {/* The reason, in words. An icon alone says "something" went wrong,
                which is the least useful half of what we know. */}
            {failed && (
              <div
                role="alert"
                style={{
                  fontSize: '0.66rem',
                  color: 'var(--wks-error)',
                  paddingLeft: 32,
                  paddingBottom: 6,
                }}
              >
                {String(status[dir])}
              </div>
            )}
          </div>
        );
      })}
    </Section>
  );
};

/**
 * The icon-URL field, with what happened to it.
 *
 * Held locally while typing so the value doesn't fight the config round-trip,
 * and committed on blur or Enter — each commit is a download, so per-keystroke
 * would mean a request per character.
 */
const IconUrlField: React.FC<{
  value: string;
  status?: 'loading' | 'ok' | string;
  onCommit: (v: string) => void;
  label: string;
}> = ({ value, status, onCommit, label }) => {
  const [draft, setDraft] = useState(value);
  const [focused, setFocused] = useState(false);
  // Adopt external changes (a reset elsewhere) only while not being edited.
  React.useEffect(() => {
    if (!focused) setDraft(value);
  }, [value, focused]);

  const failed = Boolean(status) && status !== 'ok' && status !== 'loading';
  return (
    <span style={{ display: 'inline-flex', alignItems: 'center', gap: 6, flex: 'none' }}>
      <input
        style={{
          ...inputStyle,
          width: 150,
          borderColor: failed ? 'var(--wks-error)' : undefined,
        }}
        value={draft}
        placeholder="paste an icon URL"
        spellCheck={false}
        aria-label={label}
        title="Paste an http(s) image URL. It is downloaded once and kept locally, so it keeps working offline."
        onFocus={() => setFocused(true)}
        onChange={(e) => setDraft(e.target.value)}
        onBlur={() => {
          setFocused(false);
          if (draft.trim() !== value.trim()) onCommit(draft);
        }}
        onKeyDown={(e) => {
          if (e.key === 'Enter') (e.target as HTMLInputElement).blur();
          if (e.key === 'Escape') {
            setDraft(value);
            (e.target as HTMLInputElement).blur();
          }
        }}
      />
      <span style={{ width: 14, flex: 'none', display: 'grid', placeItems: 'center' }}>
        {status === 'loading' && (
          <span
            title="Downloading…"
            style={{
              width: 10,
              height: 10,
              border: '1.5px solid var(--wks-accent)',
              borderTopColor: 'transparent',
              borderRadius: '50%',
              animation: 'claudeSpinner 0.8s linear infinite',
            }}
          />
        )}
        {status === 'ok' && (
          <Check size={12} strokeWidth={2.25} style={{ color: 'var(--wks-success)' }} />
        )}
        {failed && (
          <AlertTriangle
            size={12}
            strokeWidth={2}
            style={{ color: 'var(--wks-error)' }}
            aria-label={String(status)}
          />
        )}
      </span>
    </span>
  );
};

export default ProjectsSection;
