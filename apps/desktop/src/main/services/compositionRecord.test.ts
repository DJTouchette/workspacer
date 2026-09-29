import { describe, expect, it } from 'vitest';
import { crossings } from '../../../tests/support/compositionDecisions';
import tiers from '../../../../../services/hub-rs/tests/fixtures/authorization-compositions.json';
import vocabulary from '../../../../../services/hub-rs/assets/hub-vocabulary.json';
// Each proof is exercised by compositionBearings or compositionObjectBearings;
// names cannot be borrowed by a different pair to create a closure exemption.
const PROOFS: Record<string, string[]> = {
  'jobs-owner': ['jobs.upsert', 'jobs.run'],
  'layout-scrub': ['layout.set', 'agents.spawn'],
  'replay-containment': ['replay.open', 'replay.read'],
  'terminal-topics': ['sessions.attachTerminal', 'pty.bytes.*'],
  'saved-session-scrub': ['sessions.save', 'agents.spawn'],
  'saved-layout-scrub': ['layouts.save', 'agents.spawn'],
  'push-endpoint': ['push.subscribe', 'agents.sendMessage'],
};
const SYMBOLS: Record<string, string> = {
  'jobs-owner': 'jobsTrusted',
  'layout-scrub': 'scrubAdoptedSpawnFields',
  'replay-containment': 'resolveInside',
  'terminal-topics': 'EventTopicSpec',
  'saved-session-scrub': 'scrubBootDocumentAgents',
  'saved-layout-scrub': 'scrubBootDocumentAgents',
  'push-endpoint': 'validatePushEndpoint',
};
function check(records = crossings, policies = tiers.pairs): void {
  if (records.length < 8 || records.length !== policies.length)
    throw Error('composition population collapsed');
  const names = new Set<string>(),
    pairs = new Set<string>(),
    shapes = new Set<string>(),
    proofs = new Set<string>();
  for (const record of records) {
    const key = record.A + '\0' + record.B;
    if (names.has(record.Name) || pairs.has(key) || record.A === record.B)
      throw Error('duplicate or self composition');
    names.add(record.Name);
    pairs.add(key);
    shapes.add(record.Shape);
    if (record.Crossing.trim().length < 80) throw Error('missing boundary crossing reason');
    for (const half of [record.A, record.B])
      if (
        !vocabulary.methods.includes(half) &&
        !vocabulary.topics.some(
          (row) => row.Pattern === half && row.Disposition === 'guarded-by-capability',
        )
      )
        throw Error('unknown composition half');
    const policy = policies.filter((row) => row.a === record.A && row.b === record.B);
    if (policy.length !== 1) throw Error('unbound policy');
    const p = policy[0],
      closed = !!record.ClosedBy;
    if (closed) {
      if (
        !p.closedProof ||
        p.acceptedIn.length ||
        JSON.stringify(PROOFS[p.closedProof]) !== JSON.stringify([record.A, record.B])
      )
        throw Error('invalid pair-bound closure');
      if (!record.ClosedBy!.includes(SYMBOLS[p.closedProof]))
        throw Error('unnamed closure mechanism');
      proofs.add(p.closedProof);
    } else if (
      p.closedProof ||
      p.acceptedIn.length === 0 ||
      p.acceptedIn.some((tier) => !['view', 'triage'].includes(tier))
    )
      throw Error('unaccepted open composition');
  }
  if (
    JSON.stringify([...shapes].sort()) !==
    JSON.stringify(['ShapeWidenThenUse', 'ShapeWriteThenInterpret'])
  )
    throw Error('composition shape coverage');
  if (JSON.stringify([...proofs].sort()) !== JSON.stringify(Object.keys(PROOFS).sort()))
    throw Error('unused closure exemption');
}
describe('composition crossing registry', () => {
  it('retains both crossing shapes and exact closed/accepted pair bindings', () =>
    expect(() => check()).not.toThrow());
  it('rejects false closure borrowing, unclassified halves, stale exemptions and missing crossing decisions', () => {
    const mutate = (f: (rows: typeof crossings) => void): void => {
      const rows = structuredClone(crossings);
      f(rows);
      expect(() => check(rows)).toThrow();
    };
    mutate((rows) => {
      rows[0].Crossing = 'safe';
    });
    mutate((rows) => {
      rows[0].B = rows[0].A;
    });
    mutate((rows) => {
      rows[0].A = 'invented.actor';
    });
    mutate((rows) => {
      rows[0].Shape = 'ShapeWidenThenUse';
    });
    mutate((rows) => {
      rows[7].ClosedBy = 'jobsTrusted';
    });
    mutate((rows) => {
      rows[0].ClosedBy = 'A vaguely named safety check';
    });
    const policies = structuredClone(tiers.pairs);
    policies[0].closedProof = 'push-endpoint';
    expect(() => check(crossings, policies)).toThrow('pair-bound');
  });
});
