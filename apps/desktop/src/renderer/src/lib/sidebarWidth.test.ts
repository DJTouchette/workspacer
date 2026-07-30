import { describe, it, expect } from 'vitest';
import {
  clampSidebarWidth,
  resolveSidebarWidth,
  SIDEBAR_DEFAULT_WIDTH,
  SIDEBAR_MIN_WIDTH,
  SIDEBAR_MAX_WIDTH,
} from './sidebarWidth';

describe('clampSidebarWidth', () => {
  it('passes a sane width through, rounded to whole pixels', () => {
    expect(clampSidebarWidth(340, 1920)).toBe(340);
    expect(clampSidebarWidth(340.6, 1920)).toBe(341);
  });

  it('holds the min/max band', () => {
    expect(clampSidebarWidth(40, 1920)).toBe(SIDEBAR_MIN_WIDTH);
    expect(clampSidebarWidth(5000, 3440)).toBe(SIDEBAR_MAX_WIDTH);
  });

  it('caps at a share of the viewport so panes keep the majority', () => {
    // 45% of a 900px window is 405 — narrower than the absolute max.
    expect(clampSidebarWidth(520, 900)).toBe(405);
  });

  it('lets the minimum win when the viewport share would go below it', () => {
    // A phone: 45% of 375 is 169, under the 220 floor. The sidebar overlays the
    // content at those widths, so the floor is the right answer.
    expect(clampSidebarWidth(300, 375)).toBe(SIDEBAR_MIN_WIDTH);
  });

  it('falls back to the absolute max with no viewport (headless / tests)', () => {
    expect(clampSidebarWidth(5000)).toBe(SIDEBAR_MAX_WIDTH);
  });
});

describe('resolveSidebarWidth', () => {
  it('uses the default when nothing is stored', () => {
    expect(resolveSidebarWidth(undefined, 1920)).toBe(SIDEBAR_DEFAULT_WIDTH);
  });

  it('uses a stored width', () => {
    expect(resolveSidebarWidth(352, 1920)).toBe(352);
  });

  it('survives a hand-mangled config.yaml', () => {
    for (const junk of [0, -10, NaN, Infinity, undefined]) {
      expect(resolveSidebarWidth(junk as number, 1920)).toBe(SIDEBAR_DEFAULT_WIDTH);
    }
    // A string slipped past YAML typing is not a number — same fallback.
    expect(resolveSidebarWidth('320' as unknown as number, 1920)).toBe(SIDEBAR_DEFAULT_WIDTH);
  });

  it('re-clamps a width stored on a much wider monitor', () => {
    // 520 saved on an ultrawide, reopened on a 900px window.
    expect(resolveSidebarWidth(520, 900)).toBe(405);
  });
});
