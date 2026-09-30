import * as fs from 'fs';
import * as path from 'path';
import { createHash } from 'node:crypto';
import promptFixture from '../../../tests/fixtures/worker-escalation-prompt.json';
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

  it('preserves the captured exact escalation prompt without a live Go checkout', () => {
    expect(promptFixture.schemaVersion).toBe(1);
    expect(promptFixture.literalCount).toBeGreaterThan(5);
    expect(promptFixture.prompt.length).toBeGreaterThan(900);
    expect(promptFixture.referenceCommit).toMatch(/^[0-9a-f]{40}$/);
    expect(promptFixture.sourceSha256).toMatch(/^[0-9a-f]{64}$/);
    expect(promptFixture.source).toBe('services/hub/cmd/brain/workerescalation.go');
    expect(buildWorkerEscalationContract()).toBe(promptFixture.prompt);
    // Retain provenance verification while the historical file exists. The
    // exact live TS assertion above remains mandatory after its deletion.
    const original = path.join(__dirname, '../../../../..', promptFixture.source);
    if (fs.existsSync(original))
      expect(createHash('sha256').update(fs.readFileSync(original)).digest('hex')).toBe(
        promptFixture.sourceSha256,
      );
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
