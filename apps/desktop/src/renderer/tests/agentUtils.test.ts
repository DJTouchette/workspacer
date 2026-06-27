/**
 * Regression test: fmtTokens must not emit a nonsensical "1000k" unit. The
 * magnitude branch checked the raw value against 1_000_000 but the 'k' branch
 * rounded n/1000 with toFixed(0), so a value in [999_500, 1_000_000) rounded up
 * to "1000k" instead of promoting to "1.0M".
 */
import { describe, it, expect } from 'vitest';
import { fmtTokens } from '../src/components/claude/agentUtils';

describe('fmtTokens — magnitude boundary', () => {
  it('promotes values that would round to 1000k up to M', () => {
    expect(fmtTokens(999_950)).toBe('1.0M');
    expect(fmtTokens(999_999)).toBe('1.0M');
    expect(fmtTokens(999_500)).toBe('1.0M');
  });

  it('keeps values that round below 1000k in k', () => {
    expect(fmtTokens(999_499)).toBe('999k');
    expect(fmtTokens(12_345)).toBe('12k');
  });

  it('handles the existing ranges', () => {
    expect(fmtTokens(0)).toBe('');
    expect(fmtTokens(500)).toBe('500');
    expect(fmtTokens(1500)).toBe('1.5k');
    expect(fmtTokens(2_000_000)).toBe('2.0M');
  });
});
