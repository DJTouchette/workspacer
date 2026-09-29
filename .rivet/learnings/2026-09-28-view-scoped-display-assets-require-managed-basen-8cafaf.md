---
title: View-scoped display assets require managed basename confinement and descriptor identity
date: 2026-09-28
suggested_doc: headless-desktop-services
related_paths:
  - services/hub-rs/src/services/ui_assets.rs
  - services/hub-rs/src/services/files.rs
promoted: false
---

# View-scoped display assets require managed basename confinement and descriptor identity

## Observation
ui.fonts/ui.asset are intentionally available to view credentials, while desktop.installUiFont/downloadProjectIcon/readFileBytes/filePickerList remain owner methods. Asset reads require the managed font extension or content-addressed icon filename, canonical containment under that managed directory, regular-file descriptor identity checks, and an actual-byte cap. The authenticated arbitrary-file path helper now uses ambient OS authority; its old workspace/sensitive-path comments are historical. Image previews preserve the Go inline2MiB/browser MIME policy while extending header-only dimension probing to BMP/WebP like the TS reference, preventing pixel bombs without decoding.
