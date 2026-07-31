// The saved-session format version is a cross-language contract: the Go brain
// (cmd/brain/stores.go) writes it, this build's reader refuses anything higher.
// contracts/session-schema.json is the shared fixture both sides assert against,
// so bumping one writer without the other fails here or in the Go twin.
import { describe, it, expect } from 'vitest';
import { readFileSync } from 'fs';
import * as path from 'path';
import { SESSION_SCHEMA_VERSION } from './sessionSchema';

describe('session schema version — cross-language contract', () => {
  it('matches the shared fixture', () => {
    const fixture = JSON.parse(
      readFileSync(path.join(__dirname, '../../../../../contracts/session-schema.json'), 'utf-8'),
    );
    expect(SESSION_SCHEMA_VERSION).toBe(fixture.version);
  });
});
