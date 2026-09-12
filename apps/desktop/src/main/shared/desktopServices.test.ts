import { readFileSync } from 'node:fs';
import path from 'node:path';
import { describe, it, expect } from 'vitest';
import generated from './desktopServices.generated';
const root = path.resolve(__dirname,'../../../../..');
const manifest = JSON.parse(readFileSync(path.join(root,'contracts/desktop-service-methods.json'),'utf8'));

describe('desktop service manifest', () => {
  it('matches generated registration and every fixed dispatch implementation', () => {
    expect(generated.ownerMethods).toEqual(manifest.ownerMethods);
    expect(generated.assetMethods).toEqual(manifest.assetMethods);
    const host = readFileSync(path.join(root,'apps/desktop/src/main/headless/desktopHost.ts'),'utf8');
    const brain = readFileSync(path.join(root,'services/hub/cmd/brain/desktophost.go'),'utf8');
    for (const method of [...manifest.ownerMethods,...manifest.assetMethods]) {
      expect(host.includes(`case '${method}':`) || brain.includes(`method == "${method}"`),method).toBe(true);
    }
    const ownerImplementation = readFileSync(path.join(root,'services/hub/internal/bus/desktop.go'),'utf8');
    for (const method of manifest.ownerMethods) expect(ownerImplementation).toContain(`authenticatedDesktopUser("${method}", cn)`);
    for (const row of manifest.authorityCases) {
      expect(row.owner).toBe(row.identity === 'owner');
      expect(row.why.length).toBeGreaterThan(10);
    }
    expect(ownerImplementation).toContain('cn.authenticatedHost');
  });
});
