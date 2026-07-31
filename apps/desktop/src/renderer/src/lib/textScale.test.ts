import { describe, it, expect } from 'vitest';
import {
  clampTextScale,
  textScaleToRootFontSize,
  MIN_TEXT_SCALE,
  MAX_TEXT_SCALE,
  TEXT_SCALE_STEP,
  DEFAULT_TEXT_SCALE,
} from './textScale';

describe('clampTextScale', () => {
  it('passes an in-range scale through', () => {
    expect(clampTextScale(1.0)).toBe(1.0);
    expect(clampTextScale(1.25)).toBe(1.25);
  });

  it('holds the bounds, inclusive', () => {
    expect(clampTextScale(MIN_TEXT_SCALE)).toBe(MIN_TEXT_SCALE);
    expect(clampTextScale(MAX_TEXT_SCALE)).toBe(MAX_TEXT_SCALE);
    expect(clampTextScale(0.1)).toBe(MIN_TEXT_SCALE);
    expect(clampTextScale(99)).toBe(MAX_TEXT_SCALE);
    expect(clampTextScale(-5)).toBe(MIN_TEXT_SCALE);
  });

  it('quantises to whole percents', () => {
    expect(clampTextScale(1.234)).toBe(1.23);
    expect(clampTextScale(1.235)).toBe(1.24);
  });

  it('survives a junk value rather than poisoning the config', () => {
    expect(clampTextScale(NaN)).toBe(DEFAULT_TEXT_SCALE);
    expect(clampTextScale(Infinity)).toBe(DEFAULT_TEXT_SCALE);
    expect(clampTextScale(-Infinity)).toBe(DEFAULT_TEXT_SCALE);
  });

  it('returns exactly to 1.0 after stepping up and back down', () => {
    // 0.05 is not representable in binary floating point: without the rounding
    // this round-trips to 0.9999999999999999, which is !== the default and so
    // would write a config on every reset.
    const up = clampTextScale(DEFAULT_TEXT_SCALE + TEXT_SCALE_STEP);
    const down = clampTextScale(up - TEXT_SCALE_STEP);
    expect(down).toBe(DEFAULT_TEXT_SCALE);
  });

  it('walks the whole range in steps without drifting off the grid', () => {
    let scale = MIN_TEXT_SCALE;
    const seen: number[] = [];
    while (scale < MAX_TEXT_SCALE) {
      scale = clampTextScale(scale + TEXT_SCALE_STEP);
      seen.push(scale);
    }
    // Every stop is a whole number of percent, and the walk terminates at the
    // ceiling rather than oscillating just below it.
    for (const s of seen) expect(Math.round(s * 100)).toBeCloseTo(s * 100, 10);
    expect(seen[seen.length - 1]).toBe(MAX_TEXT_SCALE);
  });
});

describe('textScaleToRootFontSize', () => {
  it('is empty at the default, so the stylesheet keeps control', () => {
    // An explicit '100%' would override the user's own browser/OS font size.
    expect(textScaleToRootFontSize(DEFAULT_TEXT_SCALE)).toBe('');
  });

  it('renders a percentage anywhere else', () => {
    expect(textScaleToRootFontSize(1.25)).toBe('125.0%');
    expect(textScaleToRootFontSize(MIN_TEXT_SCALE)).toBe('80.0%');
    expect(textScaleToRootFontSize(MAX_TEXT_SCALE)).toBe('150.0%');
  });

  it('keeps one decimal place for the odd half-percent', () => {
    expect(textScaleToRootFontSize(1.005)).toBe('100.5%');
  });
});
