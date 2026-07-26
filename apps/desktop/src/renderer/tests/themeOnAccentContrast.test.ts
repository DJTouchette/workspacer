/**
 * Contrast guard for text painted ON the accent fill.
 *
 * The notification bell's unread count used `--wks-accent-text` over
 * `background: var(--wks-accent)`. That token is *accent-coloured text for a
 * normal background* — over the accent itself it is the accent on the accent:
 * under 3:1 in every shipped theme and byte-identical in 12 of them, which
 * renders the count as a bare dot. It read as a platform bug (fine on Windows,
 * a dot on Wayland) only because at 1.3–1.7:1 the platform's font antialiasing
 * decides whether you can make the glyph out at all.
 *
 * So two properties are pinned here:
 *  1. `--wks-bg-raised` — the codebase's on-accent idiom, used by the new-agent
 *     button and now the badge — stays legible on the accent in EVERY theme,
 *     including any theme added later.
 *  2. `accentText` is NOT a viable on-accent colour, so a future author who
 *     reaches for it gets this test's explanation instead of rediscovering the
 *     bug. (An assertion, not a wish: it documents why the idiom exists.)
 */
import { describe, it, expect } from 'vitest';
import * as themes from '../src/themes';

interface ThemeLike {
  name: string;
  accent: string;
  accentText: string;
  bgRaised: string;
}

/** Parse the two colour forms the themes use: #rrggbb and rgb()/rgba(). */
function parseColor(color: string): [number, number, number] | null {
  const hex = /^#([0-9a-fA-F]{6})$/.exec(color.trim());
  if (hex) {
    return [0, 2, 4].map((i) => parseInt(hex[1].substr(i, 2), 16)) as [number, number, number];
  }
  const rgb = /rgba?\(\s*(\d+)[\s,]+(\d+)[\s,]+(\d+)/.exec(color);
  return rgb ? [Number(rgb[1]), Number(rgb[2]), Number(rgb[3])] : null;
}

/** WCAG relative luminance. */
function luminance([r, g, b]: [number, number, number]): number {
  const [lr, lg, lb] = [r, g, b].map((v) => {
    const c = v / 255;
    return c <= 0.03928 ? c / 12.92 : Math.pow((c + 0.055) / 1.055, 2.4);
  });
  return 0.2126 * lr + 0.7152 * lg + 0.0722 * lb;
}

function contrast(a: string, b: string): number | null {
  const ca = parseColor(a);
  const cb = parseColor(b);
  if (!ca || !cb) return null;
  const la = luminance(ca);
  const lb = luminance(cb);
  return (Math.max(la, lb) + 0.05) / (Math.min(la, lb) + 0.05);
}

/** Every exported Theme object, whatever it's named. */
function shippedThemes(): ThemeLike[] {
  return Object.values(themes as Record<string, unknown>).filter(
    (v): v is ThemeLike =>
      !!v &&
      typeof v === 'object' &&
      typeof (v as ThemeLike).name === 'string' &&
      typeof (v as ThemeLike).accent === 'string' &&
      typeof (v as ThemeLike).accentText === 'string' &&
      typeof (v as ThemeLike).bgRaised === 'string',
  );
}

describe('on-accent text contrast', () => {
  it('finds the shipped themes to check', () => {
    // Guard the guard: a refactor that renames the exports must not turn this
    // whole file into a silent no-op.
    expect(shippedThemes().length).toBeGreaterThan(10);
  });

  it('bg-raised stays legible on the accent fill in every theme', () => {
    const failures: string[] = [];
    for (const theme of shippedThemes()) {
      const ratio = contrast(theme.accent, theme.bgRaised);
      if (ratio === null) {
        failures.push(`${theme.name}: unparseable colours`);
      } else if (ratio < 3) {
        failures.push(`${theme.name}: ${ratio.toFixed(2)}:1`);
      }
    }
    expect(
      failures,
      `themes whose accent fill would swallow on-accent text (need >=3:1): ${failures.join(', ')}`,
    ).toEqual([]);
  });

  it('accentText is the wrong token for an accent fill — it is the accent', () => {
    // Not a lament: this is why --wks-bg-raised is the idiom. If a theme ever
    // does give accentText real contrast against accent, that is a deliberate
    // change and this test should be revisited rather than silently relied on.
    const readable = shippedThemes()
      .filter((t) => (contrast(t.accent, t.accentText) ?? 0) >= 3)
      .map((t) => t.name);
    expect(readable).toEqual([]);
  });
});
