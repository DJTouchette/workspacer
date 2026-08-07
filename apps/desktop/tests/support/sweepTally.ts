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
