import { describe, expect, it } from 'vitest';
import fs from 'fs';
import path from 'path';
import {
  checkSweepSources,
  checkRustHostTests,
  requiredRustHostTests,
} from '../../../tests/support/sweepSourceGuard';

function sources(directory: string, out = new Map<string, string>()) {
  for (const entry of fs.readdirSync(directory, { withFileTypes: true })) {
    const file = path.join(directory, entry.name);
    if (entry.isDirectory()) sources(file, out);
    else if (entry.name.endsWith('.test.ts')) out.set(file, fs.readFileSync(file, 'utf8'));
  }
  return out;
}
const ROOT = path.resolve(__dirname, '..');
describe('fixture sweep source invariants', () => {
  it('retains the executable Rust host cases and rejects removal or disabling one', () => {
    const root = path.resolve(ROOT, '../../../..');
    const files = new Map(
      Object.keys(requiredRustHostTests).map((file) => [
        file,
        fs.readFileSync(path.join(root, 'services/hub-rs', file), 'utf8'),
      ]),
    );
    expect(checkRustHostTests(files)).toEqual([]);
    const file = 'tests/library.rs';
    const name = requiredRustHostTests[file][0];
    for (const replacement of [
      `fn removed_${name}(`,
      `#[ignore]\nfn ${name}(`,
      `#[cfg(unix)]\nfn ${name}(`,
    ]) {
      const changed = new Map(files);
      changed.set(file, files.get(file)!.replace(`fn ${name}(`, replacement));
      expect(checkRustHostTests(changed).some((error) => error.includes(name))).toBe(true);
    }
    const hidden = new Map(files);
    hidden.set(file, `#[cfg(unix)] mod hidden {${files.get(file)}}`);
    expect(checkRustHostTests(hidden).some((error) => error.includes(name))).toBe(true);
  });
  it('checks all current desktop sweeps, host gates and newly discovered test files', () => {
    const result = checkSweepSources(sources(ROOT));
    expect(result.errors).toEqual([]);
    // Original sweepmeta's 30 was a mixed Go+TS floor. Pin the entire unchanged
    // desktop population separately so retiring Go cannot hide lost TS guards.
    expect(result.counters).toBeGreaterThanOrEqual(29);
    expect(result.hostReferences).toBeGreaterThanOrEqual(15);
  }, 30_000); // Parses the complete live test tree; allow concurrent CI workers.
  it('rejects missing observations, sibling-scope decoys and registration increments', () => {
    const good = `import {it,describe} from 'vitest';
      import {SweepTally,itSweptBothVerdicts,gatedIt,itRanEveryGatedTest,CAN_LINK} from './support';
      describe('x',()=>{const t=new SweepTally(); const gate={ran:0};
      const itLinks=gatedIt(CAN_LINK,gate);
      for(const c of cases){itLinks('case',()=>{t.ran('allow')});}
      itSweptBothVerdicts(t,'sweep');itRanEveryGatedTest(gate,'links',1);});`;
    const check = (source: string) =>
      checkSweepSources(new Map([['fixture.test.ts', source]])).errors;
    expect(check(good)).toEqual([]);
    for (const mutant of [
      good.replace("itSweptBothVerdicts(t,'sweep');", ''),
      good.replace("itRanEveryGatedTest(gate,'links',1);", ''),
      good.replace(
        "itLinks('case',()=>{t.ran('allow')});",
        "t.ran('allow');itLinks('case',()=>{});",
      ),
      good.replace('gatedIt(CAN_LINK,gate)', '(CAN_LINK ? it : it.skip)'),
      good.replace("t.ran('allow')", "try {setup()} catch {return;} t.ran('allow')"),
      good.replace(
        "itSweptBothVerdicts(t,'sweep');",
        "describe('sibling',()=>{const t=new SweepTally();itSweptBothVerdicts(t,'sweep');});",
      ),
    ])
      expect(check(mutant).length, mutant).toBeGreaterThan(0);
    expect(check(`// const t=new SweepTally();\nconst example='catch {return} CAN_LINK';`)).toEqual(
      [],
    );
  });
  it('detects removal of a live fixture floor and a new unobserved source file', () => {
    const file = path.join(ROOT, 'lib/spawnCwd.test.ts');
    const files = new Map([[file, fs.readFileSync(file, 'utf8')]]);
    const floor = "itSweptTheWholeCorpus(tally, 'the spawnCwds block', 14, { allow: 0, deny: 0 });";
    expect(files.get(file)).toContain(floor);
    files.set(file, files.get(file)!.replace(floor, '// removed execution floor'));
    expect(
      checkSweepSources(files).errors.some(
        (error) => error.includes('spawnCwd.test.ts') && error.includes('no execution floor'),
      ),
    ).toBe(true);
    const added = path.join(ROOT, 'services/new-sweep.test.ts');
    files.set(added, "import {SweepTally} from './support'; const missed = new SweepTally();");
    expect(
      checkSweepSources(files).errors.some((error) => error.includes('new-sweep.test.ts')),
    ).toBe(true);
  });
});
