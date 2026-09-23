---
title: recon symbols caps at 30 results, ordered by path
date: 2026-08-26
promoted: false
---

# recon symbols caps at 30 results, ordered by path

## Observation
Verified on rivet v0.20.0: 'rivet recon symbols <query>' returns at most 30 symbols, and results are ordered by file path rather than by relevance score. Because apps/desktop/** sorts first, any broad query (Config, Session, Node, Pane) fills the entire 30-result budget with TypeScript and never reaches services/hub (Go) or apps/tui (Rust) — even though both ARE fully indexed. Confirmed: 'symbols Config' = 30/30 .ts hits, but 'symbols file:services/hub/cmd/brain/config.go' lists configService/newConfigService fine, and the narrow query 'symbols configService' correctly returns TS + Go together. Consequence: CLAUDE.md's advice that 'symbols Config resolves across all three languages in a single call' no longer holds for common words. Use a distinctive query (configService, not Config) when you want the cross-stack sweep, or fall back to recon.grep --type definition.
