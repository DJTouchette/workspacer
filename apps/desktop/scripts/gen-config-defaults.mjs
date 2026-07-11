// Generates apps/desktop/src/main/services/configDefaults.generated.ts from the
// canonical default-config JSON that the headless brain (Go) embeds:
//   services/hub/cmd/brain/config_defaults.json
//
// This is the "single source of truth" seam: the brain go:embeds the JSON, and
// the desktop consumes it through the generated TS module below, so the two
// runtimes can't drift the way the old hand-transcribed Go copy did. The plain
// node runtime (no tsx) only needs to read JSON and write a .ts string.
//
// Run: npm run gen:config-defaults  (wired into prebuild:main).
// A drift test (configService.test.ts) fails if the committed .ts falls out of
// sync with the JSON, so a stale checkout is caught even without a rebuild.

import { readFileSync, writeFileSync } from 'fs';
import { fileURLToPath } from 'url';
import { dirname, join } from 'path';

const here = dirname(fileURLToPath(import.meta.url));
const repoRoot = join(here, '..', '..', '..');
const jsonPath = join(repoRoot, 'services', 'hub', 'cmd', 'brain', 'config_defaults.json');

// Two consumers, one source: the main process (Node) and the renderer (browser)
// build graphs don't share modules, and the renderer tsconfig doesn't include
// src/main, so each gets its own generated leaf with identical content.
const outPaths = [
  join(here, '..', 'src', 'main', 'services', 'configDefaults.generated.ts'),
  join(here, '..', 'src', 'renderer', 'src', 'hooks', 'configDefaults.generated.ts'),
];

const raw = readFileSync(jsonPath, 'utf-8');
// Parse + re-stringify so the emitted object is normalized (and we fail loudly
// here rather than shipping invalid JSON into the .ts).
const defaults = JSON.parse(raw);

const banner =
  '// GENERATED FILE — do not edit by hand.\n' +
  '// Source of truth: services/hub/cmd/brain/config_defaults.json (the brain go:embeds it).\n' +
  '// Regenerate: npm run gen:config-defaults  (apps/desktop/scripts/gen-config-defaults.mjs).\n' +
  '//\n' +
  '// The main process (configService.ts) and the renderer (hooks/configDefaults.ts)\n' +
  '// both build their defaults from this; drift tests assert each generated copy still\n' +
  '// deep-equals the JSON, so the desktop + brain defaults can never drift.\n\n';

const body = `export const CONFIG_DEFAULTS = ${JSON.stringify(defaults, null, 2)} as const;\n`;

for (const outPath of outPaths) {
  writeFileSync(outPath, banner + body, 'utf-8');
  console.log(`[gen-config-defaults] wrote ${outPath}`);
}
