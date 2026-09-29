//! Header-only image probing: never allocate an attacker-sized pixel buffer.
use anyhow::{Result, bail};
use base64::Engine;
use serde_json::{Value, json};
use std::path::Path;
pub fn mime(path: &Path) -> Option<&'static str> {
    match path.extension()?.to_str()?.to_ascii_lowercase().as_str() {
        "png" => Some("image/png"),
        "jpg" | "jpeg" => Some("image/jpeg"),
        "gif" => Some("image/gif"),
        "webp" => Some("image/webp"),
        "svg" => Some("image/svg+xml"),
        "bmp" => Some("image/bmp"),
        "ico" => Some("image/x-icon"),
        "avif" => Some("image/avif"),
        _ => None,
    }
}
pub fn dimensions(data: &[u8]) -> Option<(u32, u32)> {
    let b = &data[..data.len().min(65536)];
    let be16 = |i| u16::from_be_bytes([b[i], b[i + 1]]) as u32;
    let le16 = |i| u16::from_le_bytes([b[i], b[i + 1]]) as u32;
    let be32 = |i| u32::from_be_bytes(b[i..i + 4].try_into().unwrap());
    let le32 = |i| u32::from_le_bytes(b[i..i + 4].try_into().unwrap());
    if b.len() >= 24 && b[..8] == *b"\x89PNG\r\n\x1a\n" && b[12..16] == *b"IHDR" {
        return Some((be32(16), be32(20)));
    }
    if b.len() >= 10 && (&b[..6] == b"GIF87a" || &b[..6] == b"GIF89a") {
        return Some((le16(6), le16(8)));
    }
    if b.len() >= 26 && &b[..2] == b"BM" {
        return Some((
            (le32(18) as i32).unsigned_abs(),
            (le32(22) as i32).unsigned_abs(),
        ));
    }
    if b.len() >= 30 && &b[..4] == b"RIFF" && &b[8..12] == b"WEBP" {
        match &b[12..16] {
            b"VP8 " => return Some((le16(26) & 0x3fff, le16(28) & 0x3fff)),
            b"VP8L" => {
                let bits = le32(21);
                return Some(((bits & 0x3fff) + 1, ((bits >> 14) & 0x3fff) + 1));
            }
            b"VP8X" => {
                let n = |i| b[i] as u32 | ((b[i + 1] as u32) << 8) | ((b[i + 2] as u32) << 16);
                return Some((n(24) + 1, n(27) + 1));
            }
            _ => (),
        }
    }
    if b.len() >= 4 && &b[..2] == b"\xff\xd8" {
        let mut i = 2;
        while i + 9 < b.len() {
            if b[i] != 0xff {
                i += 1;
                continue;
            }
            let m = b[i + 1];
            if m == 0xd8 || m == 1 || (0xd0..=0xd7).contains(&m) {
                i += 2;
                continue;
            }
            if (0xc0..=0xcf).contains(&m) && ![0xc4, 0xc8, 0xcc].contains(&m) {
                return Some((be16(i + 7), be16(i + 5)));
            }
            let n = be16(i + 2) as usize;
            if n < 2 {
                return None;
            }
            i += 2 + n;
        }
    }
    None
}
pub fn read(path: &Path) -> Result<Value> {
    let mime = mime(path).ok_or_else(|| anyhow::anyhow!("not a previewable browser image"))?;
    let bytes = super::files::bounded_bytes(path, 2 * 1024 * 1024)?;
    let (width, height) = dimensions(&bytes).unwrap_or((0, 0));
    if u64::from(width) * u64::from(height) > 40_000_000 {
        bail!("image exceeds 40,000,000 source pixels");
    }
    Ok(
        json!({"path":path,"dataUrl":format!("data:{mime};base64,{}",base64::engine::general_purpose::STANDARD.encode(&bytes)),"width":width,"height":height,"size":bytes.len()}),
    )
}
