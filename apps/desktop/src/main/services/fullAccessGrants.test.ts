import { describe, expect, it } from 'vitest';
import {
  desiredSessionGrants,
  managerFullAccessFromConfig,
  reconcileFullAccessGrants,
  startFullAccessGrantSync,
} from './fullAccessGrants';

describe('retired full-access grant reconciliation', () => {
  it('is inert regardless of legacy config or announcement requests', () => {
    expect(managerFullAccessFromConfig()).toBe(false);
    expect(desiredSessionGrants()).toEqual({ manager: false });
    expect(reconcileFullAccessGrants()).toBe(0);
    expect(reconcileFullAccessGrants(true)).toBe(0);
    expect(startFullAccessGrantSync()).toBeUndefined();
  });
});
