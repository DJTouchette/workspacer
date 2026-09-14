/** Wire contract: redact the entire report, then retain the longest prefix of at
 * most 4,000 UTF-16 units without splitting a surrogate pair. Keep in lockstep
 * with claudemon/session/intent_report.rs and contracts/intent-report-cases.json.
 * Recognition is deliberately ASCII and conservative (no word-boundary escape).
 */
export const INTENT_REPORT_LIMIT = 4000;
const asciiLower = (s: string) => s.replace(/[A-Z]/g, (c) => c.toLowerCase());
const space = (c: string) => !!c && /\s/u.test(c);
const opaque = (c: string) => !!c && !space(c) && !',;"\'}'.includes(c);
const token = (c: string) => !!c && /[A-Za-z0-9_-]/.test(c);
const authorization = (c: string) => !!c && /[A-Za-z0-9+/_=.-]/.test(c);

export function boundIntentReport(text: string): {
  report: string;
  redacted: boolean;
  truncated: boolean;
} {
  const lower = asciiLower(text);
  const chunks: string[] = [];
  let copied = 0;
  for (let i = 0; i < text.length; i++) {
    let end = i;
    let replacement = '';
    const consume = (start: number, allowed: (c: string) => boolean) => {
      while (start < text.length && allowed(text[start])) start++;
      return start;
    };
    if (text.startsWith('-----BEGIN ', i)) {
      const header = text.indexOf('PRIVATE KEY-----', i + 11);
      if (header >= 0 && !text.slice(i + 11, header).includes('-')) {
        const footer = text.indexOf('-----END ', header + 16);
        const close = footer < 0 ? -1 : text.indexOf('PRIVATE KEY-----', footer + 9);
        end = close < 0 ? text.length : close + 16;
        replacement = '[redacted private key]';
      }
    }
    if (end === i) {
      for (const prefix of ['bearer', 'basic']) {
        const start = i + prefix.length;
        if (lower.startsWith(prefix, i) && space(text[start])) {
          const value = consume(start, space);
          end = consume(value, authorization);
          if (end > value) replacement = '[redacted authorization]';
        }
      }
      for (const prefix of ['sk-', 'ghp_', 'gho_', 'ghu_', 'ghs_', 'ghr_', 'github_pat_']) {
        if (text.startsWith(prefix, i)) {
          const value = i + prefix.length;
          const candidate = consume(value, token);
          if (candidate - value >= (prefix === 'github_pat_' ? 1 : 16)) {
            end = candidate;
            replacement = '[redacted token]';
          }
        }
      }
      for (const key of [
        'password',
        'apikey',
        'api_key',
        'api-key',
        'accesstoken',
        'access_token',
        'access-token',
        'secret',
      ]) {
        if (!lower.startsWith(key, i)) continue;
        let separator = consume(i + key.length, space);
        // Also recognize JSON/quoted keys and values without changing quotes.
        if ('"\''.includes(text[separator] || '\0')) separator = consume(separator + 1, space);
        if (!'=:'.includes(text[separator] || '\0')) continue;
        let value = consume(separator + 1, space);
        const quote = '"\''.includes(text[value] || '\0') ? text[value++] : undefined;
        let candidate = value;
        if (quote) {
          while (candidate < text.length && text[candidate] !== quote) {
            candidate += text[candidate] === '\\' && candidate + 1 < text.length ? 2 : 1;
          }
        } else candidate = consume(value, opaque);
        if (candidate > value && text.slice(value, candidate) !== '[redacted]') {
          end = candidate;
          replacement = text.slice(i, value) + '[redacted]';
        }
      }
      for (const scheme of ['http://', 'https://']) {
        if (!lower.startsWith(scheme, i)) continue;
        const start = i + scheme.length;
        const stop = consume(start, (c) => !space(c) && c !== '/' && c !== '@');
        if (text[stop] === '@' && text.slice(start, stop).includes(':')) {
          end = stop + 1;
          replacement = text.slice(i, start) + '[redacted]@';
        }
      }
    }
    if (replacement) {
      chunks.push(text.slice(copied, i), replacement);
      copied = end;
      i = end - 1;
    }
  }
  chunks.push(text.slice(copied));
  const redactedText = chunks.join('');
  let end = Math.min(redactedText.length, INTENT_REPORT_LIMIT);
  if (
    end < redactedText.length &&
    /[\uD800-\uDBFF]/.test(redactedText[end - 1]) &&
    /[\uDC00-\uDFFF]/.test(redactedText[end])
  )
    end--;
  return {
    report: redactedText.slice(0, end),
    redacted: redactedText !== text,
    truncated: end < redactedText.length,
  };
}
