#!/usr/bin/env node
/** Capture raw bytes from retained TypeScript stores; check is source-only. */
import fs from "node:fs";
import os from "node:os";
import path from "node:path";
import { fileURLToPath } from "node:url";
import { createHash } from "node:crypto";
import { spawnSync } from "node:child_process";
const root = path.resolve(path.dirname(fileURLToPath(import.meta.url)), "..");
const target = path.join(root, "services/hub-rs/assets/persisted-ts");
const files = [
  "dispatch-history.json",
  "fleet-review.json",
  "manager-replacements.json",
];
const sources = [
  "apps/desktop/src/main/services/dispatchHistoryStore.ts",
  "apps/desktop/src/main/services/fleetReviewStore.ts",
  "apps/desktop/src/main/services/managerReplacementState.ts",
  "apps/desktop/src/main/lib/atomicWriteFile.ts",
  "apps/desktop/src/main/services/persistedWriterCapture.test.ts",
  "scripts/capture-persisted-ts.mjs",
];
const hash = (filename) =>
  createHash("sha256").update(fs.readFileSync(filename)).digest("hex");
const records = (paths, base) =>
  Object.fromEntries(paths.map((p) => [p, hash(path.join(base, p))]));
const mode = process.argv[2];
if (!["--write", "--check"].includes(mode) || process.argv.length !== 3)
  throw Error("usage: node scripts/capture-persisted-ts.mjs --write|--check");
if (mode === "--write") {
  const temp = fs.mkdtempSync(
    path.join(os.tmpdir(), "wks-persisted-ts-capture-"),
  );
  try {
    const output = path.join(temp, "output");
    fs.mkdirSync(output);
    const config = path.join(temp, "vitest.config.mjs");
    fs.writeFileSync(
      config,
      "export default " +
        JSON.stringify({
          root: path.join(root, "apps/desktop"),
          cacheDir: path.join(temp, "cache"),
          test: {
            environment: "node",
            globals: true,
            setupFiles: [
              path.join(root, "apps/desktop/tests/support/tmpdirCleanup.ts"),
            ],
          },
        }),
    );
    const result = spawnSync(
      process.execPath,
      [
        path.join(root, "apps/desktop/node_modules/vitest/vitest.mjs"),
        "run",
        "src/main/services/persistedWriterCapture.test.ts",
        "--cache=false",
        "--config",
        config,
      ],
      {
        cwd: path.join(root, "apps/desktop"),
        env: { ...process.env, WKS_PERSISTED_TS_CAPTURE: output },
        stdio: "inherit",
      },
    );
    if (result.status !== 0)
      throw Error(
        `TypeScript writer capture failed: ${result.status ?? result.error}`,
      );
    if (
      JSON.stringify(fs.readdirSync(output).sort()) !==
      JSON.stringify([...files].sort())
    )
      throw Error("capture did not execute all three writers");
    fs.mkdirSync(target, { recursive: true });
    for (const file of files)
      fs.copyFileSync(path.join(output, file), path.join(target, file));
    const manifest = {
      version: 1,
      origin: "current-retained-typescript-writers",
      capturedAt: new Date().toISOString(),
      description:
        "Exact disk bytes produced by real TypeScript writers. This is not a captured historical release or Rust-generated input.",
      variableProvenance:
        "UUIDs and wall-clock timestamps are real writer output. Review paths are a fresh temporary Git repository/worktree, removed before capture completes. Git identity and commit timestamps are fixed. No byte normalization is applied; --check verifies this captured run and current source hashes, not a fresh byte-identical replay.",
      sources: records(sources, root),
      artifacts: records(files, target),
    };
    fs.writeFileSync(
      path.join(target, "manifest.json"),
      JSON.stringify(manifest, null, 2) + "\n",
    );
  } finally {
    fs.rmSync(temp, { recursive: true, force: true });
  }
} else {
  const manifest = JSON.parse(
    fs.readFileSync(path.join(target, "manifest.json"), "utf8"),
  );
  if (
    manifest.version !== 1 ||
    manifest.origin !== "current-retained-typescript-writers"
  )
    throw Error("unrecognized writer provenance");
  if (
    JSON.stringify(manifest.sources) !== JSON.stringify(records(sources, root))
  )
    throw Error("writer/capture source provenance drift");
  if (
    JSON.stringify(manifest.artifacts) !==
    JSON.stringify(records(files, target))
  )
    throw Error("captured TypeScript artifact drift");
  console.log(
    "Three exact TypeScript-writer captures and source provenance verified.",
  );
}
