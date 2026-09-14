import { expect, it } from 'vitest';
import fixtures from '../../../../../contracts/intent-report-cases.json';
import { boundIntentReport } from './intentReport';

const prefix = (text: string) => {
  let out = '';
  for (const c of text) {
    if (out.length + c.length > 4000) break;
    out += c;
  }
  return out;
};
function reportCases(input: string, output: string) {
  const cases = [{ input, output }];
  for (const anchor of [3999, 4000, 4001]) {
    // Move every point of each credential (including the prefix and final
    // character) across both sides of the bound. Emoji exercises UTF-16.
    for (let split = 0; split <= input.length; split++) {
      const padding = '😀' + '.'.repeat(anchor - split - 3) + '\n';
      cases.push({ input: padding + input, output: padding + output });
    }
  }
  return cases;
}
it.each(fixtures.cases)(
  'shares redaction and UTF-16 boundary semantics with Rust: %#',
  ({ input, output }) => {
    for (const { input: source, output: expected } of reportCases(input, output)) {
      const actual = boundIntentReport(source);
      expect(actual, source.slice(-180)).toEqual({
        report: prefix(expected),
        redacted: expected !== source,
        truncated: expected.length > 4000,
      });
      expect(actual.report.length).toBeLessThanOrEqual(4000);
      expect(boundIntentReport(actual.report).report).toEqual(actual.report);
    }
  },
);
it('does not split surrogate pairs or otherwise change ordinary formatting', () => {
  for (const n of [3998, 3999, 4000, 4001]) {
    const input = 'x'.repeat(n) + '😀\n';
    expect(boundIntentReport(input).report).toBe(prefix(input));
  }
  expect(boundIntentReport('x'.repeat(3999) + '\ud800').report).toHaveLength(4000);
});
it('bounds after redaction, including expansion and secrets beyond the retained prefix', () => {
  expect(boundIntentReport('password=' + 'x'.repeat(5000))).toEqual({
    report: 'password=[redacted]',
    redacted: true,
    truncated: false,
  });
  expect(boundIntentReport('x'.repeat(4001) + '\nBearer abc')).toEqual({
    report: 'x'.repeat(4000),
    redacted: true,
    truncated: true,
  });
  expect(boundIntentReport('Bearer\u00a0abcdef')).toMatchObject({
    report: '[redacted authorization]',
  });
});
