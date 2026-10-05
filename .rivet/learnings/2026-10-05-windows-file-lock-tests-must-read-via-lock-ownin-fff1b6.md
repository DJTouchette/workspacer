---
title: Windows file-lock tests must read via lock-owning handle
date: 2026-10-05
confidence: high
suggested_doc: native-embedded-backend
related_paths:
  - services/hub-rs/tests/files.rs
promoted: false
---

# Windows file-lock tests must read via lock-owning handle

## Observation
Native feedback release CI on b785771c failed ordinary_write_waits_for_the_same_os_lock_without_truncating on Windows error33: std::fs::read_to_string opened another handle while fs2 exclusive byte-range lock was held. Windows enforces lock against that separate handle; Unix advisory locks allowed it. Read the base content through the held lock-owning File to verify no truncation without violating the Windows lock.
