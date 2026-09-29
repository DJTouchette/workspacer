// Generates desktop defaults from the canonical JSON embedded by the Rust backend.
// Source: services/hub-rs/assets/config-defaults.json
//
// Run: npm run gen:config-defaults  (wired into prebuild:main).
// A drift test (configService.test.ts) fails if the committed .ts falls out of
// sync with the JSON, so a stale checkout is caught even without a rebuild.

import { readFileSync, writeFileSync } from 'fs';
import { fileURLToPath } from 'url';
import { dirname, join } from 'path';
import prettier from 'prettier';

const here = dirname(fileURLToPath(import.meta.url));
const repoRoot = join(here, '..', '..', '..');
const jsonPath = join(repoRoot, 'services', 'hub-rs', 'assets', 'config-defaults.json');

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
  '// Source of truth: services/hub-rs/assets/config-defaults.json (embedded by the Rust backend).\n' +
  '// Regenerate: npm run gen:config-defaults  (apps/desktop/scripts/gen-config-defaults.mjs).\n' +
  '//\n' +
  '// The main process (configService.ts) and the renderer (hooks/configDefaults.ts)\n' +
  '// both build their defaults from this; drift tests assert each generated copy still\n' +
  '// deep-equals the JSON, so the desktop + Rust backend defaults can never drift.\n\n';

const body = `export const CONFIG_DEFAULTS = ${JSON.stringify(defaults, null, 2)} as const;\n`;

// Format our own output so the generator is self-contained and deterministic:
// the committed files are Prettier-formatted (single quotes, unquoted keys,
// trailing commas per apps/desktop/.prettierrc), but a raw JSON.stringify emits
// double-quoted keys and no trailing commas. Without this step, running the
// generator dirties the tree until a separate `prettier --write` pass runs.
// Resolve Prettier's config from the output path so the repo's .prettierrc wins.
for (const outPath of outPaths) {
  const options = (await prettier.resolveConfig(outPath)) ?? {};
  const formatted = await prettier.format(banner + body, { ...options, parser: 'typescript' });
  writeFileSync(outPath, formatted, 'utf-8');
  console.log(`[gen-config-defaults] wrote ${outPath}`);
}
