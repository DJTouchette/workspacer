---
title: Inline JPEG probing must inspect bounded metadata and marker fill bytes
date: 2026-09-29
confidence: high
suggested_doc: headless-desktop-services
related_paths:
  - services/hub-rs/src/services/image_preview.rs
promoted: false
---

# Inline JPEG probing must inspect bounded metadata and marker fill bytes

## Observation
Rust image_preview dimensions inspected only65536bytes, while a legal single JPEG APP1 metadata segment can place SOF beyond that window inside the2MiB inline limit. RepeatedFF fill bytes before SOF also hid dimensions. Go image/jpeg/reader.go explicitly accepts marker fill bytes and DecodeConfig skips APPn segments until dimensions/SOS. Rust now inspects the full bounded inline prefix and skips FF fill, with synthetic header fixtures proving dimensions and40MP refusal after large metadata; no pixel buffer is decoded.
