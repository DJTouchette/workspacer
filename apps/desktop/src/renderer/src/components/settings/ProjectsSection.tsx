import React, { useCallback, useMemo, useState } from 'react';
import { RotateCcw, Check, AlertTriangle } from 'lucide-react';
import { Config, ProjectIdentity } from '../../hooks/useConfig';
import { Section, SmallButton, inputStyle } from './primitives';
import { ProjectMark } from '../ProjectMark';
import { projectKey } from '../../lib/projectKey';
import { basenameOf } from '../../lib/projectIdentity';

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

  // Every directory the app already knows, in the order a person would expect
  // to find them: the ones they pinned, then the ones they've touched, then the
  // ones that are only known because they carry other per-directory config.
  const dirs = useMemo(() => {
    const seen = new Set<string>();
    const out: string[] = [];
    const push = (d?: string) => {
      if (!d) return;
      const k = projectKey(d);
      if (!k || seen.has(k)) return;
      seen.add(k);
      out.push(k);
    };
    (config.directories?.favourites ?? []).forEach(push);
    (config.directories?.recent ?? []).forEach(push);
    Object.keys(config.scripts ?? {}).forEach(push);
    Object.keys(config.widgets ?? {}).forEach(push);
    // A configured project that has since dropped off every other list must
    // still be editable — otherwise its customization is stranded.
    Object.keys(projects).forEach(push);
    return out;
  }, [config.directories, config.scripts, config.widgets, projects]);

  const update = useCallback(
    (dir: string, patch: Partial<ProjectIdentity>) => {
      const key = projectKey(dir);
      const next: Record<string, ProjectIdentity> = { ...projects };
      const merged = { ...(next[key] ?? {}), ...patch };
      // Drop blanks so an entry never persists as "all defaults" — and when the
      // last field is cleared, drop the entry itself. `projects` is replaced
      // wholesale on save (configService), so a delete really deletes.
      for (const k of Object.keys(merged) as (keyof ProjectIdentity)[]) {
        if (!String(merged[k] ?? '').trim()) delete merged[k];
      }
      if (Object.keys(merged).length) next[key] = merged;
      else delete next[key];
      void save({ projects: next });
    },
    [projects, save],
  );

  /** Per-row download state, so a paste reports back instead of silently
   *  doing nothing (or worse, appearing to work while the URL 404s). */
  const [status, setStatus] = useState<Record<string, 'loading' | 'ok' | string>>({});

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
      {dirs.map((dir) => {
        const entry = projects[dir] ?? {};
        const customized = Object.keys(entry).length > 0;
        const st = status[dir];
        const failed = Boolean(st) && st !== 'ok' && st !== 'loading';
        return (
          <div key={dir} style={{ borderTop: '1px solid var(--wks-border-subtle)' }}>
            <div style={{ display: 'flex', alignItems: 'center', gap: 10, padding: '8px 0' }}>
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
                value={entry.color || '#6b8afd'}
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
                <SmallButton
                  onClick={() => reset(dir)}
                  label={<RotateCcw size={12} strokeWidth={2} />}
                />
              ) : (
                <span style={{ width: 26, flex: 'none' }} aria-hidden />
              )}
            </div>
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
