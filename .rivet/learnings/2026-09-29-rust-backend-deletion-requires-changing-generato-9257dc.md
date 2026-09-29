---
title: Rust backend deletion requires changing generators and executable fixtures, not just package contents
date: 2026-09-29
promoted: false
---

# Rust backend deletion requires changing generators and executable fixtures, not just package contents

## Observation
Executable cutover audit found default Makefile and desktop npm build/package chains still compiling the four Go commands and desktop-host.cjs, while electron-builder includes those resources on all platforms. Main release defaults native packaging to legacy despite an independent Rust preview workflow. Desktop prebuild generators read or rewrite Go policy/skill files, and Rust capability generation parses Go registries; these must use surviving canonical contracts before deleting Go. Playwright app/mobile fixtures and the cross-language dispatch-chain fixture compile Go at test time, so deleting setup-go is not a port. TUI Rust startup remains an opt-in environment branch, while native --local already selects Rust when the rust-hub feature is enabled; Makefile services-dir flags and default features therefore must change together. Electron shared headless business modules and Node build tooling are not the private companion process and must remain where still used.
