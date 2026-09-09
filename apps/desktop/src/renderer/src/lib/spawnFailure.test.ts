import { it, expect } from 'vitest';
import { spawnFailureMessage } from './spawnFailure';
it('keeps an integration recovery hint through repeated wrapping without exposing raw plugin errors', () => {
  const first = spawnFailureMessage(
    'codex',
    new Error('[WKS_LAUNCH_INTEGRATION] plugin error secret=private'),
  );
  const second = spawnFailureMessage('codex', new Error(first));
  expect(second).toContain('selected launch integration');
  expect(second).toContain('None');
  expect(second).not.toContain('private');
  expect(spawnFailureMessage('codex', new Error('secret=private'))).not.toContain('private');
});
