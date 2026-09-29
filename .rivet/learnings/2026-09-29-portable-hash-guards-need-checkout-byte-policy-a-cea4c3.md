---
title: Portable hash guards need checkout byte policy and handoff fixtures need canonical host roots
date: 2026-09-29
promoted: false
---

# Portable hash guards need checkout byte policy and handoff fixtures need canonical host roots

## Observation
Windows baseline failure was exactly CRLF conversion: LF fleet-quiescence SHA a7779d... became the CI bd8d35... hash under CRLF. Scoped .gitattributes pins contracts JSON and retained Go source to LF; raw-byte hash checks and mutations remain intact, with a real Git checkout-filter regression. Mac handoff failures came from mocked /var cwd versus real coordinator canonical /private/var cwd; the TS verifier also rejects linked parents. Canonicalize the mock host cwd, not untrusted artifact pointers. Rust artifact inspection additionally must use nonblocking opens/type checks so a FIFO cannot block before regular-file validation.
