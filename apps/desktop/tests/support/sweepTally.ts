/**
 * The machinery a fixture-driven suite needs to prove it ran.
 *
 * Every corpus loader in this repo has the same failure mode, and it reports a
 * PASS: the cases are enumerated from the fixture, each case carries a host
 * requirement (needsSymlinks, needsUnreadableDir, needsHome, posixOnly), and a
 * host that fails one of them turns the sweep into `it.skip` — a green run over
 * zero executed cases. Vitest prints `3 skipped` in a line nobody reads and
 * exits 0.
 *
 * It has now been found three times in this repo: a Go rootSet oracle whose 8
 * subtests all skipped, a brain method sweep that ran ZERO deny cases whenever
 * TMPDIR sat under $HOME, and the four tests that are the ENTIRE oracle for the
 * sessions store's derived-entry containment.
 *
 * The rule this file enforces: a group's only outcomes are RUN or LOUDLY
 * SKIPPED. Something must assert that the number of cases actually executed is
 * non-zero — with denies counted SEPARATELY from allows, because a sweep that
 * ran only its allow cases proves a guard lets things through and nothing else,
 * and one that ran only denies is satisfied by a guard that refuses everything.
 *
 * The Go twin is services/hub/internal/sweepguard.
 */
import { it, expect } from 'vitest';
import fs from 'fs';
import os from 'os';
import path from 'path';
import { execFileSync } from 'child_process';

/**
 * The lever that makes every host gate in this repo falsifiable on a machine
 * that HAS the privilege: WKS_TEST_NO_SYMLINKS=1 / WKS_TEST_NO_GIT=1 simulate
 * the host that does not. Without it the only way to watch a floor bite is to
 * find such a host — which is precisely why gates without assertions survived
 * here for so long: their failure is invisible everywhere anyone looks.
 *
 * The Go twin is cmd/brain/hostgate_test.go's hostFeatureDisabled.
 */
export function hostFeatureDisabled(feature: string): boolean {
  const v = (process.env[`WKS_TEST_NO_${feature}`] ?? '').trim();
  return v !== '' && v !== '0' && v.toLowerCase() !== 'false';
}

/**
 * Can this host create a symlink? Five test files had a byte-identical copy of
 * this probe, none of which honoured the lever above; keeping one copy is what
 * lets a single env var turn the whole suite into a no-symlink host and prove
 * the floors are real.
 */
export const CAN_SYMLINK: boolean = (() => {
  if (hostFeatureDisabled('SYMLINKS')) return false;
  const probe = fs.mkdtempSync(path.join(os.tmpdir(), 'wks-symprobe-'));
  try {
    fs.symlinkSync(probe, path.join(probe, 'l'));
    return true;
  } catch {
    return false;
  } finally {
    fs.rmSync(probe, { recursive: true, force: true });
  }
})();

/** Is a real `git` binary on PATH? Same lever, same reason. */
export const HAS_GIT: boolean = (() => {
  if (hostFeatureDisabled('GIT')) return false;
  try {
    execFileSync('git', ['--version'], { stdio: 'ignore' });
    return true;
  } catch {
    return false;
  }
})();

const ALLOW_WORDS = new Set(['allow', 'accept', 'ok', 'pass']);
const DENY_WORDS = new Set(['deny', 'refuse', 'reject', 'fail']);

export class SweepTally {
  allow = 0;
  deny = 0;
  other = 0;
  skippedCount = 0;
  private readonly reasons = new Map<string, number>();

  /** Record one EXECUTED case, filed by the fixture's own verdict word.
   *  Call it from inside the test body, past every skip gate — never from the
   *  loop that registers the tests, which counts enumeration and not execution. */
  ran(expect: string): void {
    const word = expect.trim().toLowerCase();
    if (ALLOW_WORDS.has(word)) this.allow++;
    else if (DENY_WORDS.has(word)) this.deny++;
    else this.other++;
  }

  /** Record a case the sweep did not execute, with the reason. The reasons are
   *  what make a floor failure actionable: "0 deny (skipped 41: needsSymlinks
   *  x41)" names the host privilege to fix; a bare zero does not. */
  skip(reason: string): void {
    this.skippedCount++;
    const key = reason.trim() || 'unspecified';
    this.reasons.set(key, (this.reasons.get(key) ?? 0) + 1);
  }

  get executed(): number {
    return this.allow + this.deny + this.other;
  }

  /** Every case the sweep REACHED — executed plus skipped. The host-independent
   *  number: a case that skips for want of symlink privilege still counts, a case
   *  deleted from the fixture does not. */
  get enumerated(): number {
    return this.executed + this.skippedCount;
  }

  private skipSuffix(): string {
    if (this.skippedCount === 0) return '';
    const parts = [...this.reasons.entries()]
      .sort(([a], [b]) => a.localeCompare(b))
      .map(([reason, n]) => `${reason} x${n}`);
    return `; ${this.skippedCount} case(s) skipped: ${parts.join(', ')}`;
  }

  summary(): string {
    return `executed ${this.executed} cases (${this.allow} allow, ${this.deny} deny, ${this.other} other)${this.skipSuffix()}`;
  }

  /** The message a floor failure should carry, or null when the floor is met. */
  problem(what: string, minAllow: number, minDeny: number): string | null {
    const missing: string[] = [];
    if (this.allow < minAllow) missing.push(`${this.allow} allow cases (want >= ${minAllow})`);
    if (this.deny < minDeny) missing.push(`${this.deny} deny cases (want >= ${minDeny})`);
    if (missing.length === 0) return null;
    return `${what} executed ${missing.join(' and ')} — a sweep that ran none of a verdict class asserted nothing about it and is a PASS that guards nothing${this.skipSuffix()}`;
  }
}

/**
 * Registers the floor as a real test, so an empty sweep is RED and named rather
 * than a skip count. It must be declared LAST inside the describe whose cases it
 * counts: vitest runs the tests of one file in declaration order, so by the time
 * this body runs every case above it has either executed or skipped.
 */
export function itSweptBothVerdicts(tally: SweepTally, what: string): void {
  it(`[floor] ${what} executed both allow and deny cases`, () => {
    expect(tally.problem(what, 1, 1), tally.summary()).toBeNull();
  });
}

/**
 * The floor with a RATCHET, and what a fixture-driven sweep must use instead of
 * itSweptBothVerdicts.
 *
 * A floor of one is satisfied by a corpus that lost 98% of its cases: 79
 * executed cases shrinking to 2 (a bad merge, a filter that stopped matching, a
 * fixture edit that drops an array) keeps one allow and one deny and stays green
 * forever. `minEnumerated` is checked against `enumerated`, not `executed`,
 * because execution is host-dependent — a machine that cannot make symlinks
 * legitimately skips half the corpus — while enumeration is a property of the
 * fixture and the loader, identical everywhere. The verdict floors stay small
 * and stay on executed: they answer a different question ("did THIS host prove
 * anything about each class").
 *
 * The Go twin is sweepguard.Tally.RequireCorpus.
 */
export function itSweptTheWholeCorpus(
  tally: SweepTally,
  what: string,
  minEnumerated: number,
  min: { allow?: number; deny?: number } = { allow: 1, deny: 1 },
): void {
  it(`[floor] ${what} swept its whole corpus`, () => {
    expect(
      tally.enumerated,
      `${what} reached ${tally.enumerated} cases but the floor is ${minEnumerated} — the corpus SHRANK. This count is host-independent, so it is not a skip: restore the cases, or lower the floor deliberately and say why. ${tally.summary()}`,
    ).toBeGreaterThanOrEqual(minEnumerated);
    expect(tally.problem(what, min.allow ?? 1, min.deny ?? 1), tally.summary()).toBeNull();
  });
}

/** The floor for a sweep that is deny-only (or accept-only) by construction. */
export function itSweptAtLeast(
  tally: SweepTally,
  what: string,
  min: { allow?: number; deny?: number },
): void {
  it(`[floor] ${what} executed its cases`, () => {
    expect(tally.problem(what, min.allow ?? 0, min.deny ?? 0), tally.summary()).toBeNull();
  });
}

/**
 * The floor for a group of HAND-WRITTEN tests gated on a host privilege
 * (`(CAN_SYMLINK ? it : it.skip)`), where there is no fixture and no verdict
 * column — only "did these run at all". `expected` is how many tests the group
 * declares, so deleting one of them is caught too: a group that quietly shrinks
 * to one test is the same hole arriving more slowly.
 */
/** A counter for a group of host-gated tests. */
export interface GateCounter {
  ran: number;
}

/**
 * `gatedIt(CAN_SYMLINK, counter)` replaces the `(CAN_SYMLINK ? it : it.skip)`
 * idiom and counts the bodies that actually ran, so `itRanEveryGatedTest` below
 * can assert the group was not silently emptied. Wrapping is what keeps the
 * count honest: incrementing at registration time would count skipped tests too,
 * which is the same lie in a different place.
 */
export function gatedIt(
  enabled: boolean,
  counter: GateCounter,
): (name: string, fn: () => void | Promise<void>) => void {
  return (name, fn) => {
    (enabled ? it : it.skip)(name, async () => {
      counter.ran++;
      await fn();
    });
  };
}

export function itRanEveryGatedTest(
  counter: { ran: number },
  what: string,
  expected: number,
): void {
  it(`[floor] ${what} ran all ${expected} of its host-gated tests`, () => {
    expect(
      counter.ran,
      `${what} executed ${counter.ran} of ${expected} tests. These are the ONLY oracle for what they cover, so a host that cannot run them must be RED, not a green run with a skip count nobody reads.`,
    ).toBe(expected);
  });
}
