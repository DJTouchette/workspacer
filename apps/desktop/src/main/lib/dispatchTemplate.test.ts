/**
 * The strict host-side renderer for dispatch templates (lib/dispatchTemplate).
 *
 * The load-bearing behaviour is the HARD ERROR: placeholders are required by
 * default and an unfilled one refuses the render naming the missing param —
 * never a silent default, which is what the renderer's applyTemplate does for
 * its interactive form fields and exactly what a dispatch must not do (a
 * rendered template reads finished, so a silent default dispatches boilerplate
 * without the task-specific reasoning only the manager can write).
 */
import { describe, it, expect } from 'vitest';
import { renderDispatchTemplate, dispatchTemplateParams } from './dispatchTemplate';

describe('renderDispatchTemplate — happy path', () => {
  it('fills required placeholders from params and {{cwd}} from context', () => {
    const out = renderDispatchTemplate(
      'SHIP TASK in {{cwd}}.\n\n{{task}}\n\nReport when done.',
      { task: 'Fix the off-by-one in parse()' },
      { cwd: '/home/u/proj' },
    );
    expect(out).toBe(
      'SHIP TASK in /home/u/proj.\n\nFix the off-by-one in parse()\n\nReport when done.',
    );
  });

  it('tolerates the renderer prompt-var spelling ({{?name}}) with the same params', () => {
    expect(renderDispatchTemplate('do {{?task}}', { task: 'x' })).toBe('do x');
  });

  it('a param value wins over an optional placeholder default', () => {
    const out = renderDispatchTemplate(
      'Deliver: {{delivery:open a PR}}',
      { delivery: 'merge locally on a branch' },
      {},
    );
    expect(out).toBe('Deliver: merge locally on a branch');
  });

  it('an unfilled OPTIONAL placeholder renders its explicit default', () => {
    expect(renderDispatchTemplate('Deliver: {{delivery:open a PR}}', {})).toBe(
      'Deliver: open a PR',
    );
  });

  it('a param named cwd overrides the auto var', () => {
    expect(renderDispatchTemplate('in {{cwd}}', { cwd: '/elsewhere' }, { cwd: '/proj' })).toBe(
      'in /elsewhere',
    );
  });
});

describe('renderDispatchTemplate — the hard rule', () => {
  it('an unfilled REQUIRED placeholder is an error naming the param, never a default', () => {
    expect(() => renderDispatchTemplate('do {{task}} now', {})).toThrow(/\{\{task\}\}/);
  });

  it('a blank value is the same dodge as omitting it', () => {
    expect(() => renderDispatchTemplate('do {{task}}', { task: '   ' })).toThrow(/\{\{task\}\}/);
    expect(() => renderDispatchTemplate('do {{task}}', { task: '' })).toThrow(/\{\{task\}\}/);
  });

  it('names the FIRST missing param of several, and mentions the optional syntax', () => {
    expect(() => renderDispatchTemplate('{{symptom}} vs {{explanationA}}', {})).toThrow(
      /\{\{symptom\}\}.*optional/s,
    );
  });

  it('a param naming no placeholder is refused too (typo guard)', () => {
    expect(() => renderDispatchTemplate('do {{task}}', { task: 'x', tsak: 'y' })).toThrow(
      /unknown templateParams "tsak"/,
    );
  });

  it('{{cwd}} is an auto var, not a required param', () => {
    expect(renderDispatchTemplate('in {{cwd}}', {}, {})).toBe('in ');
  });
});

describe('dispatchTemplateParams', () => {
  it('lists distinct placeholder names in first-seen order, auto vars excluded', () => {
    const params = dispatchTemplateParams(
      'in {{cwd}}: {{task}} then {{delivery:open a PR}} and {{task}} again',
    );
    expect(params.map((p) => p.name)).toEqual(['task', 'delivery']);
    expect(params[0]).toEqual({ name: 'task', required: true });
    expect(params[1]).toEqual({ name: 'delivery', required: false, default: 'open a PR' });
  });

  // The advertised shape and the ENFORCED shape come off the same parser, which
  // is the whole reason library.list may publish it: every param this reports as
  // required is one renderDispatchTemplate refuses to default, and every one it
  // reports optional renders to exactly the advertised `default`.
  it('agrees with renderDispatchTemplate about what is required', () => {
    const text = 'do {{task}} and deliver {{delivery:open a PR}}';
    const params = dispatchTemplateParams(text);
    for (const p of params.filter((x) => x.required)) {
      expect(() => renderDispatchTemplate(text, {})).toThrow(
        new RegExp(`required placeholder \\{\\{${p.name}\\}\\}`),
      );
    }
    const rendered = renderDispatchTemplate(text, { task: 'x' });
    for (const p of params.filter((x) => !x.required)) {
      expect(rendered).toContain(p.default!);
    }
  });

  it('a name first seen bare stays required even when a later token defaults it', () => {
    expect(dispatchTemplateParams('{{task}} … {{task:fallback}}')).toEqual([
      { name: 'task', required: true },
    ]);
  });
});

// ---------------------------------------------------------------------------
// contracts/dispatch-template-params-cases.json — the cross-language corpus.
//
// `library.list` advertises this param list on every kind 'dispatch' item, and
// BOTH providers answer that call: this copy under a desktop client, the Go
// port (cmd/brain/dispatchparams.go) under the default catalog delegation, which
// is every web/mobile/MCP caller. A manager must not learn a different set of
// placeholders depending on which one ran, so the two are pinned to one corpus.
//
// The whitespace cases are the point: this parser spells its token with
// JavaScript's `\s` and trims with String.prototype.trim(), and neither of Go's
// obvious equivalents is that set. See the fixture's own header.
// ---------------------------------------------------------------------------

import * as fs from 'fs';
import * as path from 'path';
import { SweepTally, itSweptTheWholeCorpus } from '../../../tests/support/sweepTally';
import type { DispatchTemplateParam } from './dispatchTemplate';

interface ParamsCase {
  name: string;
  template: string;
  expect: DispatchTemplateParam[];
  why?: string;
}

interface ParamsFixture {
  owners: Record<string, string>;
  cases: ParamsCase[];
}

const PARAMS_OWNER = 'apps/desktop/src/main/lib/dispatchTemplate.ts';

// apps/desktop/src/main/lib/ → five levels below the repo root, where contracts/ sits.
const paramsFixture: ParamsFixture = JSON.parse(
  fs.readFileSync(
    path.join(__dirname, '../../../../../contracts/dispatch-template-params-cases.json'),
    'utf-8',
  ),
);

describe('dispatch template params — cross-language contract', () => {
  it('the fixture loads and names this owner', () => {
    // Renaming this file without updating the fixture must FAIL, not silently
    // stop testing anything.
    expect(
      paramsFixture.owners[PARAMS_OWNER],
      `the fixture must name ${PARAMS_OWNER}`,
    ).toBeDefined();
    expect(paramsFixture.cases.length).toBeGreaterThan(0);
  });

  const tally = new SweepTally();
  for (const c of paramsFixture.cases) {
    it(c.name, () => {
      tally.ran('other');
      // toEqual, not a name-by-name loop: ORDER is part of the contract (the
      // list is what a caller reads top-down to fill a template) and so is the
      // absence of `default` on a required param.
      expect(dispatchTemplateParams(c.template), c.why ?? '').toEqual(c.expect);
    });
  }
  // The twin of cmd/brain/dispatchparams_test.go's dispatchParamsCorpusFloor.
  itSweptTheWholeCorpus(tally, 'the dispatch-template params corpus', 18, { allow: 0, deny: 0 });
});
