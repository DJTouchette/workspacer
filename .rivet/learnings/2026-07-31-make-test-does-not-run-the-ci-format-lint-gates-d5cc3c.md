---
title: make test does not run the CI format/lint gates - run them before pushing
date: 2026-07-31
promoted: true
---

# make test does not run the CI format/lint gates - run them before pushing

## Observation
2026-07-31: pushed a commit that passed 'make test' locally and failed CI on 'npm run format:check'. make test runs ONLY the test suites. CI additionally runs, per stack: desktop = config-defaults codegen drift check + prettier --check + tsc --noEmit (both tsconfig.main.json and src/renderer) + vitest; claudemon and tui = cargo fmt --check + cargo clippy --all-targets -D warnings; hub = gofmt + go vet. Before pushing, the equivalent local sweep is: (cd apps/desktop && npm run format:check && npx tsc --noEmit -p tsconfig.main.json && cd src/renderer && npx tsc --noEmit -p .), (cd apps/tui && cargo fmt -- --check && cargo clippy --all-targets -- -D warnings), same for services/claudemon, and (cd services/hub && gofmt -l ./... && go vet ./...). Note prettier's file globs are 'src/**/*.{ts,tsx}' 'tests/**/*.{ts,tsx}' '*.ts' from apps/desktop - running prettier --write on an explicit file list misses anything you forgot to name, which is exactly how the miss happened.

## Disposition
Folded into .rivet/context/paradigms/hotspots.md (pre-push guideline).
