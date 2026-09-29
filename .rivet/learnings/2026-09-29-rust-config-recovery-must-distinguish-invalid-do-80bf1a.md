---
title: Rust config recovery must distinguish invalid document shape from empty YAML
date: 2026-09-29
promoted: false
---

# Rust config recovery must distinguish invalid document shape from empty YAML

## Observation
Go yaml.Unmarshal into map rejects scalar and array configuration documents, producing one recoverable .broken copy; serde_yaml into Value accepts those shapes. Rust refresh must therefore backup syntax failures OR non-null non-object values, while null/empty/comment-only documents remain protected without a backup. The config integration regression covers repeated saves with millisecond-separated backup names, unchanged original bytes, and repair clearing persistence blocking.
