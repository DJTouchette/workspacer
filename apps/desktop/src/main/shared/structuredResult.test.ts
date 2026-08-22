import { describe, it, expect } from 'vitest';
import {
  RESULT_FENCE,
  RESULT_SCHEMA_MAX,
  buildResultContract,
  checkResultSchema,
  extractResultBlock,
  readStructuredResult,
  validateAgainstSchema,
} from './structuredResult';

/** The dispatch contract the brief names: {commit, filesChanged, checksRun, caveats, followUps}. */
const REPORT_SCHEMA: Record<string, unknown> = {
  type: 'object',
  required: ['commit', 'filesChanged'],
  properties: {
    commit: { type: 'string' },
    filesChanged: { type: 'array', items: { type: 'string' } },
    checksRun: { type: 'array', items: { type: 'string' } },
    caveats: { type: 'string' },
    followUps: { type: 'array', items: { type: 'string' } },
  },
};

describe('checkResultSchema', () => {
  it('accepts a plain schema object', () => {
    expect(checkResultSchema(REPORT_SCHEMA)).toBeNull();
  });

  it('refuses non-objects out loud', () => {
    expect(checkResultSchema('a string')).toMatch(/JSON Schema object/);
    expect(checkResultSchema(['array'])).toMatch(/JSON Schema object/);
    expect(checkResultSchema(null)).toMatch(/JSON Schema object/);
  });

  it('refuses a schema over the cap rather than truncating it', () => {
    const huge = { type: 'object', description: 'x'.repeat(RESULT_SCHEMA_MAX) };
    expect(checkResultSchema(huge)).toMatch(/limit is/);
  });
});

describe('buildResultContract', () => {
  it('names the fence, embeds the schema, and keeps the prose requirement', () => {
    const text = buildResultContract(REPORT_SCHEMA);
    expect(text).toContain(RESULT_FENCE);
    expect(text).toContain('"filesChanged"');
    // The prose half must be demanded explicitly — the whole feature is additive.
    expect(text).toMatch(/summary first/);
    expect(text).toMatch(/FINAL message/);
  });
});

describe('extractResultBlock', () => {
  it('finds a tagged block', () => {
    expect(extractResultBlock('prose\n\n```wks-result\n{"a":1}\n```')).toBe('{"a":1}\n');
  });

  it('falls back to a plain json block', () => {
    expect(extractResultBlock('```json\n{"a":1}\n```')).toBe('{"a":1}\n');
  });

  it('prefers the tagged block over a json example, whatever the order', () => {
    const after = '```json\n{"example":true}\n```\n```wks-result\n{"real":true}\n```';
    const before = '```wks-result\n{"real":true}\n```\n```json\n{"example":true}\n```';
    expect(extractResultBlock(after)).toContain('real');
    expect(extractResultBlock(before)).toContain('real');
  });

  it('takes the LAST tagged block when a worker emitted a draft first', () => {
    const text = '```wks-result\n{"n":1}\n```\nrethinking\n```wks-result\n{"n":2}\n```';
    expect(extractResultBlock(text)).toContain('"n":2');
  });

  it('returns null when there is no block at all', () => {
    expect(extractResultBlock('just prose, no fences')).toBeNull();
  });
});

describe('validateAgainstSchema', () => {
  it('accepts a conforming object', () => {
    expect(
      validateAgainstSchema({ commit: 'abc123', filesChanged: ['a.ts'] }, REPORT_SCHEMA),
    ).toEqual([]);
  });

  it('reports a missing required property by name', () => {
    const errs = validateAgainstSchema({ commit: 'abc' }, REPORT_SCHEMA);
    expect(errs).toHaveLength(1);
    expect(errs[0]).toContain('filesChanged');
    expect(errs[0]).toContain('required');
  });

  it('reports a wrong scalar type and does not cascade into children', () => {
    const errs = validateAgainstSchema({ commit: 42, filesChanged: [1, 2, 3] }, REPORT_SCHEMA);
    expect(errs.some((e) => e.includes('commit') && e.includes('string'))).toBe(true);
    expect(errs.some((e) => e.includes('filesChanged[0]'))).toBe(true);
  });

  it('enforces enum membership', () => {
    const errs = validateAgainstSchema('maybe', { enum: ['yes', 'no'] });
    expect(errs).toHaveLength(1);
    expect(errs[0]).toContain('not one of');
  });

  it('rejects extra keys only when additionalProperties is false', () => {
    const open = { type: 'object', properties: { a: { type: 'string' } } };
    expect(validateAgainstSchema({ a: 'x', b: 1 }, open)).toEqual([]);
    expect(
      validateAgainstSchema({ a: 'x', b: 1 }, { ...open, additionalProperties: false }),
    ).toEqual(['b: unexpected property']);
  });

  it('ignores keywords outside the implemented subset rather than rejecting', () => {
    // minLength/pattern/format are not implemented: an over-rich schema must
    // UNDER-constrain, never reject a report the caller's schema permits.
    const errs = validateAgainstSchema(
      { commit: '' },
      {
        type: 'object',
        properties: { commit: { type: 'string', minLength: 7, pattern: '^[a-f]+$' } },
      },
    );
    expect(errs).toEqual([]);
  });

  it('treats integer and number distinctly', () => {
    expect(validateAgainstSchema(1.5, { type: 'integer' })).toHaveLength(1);
    expect(validateAgainstSchema(1.5, { type: 'number' })).toEqual([]);
    expect(validateAgainstSchema(2, { type: 'integer' })).toEqual([]);
  });

  it('accepts a union type list', () => {
    expect(validateAgainstSchema(null, { type: ['string', 'null'] })).toEqual([]);
    expect(validateAgainstSchema(7, { type: ['string', 'null'] })).toHaveLength(1);
  });
});

describe('readStructuredResult', () => {
  const prose = 'Landed the fix on branch wks/x. Tests green.\n\n';

  it('returns the validated object, pretty-printed', () => {
    const out = readStructuredResult(
      `${prose}\`\`\`${RESULT_FENCE}\n{"commit":"abc123","filesChanged":["a.ts","b.ts"]}\n\`\`\``,
      REPORT_SCHEMA,
    );
    expect(out.error).toBeUndefined();
    expect(JSON.parse(out.json!)).toEqual({ commit: 'abc123', filesChanged: ['a.ts', 'b.ts'] });
    expect(out.json).toContain('\n'); // pretty-printed, readable in the wake
  });

  it('reports a missing block instead of throwing', () => {
    const out = readStructuredResult(prose, REPORT_SCHEMA);
    expect(out.json).toBeUndefined();
    expect(out.error).toMatch(/no `wks-result` block/);
  });

  it('reports unparseable JSON instead of throwing', () => {
    const out = readStructuredResult(`\`\`\`${RESULT_FENCE}\n{not json}\n\`\`\``, REPORT_SCHEMA);
    expect(out.error).toMatch(/not valid JSON/);
  });

  it('reports schema violations with their paths', () => {
    const out = readStructuredResult(
      `\`\`\`${RESULT_FENCE}\n{"commit":"abc"}\n\`\`\``,
      REPORT_SCHEMA,
    );
    expect(out.json).toBeUndefined();
    expect(out.error).toMatch(/does not match the requested schema/);
    expect(out.error).toContain('filesChanged');
  });

  it('never throws on an empty final message', () => {
    expect(() => readStructuredResult('', REPORT_SCHEMA)).not.toThrow();
    expect(readStructuredResult('', REPORT_SCHEMA).error).toBeTruthy();
  });
});
