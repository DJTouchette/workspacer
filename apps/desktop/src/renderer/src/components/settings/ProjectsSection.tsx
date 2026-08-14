import React, { useCallback, useMemo } from 'react';
import { RotateCcw } from 'lucide-react';
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

  const reset = useCallback(
    (dir: string) => update(dir, { label: '', color: '', icon: '', favicon: '' }),
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
        return (
          <div
            key={dir}
            style={{
              display: 'flex',
              alignItems: 'center',
              gap: 10,
              padding: '8px 0',
              borderTop: '1px solid var(--wks-border-subtle)',
            }}
          >
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
            <input
              style={{ ...inputStyle, width: 150 }}
              value={entry.favicon ?? ''}
              placeholder="favicon URL"
              spellCheck={false}
              title="An http(s) icon URL. Takes precedence over the emoji when it loads."
              aria-label={`Favicon URL for ${dir}`}
              onChange={(e) => update(dir, { favicon: e.target.value.trim() })}
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
        );
      })}
    </Section>
  );
};

export default ProjectsSection;
