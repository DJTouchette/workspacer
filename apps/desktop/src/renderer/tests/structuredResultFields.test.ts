/**
 * The reading half of the structured-result card. The schema is authored per
 * dispatch, so these tests care mostly about the shapes NOBODY anticipated:
 * an unknown key, a field the schema required and the worker omitted, an empty
 * array, a very long one, and a payload that is not JSON at all.
 */
import { describe, it, expect } from 'vitest';
import {
  buildResultView,
  classifyValue,
  describeField,
  fieldsInSlot,
  formatNumber,
  humanizeKey,
  itemText,
  looksLikeCommit,
  shortCommit,
} from '../src/components/claude/structuredResultFields';

describe('humanizeKey', () => {
  it('turns any casing into words, and never returns nothing', () => {
    expect(humanizeKey('filesChanged')).toBe('files changed');
    expect(humanizeKey('realBugsFound')).toBe('real bugs found');
    expect(humanizeKey('items_skipped')).toBe('items skipped');
    expect(humanizeKey('decision-taken')).toBe('decision taken');
    expect(humanizeKey('caveats')).toBe('caveats');
    expect(humanizeKey('PRUrl')).toBe('pr url');
    expect(humanizeKey('')).toBe('');
  });
});

describe('classifyValue', () => {
  it('reads shape, not key — an unanticipated key still gets a treatment', () => {
    expect(classifyValue('secretsCheck', true)).toBe('boolean');
    expect(classifyValue('bytesAdded', 4211)).toBe('number');
    expect(classifyValue('decisionTaken', 'kept the old parser')).toBe('text');
    expect(classifyValue('realBugsFound', ['a bug with spaces in it'])).toBe('strings');
    expect(classifyValue('touched', ['apps/desktop/src/x.ts'])).toBe('paths');
    expect(classifyValue('rounds', [{ n: 1 }])).toBe('list');
    expect(classifyValue('coverage', { before: 10, after: 12 })).toBe('object');
  });

  it('calls every flavour of nothing `empty`, so it can say which one', () => {
    expect(classifyValue('filesChanged', [])).toBe('empty');
    expect(classifyValue('caveats', '')).toBe('empty');
    expect(classifyValue('commit', null)).toBe('empty');
    expect(classifyValue('notes', {})).toBe('empty');
    expect(classifyValue('ratio', NaN)).toBe('empty');
  });

  it('recognises a SHA by shape and by key', () => {
    expect(looksLikeCommit('commit', 'e124a078')).toBe(true);
    expect(looksLikeCommit('commit', 'v1.2.0-rc1')).toBe(true); // key says so
    expect(looksLikeCommit('mergeSha', '9ae2586')).toBe(true);
    expect(looksLikeCommit('summary', 'deadbeef')).toBe(true); // shape says so
    expect(looksLikeCommit('summary', 'fixed the thing')).toBe(false);
    expect(looksLikeCommit('commit', '')).toBe(false);
    expect(shortCommit('e124a078')).toBe('e124a078');
    expect(shortCommit('e124a0781234567890abcdef1234567890abcdef')).toBe('e124a078');
    expect(shortCommit('release-2026-08-23')).toBe('release-2026-08-23');
  });
});

describe('describeField slots', () => {
  it('puts scannable answers in the summary strip and caveats in their own band', () => {
    expect(describeField('merged', true).slot).toBe('summary');
    expect(describeField('testsFixed', 8).slot).toBe('summary');
    expect(describeField('commit', 'e124a078').slot).toBe('summary');
    expect(describeField('caveats', 'could not run actionlint').slot).toBe('caveats');
    expect(describeField('caveats', '').slot).toBe('caveats');
    expect(describeField('checksRun', ['go vet']).slot).toBe('body');
    expect(describeField('anythingElse', 'prose').slot).toBe('body');
  });
});

describe('buildResultView', () => {
  const real = JSON.stringify({
    commit: 'e124a078',
    merged: true,
    testsFixed: 8,
    filesChanged: ['apps/desktop/tests/e2e/mobileClient.test.ts', '.github/workflows/ci.yml'],
    realBugsFound: ['mobile.html fetchConv(): the conversation-poll self-rearm only fires…'],
    checksRun: ['npx vitest run (main): 2154/2154', 'npm run typecheck: clean'],
    followUps: ['watch the first live CI run'],
    caveats: 'ci.yml was not run through actionlint.',
  });

  it('keeps every field of a real result, conventional or not', () => {
    const view = buildResultView(real);
    expect(view.fallback).toBeUndefined();
    expect(view.fields.map((f) => f.key)).toEqual([
      // Conventional keys lead, in the order they read best…
      'merged',
      'commit',
      'caveats',
      'filesChanged',
      'checksRun',
      'followUps',
      // …then whatever else the dispatch asked for, in the worker's own order.
      'testsFixed',
      'realBugsFound',
    ]);
    expect(fieldsInSlot(view, 'caveats').map((f) => f.key)).toEqual(['caveats']);
    expect(fieldsInSlot(view, 'summary').map((f) => f.key)).toEqual([
      'merged',
      'commit',
      'testsFixed',
    ]);
  });

  it('renders what it has when a schema-required field never arrived', () => {
    // The schema said `commit` and `filesChanged` were required; the worker
    // sent neither. The card is not the validator — it shows the truth it got.
    const view = buildResultView(JSON.stringify({ merged: false, caveats: 'ran out of context' }));
    expect(view.fallback).toBeUndefined();
    expect(view.fields.map((f) => f.key)).toEqual(['merged', 'caveats']);
    expect(view.fields.every((f) => f.value !== undefined)).toBe(true);
  });

  it('says WHY when the payload cannot be read, and hands back the bytes', () => {
    const truncated =
      '{\n  "commit": "e124a078",\n  "checksRun": ["npx vit\n[truncated: 8192 bytes of validated result]';
    const view = buildResultView(truncated);
    expect(view.fields).toEqual([]);
    expect(view.fallback?.reason).toMatch(/truncated/);
    expect(view.fallback?.text).toBe(truncated);

    const garbage = buildResultView('not json at all');
    expect(garbage.fallback?.reason).toMatch(/not valid JSON/);
    expect(garbage.fallback?.text).toBe('not json at all');

    expect(buildResultView('').fallback?.reason).toMatch(/empty/);
    expect(buildResultView(undefined).fallback?.reason).toMatch(/empty/);
  });

  it('still shows a result that is JSON but not an object', () => {
    const view = buildResultView('["a", "b"]');
    expect(view.fallback).toBeUndefined();
    expect(view.fields).toHaveLength(1);
    expect(view.fields[0].kind).toBe('strings');
  });
});

describe('display helpers', () => {
  it('formats counts and list items without ever throwing', () => {
    expect(formatNumber(8)).toBe('8');
    expect(formatNumber(4211)).toBe('4,211');
    expect(formatNumber(1.5)).toBe('1.5');
    expect(itemText('a string')).toBe('a string');
    expect(itemText(3)).toBe('3');
    expect(itemText(null)).toBe('—');
    expect(itemText({ ok: true })).toBe('{"ok":true}');
    const cyclic: Record<string, unknown> = {};
    cyclic.self = cyclic;
    expect(() => itemText(cyclic)).not.toThrow();
  });
});
