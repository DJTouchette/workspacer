import { existsSync, readFileSync } from 'node:fs';
import path from 'node:path';
import { describe, it, expect } from 'vitest';
import generated from './desktopServices.generated';
const root = path.resolve(__dirname, '../../../../..');
const manifest = JSON.parse(
  readFileSync(path.join(root, 'contracts/desktop-service-methods.json'), 'utf8'),
);

describe('desktop service manifest', () => {
  it('retains only the in-process Electron dispatcher, without private lifecycle transport', () => {
    const host = readFileSync(
      path.join(root, 'apps/desktop/src/main/headless/desktopHost.ts'),
      'utf8',
    );
    expect(host).not.toMatch(/case 'internal\./);
    expect(host).not.toMatch(/hostBridge|hostCallId|hostResultId|process\.stdout/);
    for (const retired of ['stdio.ts', 'hostBridge.ts', 'managerReplacement.ts', 'analytics.ts'])
      expect(existsSync(path.join(root, 'apps/desktop/src/main/headless', retired)), retired).toBe(
        false,
      );
    for (const retired of [
      'build-desktop-host.mjs',
      'test-desktop-host.mjs',
      'test-headless-analytics.mjs',
    ])
      expect(existsSync(path.join(root, 'apps/desktop/scripts', retired)), retired).toBe(false);

    const scripts = JSON.parse(
      readFileSync(path.join(root, 'apps/desktop/package.json'), 'utf8'),
    ).scripts;
    expect(scripts['build:desktop-host']).toBeUndefined();
    expect(scripts['test:desktop-host']).toBeUndefined();
    expect(scripts['test:desktop-services']).toBe('node scripts/test-desktop-services.mjs');
  });

  it('matches generated registration and every fixed dispatch implementation', () => {
    expect(generated.ownerMethods).toEqual(manifest.ownerMethods);
    expect(generated.assetMethods).toEqual(manifest.assetMethods);
    const host = readFileSync(
      path.join(root, 'apps/desktop/src/main/headless/desktopHost.ts'),
      'utf8',
    );
    const desktop = readFileSync(
      path.join(root, 'apps/desktop/src/main/services/nativeDesktopServices.ts'),
      'utf8',
    );
    const rust = JSON.parse(
      readFileSync(path.join(root, 'services/hub-rs/assets/brain-capabilities.json'), 'utf8'),
    );
    for (const method of [...manifest.ownerMethods, ...manifest.assetMethods]) {
      expect(
        host.includes(`case '${method}':`) || desktop.includes(`case '${method}':`),
        method,
      ).toBe(true);
    }
    for (const method of [...manifest.ownerMethods, ...manifest.assetMethods])
      expect(rust.full, method).toContain(method);
    for (const row of manifest.authorityCases) {
      expect(row.owner).toBe(row.identity === 'owner');
      expect(row.why.length).toBeGreaterThan(10);
    }
  });
});
