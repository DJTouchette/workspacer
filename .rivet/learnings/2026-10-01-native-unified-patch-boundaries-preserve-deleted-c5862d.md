---
title: Native unified patch boundaries preserve deleted files
date: 2026-10-01
promoted: false
---

# Native unified patch boundaries preserve deleted files

## Observation
Native transcript file summaries must split unified patches at paired ---/+++ headers, keep old path for +++ /dev/null, and normalize namespaced Write tool names before reading content. MultiEdit accepts parent path as well as file_path. These differ from simple +++ b/ splitting and otherwise lose deleted files or attach next headers to previous diffs.
