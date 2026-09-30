import test from "node:test";
import assert from "node:assert/strict";
import fs from "node:fs";
import os from "node:os";
import path from "node:path";
import { fileURLToPath } from "node:url";
import { spawnSync } from "node:child_process";
const root = path.resolve(path.dirname(fileURLToPath(import.meta.url)), "..");
const relative = "services/hub-rs/assets/persisted-ts";
function fixture(change) {
  const directory = fs.mkdtempSync(
    path.join(os.tmpdir(), "wks-ts-capture-check-"),
  );
  try {
    const manifest = JSON.parse(
      fs.readFileSync(path.join(root, relative, "manifest.json"), "utf8"),
    );
    for (const file of [
      ...Object.keys(manifest.sources),
      ...Object.keys(manifest.artifacts).map((f) => `${relative}/${f}`),
      `${relative}/manifest.json`,
    ]) {
      const output = path.join(directory, file);
      fs.mkdirSync(path.dirname(output), { recursive: true });
      fs.copyFileSync(path.join(root, file), output);
    }
    change?.(directory, manifest);
    return spawnSync(
      process.execPath,
      [path.join(directory, "scripts/capture-persisted-ts.mjs"), "--check"],
      { cwd: directory, encoding: "utf8" },
    );
  } finally {
    fs.rmSync(directory, { recursive: true, force: true });
  }
}
test("source-only check verifies all three actual writer captures without TS runtime or Git", () => {
  const result = fixture();
  assert.equal(result.status, 0, result.stderr);
  assert.match(result.stdout, /Three exact/);
});
test("changed captured bytes cannot pass provenance", () => {
  const result = fixture((dir) =>
    fs.appendFileSync(path.join(dir, relative, "dispatch-history.json"), " "),
  );
  assert.notEqual(result.status, 0);
  assert.match(result.stderr, /artifact drift/);
});
test("changed current writer cannot inherit the captured provenance", () => {
  const result = fixture((dir) =>
    fs.appendFileSync(
      path.join(dir, "apps/desktop/src/main/services/fleetReviewStore.ts"),
      "\n// changed writer\n",
    ),
  );
  assert.notEqual(result.status, 0);
  assert.match(result.stderr, /source provenance drift/);
});
test("missing record family or removed provenance entry fails rather than reducing the population", () => {
  let result = fixture((dir) =>
    fs.rmSync(path.join(dir, relative, "manager-replacements.json")),
  );
  assert.notEqual(result.status, 0);
  result = fixture((dir, manifest) => {
    delete manifest.artifacts["manager-replacements.json"];
    fs.writeFileSync(
      path.join(dir, relative, "manifest.json"),
      JSON.stringify(manifest),
    );
  });
  assert.notEqual(result.status, 0);
  assert.match(result.stderr, /artifact drift/);
});
