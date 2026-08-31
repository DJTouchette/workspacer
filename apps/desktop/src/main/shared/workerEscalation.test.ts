import * as fs from 'fs';
import * as path from 'path';
import { describe, expect, it } from 'vitest';
import {
  buildWorkerEscalationContract,
  isFleetDispatchedWorker,
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
  it('is scoped by authoritative parent/manager metadata', () => {
    expect(isFleetDispatchedWorker({ parentSessionId: 'manager-1' })).toBe(true);
    expect(isFleetDispatchedWorker({ parentSessionId: '  ' })).toBe(false);
    expect(isFleetDispatchedWorker({})).toBe(false);
    expect(isFleetDispatchedWorker({ parentSessionId: 'manager-1', manager: true })).toBe(false);
  });

  it('is byte-for-byte identical to the Go headless contract', () => {
    const go = fs.readFileSync(
      path.join(__dirname, '../../../../../services/hub/cmd/brain/workerescalation.go'),
      'utf8',
    );
    const expression = /workerEscalationContract\s*=([\s\S]*?)\n\)/.exec(go)?.[1];
    expect(
      expression,
      'Go workerEscalationContract declaration moved or changed shape',
    ).toBeTruthy();
    const literals = [...expression!.matchAll(/"(?:\\.|[^"\\])*"/g)].map((match) =>
      JSON.parse(match[0]),
    );
    expect(literals.length).toBeGreaterThan(5);
    expect(literals.join('')).toBe(buildWorkerEscalationContract());
  });

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
