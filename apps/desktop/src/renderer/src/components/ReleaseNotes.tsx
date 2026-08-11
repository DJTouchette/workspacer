import React, { useState } from 'react';
import { ChevronDown, ChevronRight } from 'lucide-react';
import { CHANGELOG, type ChangelogRelease } from '../lib/changelog.generated';
import { Markdown } from './markdown';
import { Surface } from './Surface';

/**
 * The release notes, rendered from CHANGELOG.md.
 *
 * The data is generated at build time (lib/changelog.generated.ts) rather than
 * fetched, because the three places this has to work are a packaged app with no
 * repo, a web renderer with no filesystem, and a machine that is offline. The
 * GitHub Release body for a tag comes from the same markdown, extracted by
 * release.yml — so what you read here and what you read on the release page
 * cannot disagree.
 *
 * Entries keep their inline markdown, so `code` and **bold** render the way they
 * read in the file.
 */

/** Section-title → tone. Anything unlisted is untinted rather than guessed at. */
const SECTION_TONE: Record<string, string> = {
  added: 'var(--wks-success)',
  changed: 'var(--wks-accent)',
  fixed: 'var(--wks-warning)',
  removed: 'var(--wks-error)',
  security: 'var(--wks-error)',
  deprecated: 'var(--wks-text-faint)',
};

const toneFor = (title: string): string =>
  SECTION_TONE[title.trim().toLowerCase()] ?? 'var(--wks-text-faint)';

export const ReleaseEntry: React.FC<{ release: ChangelogRelease }> = ({ release }) => (
  <div style={{ display: 'flex', flexDirection: 'column', gap: 10, minWidth: 0 }}>
    {release.sections.map((s) => (
      <div key={s.title} style={{ display: 'flex', flexDirection: 'column', gap: 4, minWidth: 0 }}>
        <div
          style={{
            fontSize: '0.6rem',
            textTransform: 'uppercase',
            letterSpacing: 0.6,
            color: toneFor(s.title),
          }}
        >
          {s.title}
        </div>
        {s.items.map((item, i) => (
          <div
            key={i}
            style={{
              fontSize: '0.72rem',
              lineHeight: 1.5,
              color: 'var(--wks-text-secondary)',
              paddingLeft: 10,
              // One rule per entry rather than a bullet glyph: entries here are
              // paragraphs, and a bullet on a wrapped paragraph reads as a list
              // of fragments.
              borderLeft: `2px solid ${toneFor(s.title)}`,
              minWidth: 0,
            }}
          >
            <Markdown text={item} />
          </div>
        ))}
      </div>
    ))}
  </div>
);

/**
 * Every release, newest first. The newest is open; the rest are collapsed, so
 * the pane opens on "what changed" rather than on a wall of history.
 */
export const ReleaseNotes: React.FC<{ highlightVersion?: string }> = ({ highlightVersion }) => {
  const initial = CHANGELOG.findIndex(
    (r) => !highlightVersion || r.version === highlightVersion || r.unreleased,
  );
  const [open, setOpen] = useState<Set<string>>(
    () => new Set(CHANGELOG.length ? [CHANGELOG[Math.max(0, initial)].version] : []),
  );

  if (CHANGELOG.length === 0) {
    return (
      <div style={{ fontSize: '0.72rem', color: 'var(--wks-text-disabled)' }}>
        No release notes are bundled with this build.
      </div>
    );
  }

  return (
    <div style={{ display: 'flex', flexDirection: 'column', gap: 8, minWidth: 0 }}>
      {CHANGELOG.map((r) => {
        const isOpen = open.has(r.version);
        return (
          <Surface key={r.version} elevation="raised" radius="md" pad="md">
            <button
              onClick={() =>
                setOpen((prev) => {
                  const next = new Set(prev);
                  if (next.has(r.version)) next.delete(r.version);
                  else next.add(r.version);
                  return next;
                })
              }
              aria-expanded={isOpen}
              style={{
                display: 'flex',
                alignItems: 'center',
                gap: 8,
                width: '100%',
                border: 'none',
                background: 'transparent',
                padding: 0,
                cursor: 'pointer',
                color: 'var(--wks-text-primary)',
                textAlign: 'left',
              }}
            >
              {isOpen ? (
                <ChevronDown size={13} strokeWidth={2} />
              ) : (
                <ChevronRight size={13} strokeWidth={2} />
              )}
              <span style={{ fontSize: '0.8rem', fontWeight: 600 }}>
                {r.unreleased ? 'Unreleased' : `v${r.version}`}
              </span>
              {r.date && (
                <span style={{ fontSize: '0.66rem', color: 'var(--wks-text-faint)' }}>
                  {r.date}
                </span>
              )}
              {r.version === highlightVersion && (
                <span
                  style={{
                    fontSize: '0.6rem',
                    color: 'var(--wks-accent-text)',
                    background: 'var(--wks-accent-bg)',
                    borderRadius: 'var(--wks-radius-pill)',
                    padding: '1px 8px',
                  }}
                >
                  running
                </span>
              )}
            </button>
            {isOpen && (
              <div style={{ marginTop: 10 }}>
                <ReleaseEntry release={r} />
              </div>
            )}
          </Surface>
        );
      })}
    </div>
  );
};
