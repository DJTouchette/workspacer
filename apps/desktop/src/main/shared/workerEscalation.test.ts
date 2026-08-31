import { describe, expect, it } from 'vitest';
import {
  buildWorkerEscalationContract,
  readWorkerEscalation,
  WORKER_ESCALATION_FENCE,
} from './workerEscalation';

const valid = {
  type: 'worker-escalation',
  status: 'blocked',
  reason: 'Publishing requires authority I do not have.',
  requiredAuthorityOrDecision: 'Confirm whether to publish the release.',
  changed: false,
  nextAction: 'Review the local artifact, then dispatch a publisher with release authority.',
} as const;

describe('worker escalation terminal contract', () => {
  it('advertises the fixed shape and its relationship to optional wks-result', () => {
    const text = buildWorkerEscalationContract();
    expect(text).toContain(`\`\`\`${WORKER_ESCALATION_FENCE}`);
    expect(text).toContain('requiredAuthorityOrDecision');
    expect(text).toContain('"changed": false');
    expect(text).toContain('emit `wks-result` when you complete');
  });

  it('parses and normalizes a valid terminal escalation', () => {
    const out = readWorkerEscalation(
      `I cannot publish safely.\n\n\`\`\`${WORKER_ESCALATION_FENCE}\n${JSON.stringify(valid)}\n\`\`\``,
    );
    expect(out?.value).toEqual(valid);
    expect(out?.json).toContain('"requiredAuthorityOrDecision"');
    expect(out?.error).toBeUndefined();
  });

  it('rejects malformed JSON and wrong shapes instead of accepting them', () => {
    expect(readWorkerEscalation(`\`\`\`${WORKER_ESCALATION_FENCE}\n{nope}\n\`\`\``)?.error).toMatch(
      /not valid JSON/,
    );
    expect(
      readWorkerEscalation(
        `\`\`\`${WORKER_ESCALATION_FENCE}\n${JSON.stringify({ ...valid, changed: 'no' })}\n\`\`\``,
      )?.error,
    ).toBe('changed: expected boolean');
  });

  it('returns null for ordinary prose, including a plain refusal', () => {
    expect(
      readWorkerEscalation('I cannot publish because I only have read-only authority.'),
    ).toBeNull();
  });
});
