#!/usr/bin/env node
// The root VERSION file is the release version: native packaging, the release
// workflow and build stamps all read it. apps/desktop/package.json still
// carries a copy, because the browser app built from it (served at /app by
// the server bundle) reports that version. This fails when the two disagree.
//
//   node scripts/check-version.mjs           check, exit 1 on a mismatch
//   node scripts/check-version.mjs --print   check, then print the version
import fs from 'node:fs';
import path from 'node:path';
import { fileURLToPath } from 'node:url';

const root = path.resolve(path.dirname(fileURLToPath(import.meta.url)), '..');
const fail = message => {
  console.error(`check-version: ${message}`);
  process.exit(1);
};

let raw;
try {
  raw = fs.readFileSync(path.join(root, 'VERSION'), 'utf8');
} catch (error) {
  fail(`cannot read VERSION: ${error.message}`);
}
const version = raw.trim();
if (raw !== `${version}\n` && raw !== `${version}\r\n`) fail('VERSION must be a single line holding only the version');
if (!/^\d+\.\d+\.\d+(?:-[a-zA-Z0-9.-]+)?$/.test(version)) fail(`VERSION is not a release version: ${JSON.stringify(version)}`);

const desktop = JSON.parse(fs.readFileSync(path.join(root, 'apps/desktop/package.json'), 'utf8')).version;
if (desktop !== version) {
  fail(`VERSION is ${version} but apps/desktop/package.json is ${desktop}. ` +
    'Bump both: edit VERSION, then run `npm version --no-git-tag-version <version>` in apps/desktop.');
}
if (process.argv.includes('--print')) console.log(version);
else console.log(`check-version: ${version}`);
