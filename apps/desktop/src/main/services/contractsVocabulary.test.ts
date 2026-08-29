/**
 * THE CORPUS VOCABULARY GUARD — every fixture in contracts/, every block.
 *
 * TWIN: services/hub/cmd/brain/corpusvocab_test.go. Same checks, same check IDs,
 * same mutation battery, over the same files — and each side asserts the other's
 * source still carries every ID, because the three copies of the ORIGINAL
 * vocabulary test agreed only because a comment said "TWINS:". Deleting one of
 * them was a green run everywhere.
 *
 * What this closes, from the Go twin's header:
 *
 *  - The `vocabulary` block validated tokens, groups and deniedBy for ONE array
 *    (`cases`) of ONE fixture. spawnCwds, methods, checkUse, paramShapes,
 *    projectDirNames and asciiFold got nothing; the other six fixtures got
 *    nothing at all.
 *  - So sessionFilenames.cases still carried a bare `expect: "refuse"` with no
 *    reason — the exact species the deniedBy work existed to kill, one block
 *    away in the same file.
 *
 * The registry lives in each fixture (`vocabulary.blocks`), not in here, and
 * closes both ways: every array-of-objects block must be declared, and every
 * declaration must name a real block.
 */
import { describe, it, expect } from 'vitest';
import fs from 'fs';
import path from 'path';
import { SweepTally, itSweptTheWholeCorpus } from '../../../tests/support/sweepTally';

const CONTRACTS = path.join(__dirname, '../../../../../contracts');
const GO_TWIN = path.join(__dirname, '../../../../../services/hub/cmd/brain/corpusvocab_test.go');

/** The checks this validator performs. Both loaders declare the list. */
const VOCAB_CHECK_IDS = [
  'blocks-declared',
  'blocks-exist',
  'required-fields',
  'verdict-vocabulary',
  'verdict-reason-required',
  'verdict-reason-declared',
  'verdict-reason-forbidden',
  'reason-vocabulary-used',
  'unique-case-names',
  'token-references',
  'unknown-fields',
  'optional-used',
  'block-loaders',
];

/** Fixtures that carry no case blocks. An ALLOW-LIST, not a heuristic: "has no
 *  blocks, so skip it" is how a fixture whose cases were deleted would pass. */
const NO_BLOCK_FIXTURES = new Set([
  'session-schema.json',
  'config-lock.json',
  // wholesale-config-paths.json came OFF this list when it grew `valueCases`.
  'job-preset-power-down.json',
]);

/** How many case-carrying fixtures contracts/ holds today. */
const CONTRACTS_FIXTURE_FLOOR = 10;

type Row = Record<string, unknown>;
interface VerdictDef {
  requires?: string[];
  forbids?: string[];
  reasons?: string;
}
interface BlockSpec {
  why?: string;
  required?: string[];
  /** Closes the field set from the other side. A field NAME the loaders act on
   *  was closed by nothing: renaming `configDirVia` by one character left every
   *  suite green, because encoding/json and JSON.parse both ignore an unknown
   *  key — and that field is uniquely SILENT, since dropping the symlink
   *  indirection flips no verdict, so both cases kept passing while exercising
   *  nothing they claimed to. */
  optional?: string[];
  /** The same, for the sub-keys of a field whose shape is a SCHEMA
   *  (path-containment's `tree`) rather than a data payload (deepmerge's
   *  `target`). Opt-in per block for exactly that reason. */
  nested?: Record<string, string[]>;
  /** The tests that read THIS BLOCK, "<repo-relative file>::<needle>". The
   *  per-fixture loader count is a per-FILE guard, so a block could lose every
   *  loader it had while the file kept the others; cmd/brain/contracts_test.go
   *  resolves these. */
  loaders?: string[];
  verdictField?: string;
  verdicts?: Record<string, VerdictDef>;
}

function tokenRefs(s: string): { names: string[]; unterminated: boolean } {
  const names: string[] = [];
  for (let i = 0; i < s.length;) {
    const j = s.indexOf('${', i);
    if (j < 0) break;
    const end = s.indexOf('}', j + 2);
    if (end < 0) return { names, unterminated: true };
    names.push(s.slice(j + 2, end));
    i = end + 1;
  }
  return { names, unterminated: false };
}

function walkStrings(v: unknown, where: string, visit: (where: string, s: string) => void): void {
  if (typeof v === 'string') visit(where, v);
  else if (Array.isArray(v)) v.forEach((e, i) => walkStrings(e, `${where}[${i}]`, visit));
  else if (v && typeof v === 'object') {
    for (const [k, e] of Object.entries(v as Row)) {
      visit(`${where}.${k} (key)`, k);
      walkStrings(e, `${where}.${k}`, visit);
    }
  }
}

/**
 * The whole check, as a pure function over the decoded document — a function and
 * not a test body on purpose: a test body cannot be fed a deliberately broken
 * fixture, and a check nobody has SEEN fail is indistinguishable from one that
 * cannot. See "the vocabulary guard is falsifiable" below.
 */
export function validateFixtureVocabulary(name: string, doc: Row): string[] {
  const problems: string[] = [];
  const add = (msg: string): void => void problems.push(`${name}: ${msg}`);

  const vocab = doc.vocabulary as Row | undefined;
  if (!vocab) {
    add('no `vocabulary` block — every fixture must declare one [blocks-declared]');
    return problems;
  }
  const rawBlocks = (vocab.blocks as Record<string, BlockSpec> | undefined) ?? {};
  const specs: Record<string, BlockSpec> = {};
  for (const [key, spec] of Object.entries(rawBlocks)) {
    if (key === '_comment') continue;
    specs[key] = spec;
  }
  if (Object.keys(specs).length === 0) {
    add(
      '`vocabulary.blocks` is empty — the registry is what makes this guard non-vacuous [blocks-declared]',
    );
    return problems;
  }

  // Every array-of-objects block, by dotted path.
  const found: Record<string, Row[]> = {};
  const walk = (v: unknown, at: string): void => {
    if (Array.isArray(v)) {
      if (v.length === 0) return;
      if (!v.every((e) => e && typeof e === 'object' && !Array.isArray(e))) return;
      found[at] = v as Row[];
      return;
    }
    if (v && typeof v === 'object') {
      for (const [k, e] of Object.entries(v as Row)) {
        const next = at ? `${at}.${k}` : k;
        if (at === 'vocabulary' || at.startsWith('vocabulary.')) continue;
        walk(e, next);
      }
    }
  };
  walk(doc, '');

  for (const at of Object.keys(found)) {
    if (!specs[at]) {
      add(
        `the block "${at}" holds ${found[at].length} cases and \`vocabulary.blocks\` does not declare it — nothing says what those cases must carry [blocks-declared]`,
      );
    }
  }
  for (const at of Object.keys(specs)) {
    if (!found[at]) {
      add(
        `\`vocabulary.blocks\` declares "${at}" and no such array-of-objects block exists — a declaration for a renamed block validates nothing [blocks-exist]`,
      );
    }
  }

  for (const [at, rows] of Object.entries(found)) {
    const spec = specs[at];
    if (!spec) continue;
    const names: Record<string, number> = {};
    rows.forEach((row, i) => {
      const named = typeof row.name === 'string' && row.name !== '';
      const label = named ? `${at} "${row.name as string}"` : `${at}[${i}]`;
      if (named) names[row.name as string] = (names[row.name as string] ?? 0) + 1;
      // PRESENCE, not truthiness: `"in": ""` and `"expect": null` are real
      // values a case may declare; an omitted field is a case saying nothing.
      for (const field of spec.required ?? []) {
        if (!(field in row)) {
          add(
            `${label} has no "${field}", which vocabulary.blocks says every case in this block must carry [required-fields]`,
          );
        }
      }
      if (!spec.verdictField) return;
      const verdict = row[spec.verdictField];
      const def = (spec.verdicts ?? {})[verdict as string];
      if (!def) {
        add(
          `${label} has ${spec.verdictField} ${JSON.stringify(verdict)}, which is not one of the declared verdicts ${JSON.stringify(Object.keys(spec.verdicts ?? {}).sort())} [verdict-vocabulary]`,
        );
        return;
      }
      for (const field of def.requires ?? []) {
        const v = row[field];
        if (v === undefined || v === null || v === '') {
          add(
            `${label} is a "${verdict as string}" and names no "${field}" — a bare verdict is satisfied by ANY outcome of that class, including one produced for a completely different reason [verdict-reason-required]`,
          );
          continue;
        }
        if (!def.reasons) continue;
        const declared = vocab[def.reasons] as Row | undefined;
        if (!declared || Object.keys(declared).length === 0) {
          add(
            `vocabulary.${def.reasons} is missing or empty, but ${at} points its reasons at it [verdict-reason-declared]`,
          );
          continue;
        }
        if (!(String(v) in declared)) {
          add(
            `${label} claims ${field} "${String(v)}", which vocabulary.${def.reasons} does not declare [verdict-reason-declared]`,
          );
        }
      }
      for (const field of def.forbids ?? []) {
        const v = row[field];
        if (v !== undefined && v !== null && v !== '') {
          add(
            `${label} is a "${verdict as string}" and carries "${field}", which only the other verdict may [verdict-reason-forbidden]`,
          );
        }
      }
    });
    for (const [n, count] of Object.entries(names)) {
      if (count > 1) {
        add(
          `${at} has ${count} cases named "${n}" — a duplicate name hides one of them in every report [unique-case-names]`,
        );
      }
    }

    // FIELD CLOSURE, both directions.
    const allowed = new Set([...(spec.required ?? []), ...(spec.optional ?? [])]);
    const usedOptional = new Set<string>();
    const usedNested: Record<string, Set<string>> = {};
    rows.forEach((row, i) => {
      const named = typeof row.name === 'string' && row.name !== '';
      const label = named ? `${at} "${row.name as string}"` : `${at}[${i}]`;
      for (const [key, val] of Object.entries(row)) {
        if (!allowed.has(key)) {
          add(
            `${label} carries "${key}", which vocabulary.blocks declares neither required nor optional — an undeclared key is ignored by encoding/json AND by JSON.parse, so a one-character typo in a field name silently defangs the case in every loader at once [unknown-fields]`,
          );
          continue;
        }
        usedOptional.add(key);
        const sub = (spec.nested ?? {})[key];
        if (!sub || !val || typeof val !== 'object' || Array.isArray(val)) continue;
        for (const k of Object.keys(val as Row)) {
          if (!sub.includes(k)) {
            add(
              `${label} has ${key}.${k}, which vocabulary.blocks.${at}.nested.${key} does not declare — the same silent-typo defect one level down [unknown-fields]`,
            );
            continue;
          }
          (usedNested[key] ??= new Set()).add(k);
        }
      }
    });
    for (const f of spec.optional ?? []) {
      if (!usedOptional.has(f)) {
        add(
          `vocabulary.blocks.${at} declares the optional field "${f}" and no case carries it — an optional field nothing uses legalizes a typo instead of catching one [optional-used]`,
        );
      }
    }
    for (const [key, sub] of Object.entries(spec.nested ?? {})) {
      for (const f of sub) {
        if (!usedNested[key]?.has(f)) {
          add(
            `vocabulary.blocks.${at}.nested.${key} declares "${f}" and no case carries it [optional-used]`,
          );
        }
      }
    }

    // Every block names the tests that READ it.
    if ((spec.loaders ?? []).length === 0) {
      add(
        `vocabulary.blocks.${at} names no \`loaders\` — the fixture-level loader count is per FILE, so this block could lose every test that reads it while the file kept the others and nothing would go red [block-loaders]`,
      );
    }
    for (const l of spec.loaders ?? []) {
      if (!l.includes('::')) {
        add(
          `vocabulary.blocks.${at} loader "${l}" is not "<repo-relative file>::<needle>" — without a needle the entry only says a file exists, not that anything in it reads this block [block-loaders]`,
        );
      }
    }

    for (const [verdict, def] of Object.entries(spec.verdicts ?? {})) {
      if (!def.reasons) continue;
      const declared = (vocab[def.reasons] as Row | undefined) ?? {};
      const used = new Set<string>();
      for (const row of rows) {
        if (row[spec.verdictField as string] !== verdict) continue;
        for (const field of def.requires ?? []) {
          if (typeof row[field] === 'string') used.add(row[field] as string);
        }
      }
      for (const reason of Object.keys(declared)) {
        if (!used.has(reason)) {
          add(
            `vocabulary.${def.reasons} declares "${reason}" and no ${verdict} case in ${at} names it — an unexercised classification arm is one nothing holds the copies to [reason-vocabulary-used]`,
          );
        }
      }
    }
  }

  const tokens = (vocab.tokens as Row | undefined) ?? {};
  walkStrings(doc, '', (where, s) => {
    const { names, unterminated } = tokenRefs(s);
    if (unterminated) {
      problems.push(
        `${name}: ${where} has a "\${" with no closing "}" in ${JSON.stringify(s)} — the substituter leaves it verbatim, exactly like a mis-spelled name [token-references]`,
      );
    }
    for (const tok of names) {
      if (!(tok in tokens)) {
        problems.push(
          `${name}: ${where} references \${${tok}}, which vocabulary.tokens does not declare — it passes through verbatim [token-references]\n  in: ${JSON.stringify(s)}`,
        );
      }
    }
  });

  return problems.sort();
}

function fixtures(): Record<string, Row> {
  const out: Record<string, Row> = {};
  for (const file of fs.readdirSync(CONTRACTS).sort()) {
    if (!file.endsWith('.json')) continue;
    out[file] = JSON.parse(fs.readFileSync(path.join(CONTRACTS, file), 'utf-8')) as Row;
  }
  return out;
}

describe('contracts/ — every block is declared and closed', () => {
  const all = fixtures();
  const tally = new SweepTally();

  it('contracts/ decodes to fixtures at all', () => {
    expect(Object.keys(all).length).toBeGreaterThan(0);
  });

  for (const [name, doc] of Object.entries(all)) {
    it(name, () => {
      if (NO_BLOCK_FIXTURES.has(name)) {
        expect(
          'vocabulary' in doc,
          `${name} is on the no-blocks exemption list but now declares a vocabulary — take it off the list`,
        ).toBe(false);
        tally.skip('declared exempt: carries no case blocks');
        return;
      }
      tally.ran('other');
      expect(validateFixtureVocabulary(name, doc)).toEqual([]);
    });
  }

  itSweptTheWholeCorpus(tally, 'the contracts vocabulary sweep', CONTRACTS_FIXTURE_FLOOR, {
    allow: 0,
    deny: 0,
  });
});

/**
 * The answer to "all three vocabulary tests are neuterable with every suite
 * green". Every mutation is a real defect this validator claims to catch; the
 * battery runs the SAME function the sweep above runs, over a copy of the real
 * fixture with one thing broken, and requires the matching check ID to fire.
 * Delete a check and this goes red.
 */
describe('the vocabulary guard is falsifiable', () => {
  const base = fixtures()['path-containment-cases.json'];
  const copy = (): Row => JSON.parse(JSON.stringify(base)) as Row;
  const rowsAt = (doc: Row, ...at: string[]): Row[] => {
    let cur: unknown = doc;
    for (const k of at) cur = (cur as Row)[k];
    return cur as Row[];
  };

  const mutations: Array<{ name: string; check: string; mutate: (doc: Row) => void }> = [
    {
      name: 'a case field name is mis-spelled by one character',
      check: 'unknown-fields',
      mutate: (d) => {
        const row = rowsAt(d, 'cases').find((c) => 'configDirVia' in c)!;
        row.configDirVla = row.configDirVia;
        delete row.configDirVia;
      },
    },
    {
      name: 'a tree sub-key is mis-spelled',
      check: 'unknown-fields',
      mutate: (d) => {
        const row = rowsAt(d, 'cases').find((c) => c.tree && 'symlinks' in (c.tree as Row))!;
        const tree = row.tree as Row;
        tree.symLinks = tree.symlinks;
        delete tree.symlinks;
      },
    },
    {
      name: 'an optional field is declared and used by nothing',
      check: 'optional-used',
      mutate: (d) => {
        const spec = ((d.vocabulary as Row).blocks as Row).cases as BlockSpec;
        spec.optional = [...(spec.optional ?? []), 'configDirVla'];
      },
    },
    {
      name: 'a block stops naming the tests that read it',
      check: 'block-loaders',
      mutate: (d) => {
        const spec = ((d.vocabulary as Row).blocks as Row)['sessionFilenames.cases'] as BlockSpec;
        delete spec.loaders;
      },
    },
    {
      name: 'a deny case loses its reason',
      check: 'verdict-reason-required',
      mutate: (d) => {
        const row = rowsAt(d, 'cases').find((c) => c.expect === 'deny')!;
        delete row.deniedBy;
      },
    },
    {
      name: 'a refuse case loses its reason',
      check: 'verdict-reason-required',
      mutate: (d) => {
        const row = rowsAt(d, 'sessionFilenames', 'cases').find((c) => c.expect === 'refuse')!;
        delete row.refusedBy;
      },
    },
    {
      name: 'a deny case claims an undeclared reason',
      check: 'verdict-reason-declared',
      mutate: (d) => {
        rowsAt(d, 'cases').find((c) => c.expect === 'deny')!.deniedBy = 'outside-rooots';
      },
    },
    {
      name: 'a case invents a verdict word',
      check: 'verdict-vocabulary',
      mutate: (d) => void (rowsAt(d, 'cases')[0].expect = 'maybe'),
    },
    {
      name: "an allow case carries a deny's reason",
      check: 'verdict-reason-forbidden',
      mutate: (d) => {
        rowsAt(d, 'cases').find((c) => c.expect === 'allow')!.deniedBy = 'secret';
      },
    },
    {
      name: 'a case drops a required field',
      check: 'required-fields',
      mutate: (d) => void delete rowsAt(d, 'spawnCwds', 'cases')[0].why,
    },
    {
      name: 'a new block appears with no declaration',
      check: 'blocks-declared',
      mutate: (d) => void (d.newIdeas = [{ name: 'x' }]),
    },
    {
      name: 'a declared block is renamed away',
      check: 'blocks-exist',
      mutate: (d) => {
        d.checkUseRenamed = d.checkUse;
        delete d.checkUse;
      },
    },
    {
      name: 'a declared reason stops being exercised',
      check: 'reason-vocabulary-used',
      mutate: (d) => {
        for (const row of rowsAt(d, 'sessionFilenames', 'cases')) {
          if (row.refusedBy === 'escapes-sessions-dir') row.refusedBy = 'not-a-basename';
        }
      },
    },
    {
      name: 'two cases share a name',
      check: 'unique-case-names',
      mutate: (d) => {
        const rows = rowsAt(d, 'cases');
        rows[1].name = rows[0].name;
      },
    },
    {
      name: 'a token is mis-spelled',
      check: 'token-references',
      mutate: (d) => void (rowsAt(d, 'cases')[0].target = '${ROOOT}/x'),
    },
    {
      name: 'a token reference is unterminated',
      check: 'token-references',
      mutate: (d) => void (rowsAt(d, 'cases')[0].target = '${ROOT/x'),
    },
  ];

  for (const m of mutations) {
    it(`${m.name} is caught by ${m.check}`, () => {
      const doc = copy();
      expect(
        validateFixtureVocabulary('mutant', doc),
        'the UNMUTATED copy already reports problems, so this case proves nothing',
      ).toEqual([]);
      m.mutate(doc);
      const problems = validateFixtureVocabulary('mutant', doc);
      expect(
        problems.length,
        `mutation "${m.name}" produced NO complaint — the ${m.check} check does not bite, and every fixture in contracts/ is free to carry that defect`,
      ).toBeGreaterThan(0);
      expect(
        problems.some((p) => p.includes(`[${m.check}]`)),
        `mutation "${m.name}" was caught by something, but not by ${m.check}:\n  ${problems.join('\n  ')}`,
      ).toBe(true);
    });
  }
});

/**
 * The CROSS-LOADER EXISTENCE GUARD, this side. The Go twin runs the same
 * assertion back at this file.
 */
describe('the two corpus-vocabulary loaders are one guard', () => {
  it('the Go twin exists and carries every check this one does', () => {
    expect(fs.existsSync(GO_TWIN), `${GO_TWIN} is gone — this guard has no twin`).toBe(true);
    const src = fs.readFileSync(GO_TWIN, 'utf-8');
    for (const id of VOCAB_CHECK_IDS) {
      expect(
        src.includes(`[${id}]`),
        `the Go validator does not carry the ${id} check — the two have drifted, and a fixture defect this side catches would ship on the other`,
      ).toBe(true);
    }
  });

  it('the three per-loader containment vocabulary tests are all still there', () => {
    const twins: Array<[string, string]> = [
      ['services/hub/cmd/brain/fsguard_test.go', 'TestFixtureVocabularyIsClosed'],
      ['services/hub/internal/bus/policy_test.go', 'TestFixtureVocabularyIsClosed'],
      [
        'apps/desktop/src/main/lib/pathConfinement.test.ts',
        "describe('the fixture vocabulary is closed'",
      ],
    ];
    for (const [rel, needle] of twins) {
      const full = path.join(__dirname, '../../../../..', rel);
      expect(fs.existsSync(full), `${rel} is gone`).toBe(true);
      expect(
        fs.readFileSync(full, 'utf-8').includes(needle),
        `${rel} no longer contains ${needle} — one of the three vocabulary loaders has been removed and the other two would not notice`,
      ).toBe(true);
    }
  });
});
