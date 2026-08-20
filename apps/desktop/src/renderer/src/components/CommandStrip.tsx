/**
 * CommandStrip — the command layer's armed-state chrome (COMMAND_LAYER.md).
 *
 * Appears the moment the leader arms and lives exactly as long as the armed
 * state (an enabled layer must never have invisible armed state). Two shapes:
 *
 *  - the STRIP: a bottom-centered glass bar — inverted PREFIX chip,
 *    breadcrumb, and a condensed row of the keys reachable right now;
 *  - the HUD: after `hudDelayMs` with no keystroke (a DWELL — on Linux the
 *    leader arms on key-UP, so there is no held key to track), or immediately
 *    inside a submenu, the strip expands into the full grid grouped by the
 *    registry's sections.
 *
 * Everything renders live from buildChordTree over the SAME merged shortcut
 * map the dispatcher matches against, so the chrome cannot drift from the
 * keys. When an approval is pending on the active agent and y/n are bound at
 * the root, the strip says what a blind `y` would approve.
 */
import React, { useEffect, useMemo, useState } from 'react';
import { ChevronRight } from 'lucide-react';
import {
  ACTION_REGISTRY,
  buildChordTree,
  chordBreadcrumb,
  chordNodeAt,
  formatBinding,
  formatCombo,
  CHORD_GROUP_LABELS,
  ACTION_LABELS,
} from '../lib/shortcuts';

const SECTION_OF: Record<string, string> = Object.fromEntries(
  ACTION_REGISTRY.map((a) => [a.action, a.section]),
);

interface StripItem {
  step: string;
  keyLabel: string;
  label: string;
  isGroup: boolean;
  section: string;
}

interface CommandStripProps {
  /** Live chord path (null = idle → hidden). */
  path: string[] | null;
  prefix: string;
  shortcuts: Record<string, string>;
  hudDelayMs: number;
  /** One-line summary of the active agent's pending approval (y/n context). */
  attentionHint?: string | null;
}

const kbdStyle: React.CSSProperties = {
  flexShrink: 0,
  minWidth: 14,
  textAlign: 'center',
  padding: '0 4px',
  borderRadius: 3,
  border: '1px solid var(--wks-border-input)',
  backgroundColor: 'var(--wks-bg-input)',
  color: 'var(--wks-text-secondary)',
  fontSize: '0.66rem',
  fontWeight: 600,
};

const CommandStrip: React.FC<CommandStripProps> = ({
  path,
  prefix,
  shortcuts,
  hudDelayMs,
  attentionHint,
}) => {
  const armed = path !== null;
  const inSubmenu = (path?.length ?? 0) > 0;
  const [dwelled, setDwelled] = useState(false);

  // Dwell: hesitation at the root expands the strip into the HUD. Reset on
  // every path change — a moving hand never sees the grid.
  useEffect(() => {
    setDwelled(false);
    if (!armed || inSubmenu || hudDelayMs <= 0) return;
    const t = setTimeout(() => setDwelled(true), hudDelayMs);
    return () => clearTimeout(t);
  }, [armed, inSubmenu, hudDelayMs, path]);

  const items: StripItem[] = useMemo(() => {
    if (path === null) return [];
    const node = chordNodeAt(buildChordTree(shortcuts), path);
    if (!node) return [];
    return node.children
      .map((c) => {
        const isGroup = c.node.children.length > 0;
        const action = c.node.action ?? '';
        const fullKey = [...path, c.step].join(' ');
        return {
          step: c.step,
          keyLabel: formatCombo(c.step),
          label: isGroup
            ? (CHORD_GROUP_LABELS[fullKey] ?? CHORD_GROUP_LABELS[c.step] ?? formatCombo(c.step))
            : (ACTION_LABELS[action] ?? action),
          isGroup,
          section: isGroup ? 'Menus' : (SECTION_OF[action] ?? 'Other'),
        };
      })
      .sort((a, b) => {
        // Letters/digits before punctuation — the compact strip leads with
        // the keys hands actually rest on (h j k l …), symbols trail.
        const aWord = /^[a-z0-9]/i.test(a.keyLabel) ? 0 : 1;
        const bWord = /^[a-z0-9]/i.test(b.keyLabel) ? 0 : 1;
        if (aWord !== bWord) return aWord - bWord;
        return a.keyLabel.localeCompare(b.keyLabel, undefined, { sensitivity: 'base' });
      });
  }, [path, shortcuts]);

  if (path === null) return null;

  const expanded = dwelled || inSubmenu;
  const crumbs = chordBreadcrumb(path);
  const yBound = items.some((i) => i.step.toLowerCase() === 'y');

  // Section grouping for the HUD, in registry order (Menus last).
  const sections: { section: string; items: StripItem[] }[] = [];
  if (expanded) {
    const order: string[] = [];
    const by = new Map<string, StripItem[]>();
    for (const it of items.filter((i) => !i.isGroup)) {
      if (!by.has(it.section)) {
        by.set(it.section, []);
        order.push(it.section);
      }
      by.get(it.section)!.push(it);
    }
    const groups = items.filter((i) => i.isGroup);
    for (const s of order) sections.push({ section: s, items: by.get(s)! });
    if (groups.length) sections.push({ section: 'Menus', items: groups });
  }

  return (
    <div
      role="status"
      aria-live="polite"
      style={{
        position: 'fixed',
        bottom: 12,
        left: '50%',
        transform: 'translateX(-50%)',
        zIndex: 200,
        maxWidth: 'min(920px, 92vw)',
        backgroundColor: 'var(--wks-glass-strong)',
        backdropFilter: 'blur(var(--wks-glass-blur)) saturate(170%)',
        WebkitBackdropFilter: 'blur(var(--wks-glass-blur)) saturate(170%)',
        border: '1px solid var(--wks-glass-border)',
        borderRadius: 'var(--wks-radius-md)',
        boxShadow:
          '0 12px 36px var(--wks-glass-shadow), inset 0 0 0 1.5px var(--wks-glass-highlight)',
        padding: expanded ? '10px 14px' : '5px 12px',
        fontFamily: 'var(--wks-font-mono)',
      }}
    >
      {/* Header row: inverted PREFIX chip › breadcrumb, plus the condensed
          key hints while compact. */}
      <div
        style={{
          display: 'flex',
          alignItems: 'center',
          gap: 8,
          flexWrap: expanded ? 'wrap' : 'nowrap',
          overflow: 'hidden',
        }}
      >
        <span
          style={{
            flexShrink: 0,
            padding: '1px 7px',
            borderRadius: 3,
            backgroundColor: 'var(--wks-accent)',
            color: 'var(--wks-text-on-accent)',
            fontWeight: 700,
            fontSize: '0.68rem',
          }}
        >
          {formatBinding(prefix)}
        </span>
        {crumbs.map((c, i) => (
          <span
            key={i}
            style={{
              flexShrink: 0,
              display: 'inline-flex',
              alignItems: 'center',
              gap: 4,
              color: 'var(--wks-accent)',
              fontWeight: 700,
              fontSize: '0.7rem',
            }}
          >
            <ChevronRight size={10} strokeWidth={2.25} style={{ color: 'var(--wks-text-faint)' }} />
            {c}
          </span>
        ))}
        {!expanded &&
          items.slice(0, 6).map((it) => (
            <span
              key={it.step}
              style={{
                flexShrink: 0,
                display: 'inline-flex',
                alignItems: 'baseline',
                gap: 5,
                fontSize: '0.66rem',
                color: 'var(--wks-text-muted)',
              }}
            >
              <kbd style={kbdStyle}>{it.keyLabel}</kbd>
              <span
                style={{
                  maxWidth: 96,
                  overflow: 'hidden',
                  textOverflow: 'ellipsis',
                  whiteSpace: 'nowrap',
                }}
              >
                {it.label}
              </span>
            </span>
          ))}
        {!expanded && items.length > 6 && (
          <span style={{ flexShrink: 0, fontSize: '0.62rem', color: 'var(--wks-text-faint)' }}>
            +{items.length - 6} more — wait for the grid
          </span>
        )}
      </div>

      {/* The HUD grid — sections from the registry, rendered from the tree. */}
      {expanded && (
        <div
          style={{
            // Balanced multi-columns, sections kept whole: overall height is
            // the tallest SECTION, not the tallest sum-of-sections — the
            // Command layer's long list flows beside the others instead of
            // stacking a skyscraper column.
            columnWidth: 185,
            columnGap: 22,
            marginTop: 8,
            width: 'min(880px, 88vw)',
          }}
        >
          {sections.map((s) => (
            <div key={s.section} style={{ breakInside: 'avoid', paddingBottom: 8 }}>
              <div
                style={{
                  fontSize: '0.6rem',
                  fontWeight: 700,
                  letterSpacing: '0.08em',
                  textTransform: 'uppercase',
                  color: 'var(--wks-text-faint)',
                  padding: '2px 0 3px',
                }}
              >
                {s.section}
              </div>
              {s.items.map((it) => (
                <div
                  key={it.step}
                  style={{
                    display: 'flex',
                    alignItems: 'baseline',
                    gap: 7,
                    padding: '1px 0',
                    fontSize: '0.68rem',
                    color: 'var(--wks-text-muted)',
                  }}
                >
                  <kbd style={kbdStyle}>{it.keyLabel}</kbd>
                  <span
                    style={{
                      whiteSpace: 'nowrap',
                      overflow: 'hidden',
                      textOverflow: 'ellipsis',
                      fontWeight: it.isGroup ? 600 : 400,
                      color: it.isGroup ? 'var(--wks-text-secondary)' : undefined,
                    }}
                  >
                    {it.label}
                  </span>
                  {it.isGroup && (
                    <ChevronRight
                      size={10}
                      strokeWidth={2.25}
                      style={{
                        flexShrink: 0,
                        alignSelf: 'center',
                        color: 'var(--wks-text-secondary)',
                      }}
                    />
                  )}
                </div>
              ))}
            </div>
          ))}
        </div>
      )}

      {/* Pending-approval context: what a blind y/n would decide. */}
      {yBound && attentionHint && (
        <div
          style={{
            marginTop: expanded ? 7 : 4,
            paddingTop: expanded ? 6 : 3,
            borderTop: '1px solid var(--wks-border)',
            fontSize: '0.64rem',
            color: 'var(--wks-warning)',
            whiteSpace: 'nowrap',
            overflow: 'hidden',
            textOverflow: 'ellipsis',
          }}
        >
          y approve · n deny — {attentionHint}
        </div>
      )}

      {expanded && (
        <div
          style={{
            marginTop: 7,
            paddingTop: 6,
            borderTop: '1px solid var(--wks-border)',
            fontSize: '0.62rem',
            color: 'var(--wks-text-faint)',
            display: 'flex',
            gap: 12,
          }}
        >
          {inSubmenu && <span>⌫ back</span>}
          <span>Esc cancel</span>
          <span>click disarms</span>
        </div>
      )}
    </div>
  );
};

export default CommandStrip;
