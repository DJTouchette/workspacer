//! Real image input for managed sends: the files a client uploaded through
//! the hub's `files.upload`, attached to `agents.sendMessage` as `images`.
//!
//! Only an upload can be attached. A client names a path, and this module
//! accepts it only when it is a regular, non-symlinked file the upload
//! landing pad wrote: directly inside [`upload_dir`], under the hub-composed
//! `m-<millis>-<8 hex>.<ext>` basename, with an image extension. Anything
//! else is refused, so `images` can never read an arbitrary file on the host.
//!
//! The bytes are then checked as an image (PNG, JPEG, GIF or WebP by their
//! magic numbers, never by the extension alone) and capped: at most
//! [`MAX_IMAGES`] per message and [`MAX_IMAGE_BYTES`] per image — the Claude
//! API's 5 MB ceiling applies to the base64 text, so the file itself may be
//! three quarters of that. Nothing is downscaled; an oversize image is refused
//! with a message saying so.
//!
//! Drivers that can take images structurally (Claude stream, Codex) get them
//! as [`ImageInput`]s; every other transport gets [`with_markers`]'
//! `[Image: <path>]` lines instead, the text the agent then reads the file by.
use base64::Engine;
use serde_json::{json, Value};
use std::path::{Path, PathBuf};

/// Most images one message may carry.
pub const MAX_IMAGES: usize = 8;
/// Largest image file accepted: 5 MiB of base64 (the Claude API's per-image
/// limit is on the encoded data) is 3.75 MiB of file.
pub const MAX_IMAGE_BYTES: u64 = 5 * 1024 * 1024 / 4 * 3;
/// Extensions an attachable upload may carry (`files.upload` also accepts
/// `pdf`, which is not an image and keeps the text marker).
const EXTENSIONS: [&str; 5] = ["png", "jpg", "jpeg", "gif", "webp"];

/// One validated image, ready for a provider. `path` is the upload's own
/// spelling (what Codex's `localImage` reads); `data` is its base64 for an
/// inline Claude image block.
#[derive(Clone, PartialEq, Eq)]
pub struct ImageInput {
    pub path: PathBuf,
    pub media_type: &'static str,
    pub data: String,
}

impl std::fmt::Debug for ImageInput {
    // Never print megabytes of base64 into a log line.
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ImageInput")
            .field("path", &self.path)
            .field("media_type", &self.media_type)
            .field("bytes", &(self.data.len() / 4 * 3))
            .finish()
    }
}

/// Where `files.upload` writes: one private directory per OS user under the
/// system temp directory. The hub's upload handler uses this same function.
pub fn upload_dir() -> PathBuf {
    #[cfg(unix)]
    let uid = nix::unistd::getuid().as_raw().to_string();
    #[cfg(not(unix))]
    let uid = "-1";
    std::env::temp_dir().join(format!("workspacer-uploads-{uid}"))
}

/// The hub-composed upload basename: `m-<digits>-<8 lowercase hex>.<ext>`.
fn upload_name(name: &str) -> bool {
    let Some(rest) = name.strip_prefix("m-") else {
        return false;
    };
    let Some((stem, ext)) = rest.rsplit_once('.') else {
        return false;
    };
    let Some((millis, tag)) = stem.split_once('-') else {
        return false;
    };
    !millis.is_empty()
        && millis.len() <= 20
        && millis.bytes().all(|b| b.is_ascii_digit())
        && tag.len() == 8
        && tag
            .bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
        && EXTENSIONS.contains(&ext)
}

/// Accept `path` only as an upload directly inside `dir`, without reading
/// it. Returns the path joined onto `dir` (the caller's own spelling of the
/// upload, never a symlink target).
pub fn confine_in(path: &str, dir: &Path) -> Result<PathBuf, String> {
    let refuse = || format!("image {path:?} is not a file uploaded with files.upload");
    let given = Path::new(path);
    if !given.is_absolute() {
        return Err(refuse());
    }
    let name = given
        .file_name()
        .and_then(|n| n.to_str())
        .filter(|n| upload_name(n))
        .ok_or_else(refuse)?;
    let parent = given.parent().ok_or_else(refuse)?;
    let (Ok(parent), Ok(root)) = (parent.canonicalize(), dir.canonicalize()) else {
        return Err(refuse());
    };
    if parent != root {
        return Err(refuse());
    }
    let resolved = dir.join(name);
    match std::fs::symlink_metadata(&resolved) {
        Ok(meta) if meta.file_type().is_file() => {}
        _ => return Err(refuse()),
    }
    Ok(resolved)
}

/// [`confine_in`] against the real upload directory.
pub fn confine(path: &str) -> Result<PathBuf, String> {
    confine_in(path, &upload_dir())
}

/// The media type the bytes say they are, if they are a supported image.
fn sniff(bytes: &[u8]) -> Option<&'static str> {
    if bytes.starts_with(b"\x89PNG\r\n\x1a\n") {
        Some("image/png")
    } else if bytes.starts_with(&[0xFF, 0xD8, 0xFF]) {
        Some("image/jpeg")
    } else if bytes.starts_with(b"GIF87a") || bytes.starts_with(b"GIF89a") {
        Some("image/gif")
    } else if bytes.len() >= 12 && &bytes[..4] == b"RIFF" && &bytes[8..12] == b"WEBP" {
        Some("image/webp")
    } else {
        None
    }
}

/// Confine, size-check, read and sniff one upload in `dir`.
pub fn load_in(path: &str, dir: &Path) -> Result<ImageInput, String> {
    let resolved = confine_in(path, dir)?;
    let name = resolved
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_default();
    let size = std::fs::symlink_metadata(&resolved)
        .map_err(|e| format!("image {name}: {e}"))?
        .len();
    if size > MAX_IMAGE_BYTES {
        return Err(format!(
            "image {name} is {:.1} MiB; images may be at most {:.2} MiB (resize it, or attach it as a file)",
            size as f64 / 1048576.0,
            MAX_IMAGE_BYTES as f64 / 1048576.0
        ));
    }
    let bytes = std::fs::read(&resolved).map_err(|e| format!("image {name}: {e}"))?;
    if bytes.len() as u64 > MAX_IMAGE_BYTES {
        return Err(format!("image {name} changed size while it was read"));
    }
    let media_type = sniff(&bytes)
        .ok_or_else(|| format!("image {name} is not a PNG, JPEG, GIF or WebP image"))?;
    Ok(ImageInput {
        path: resolved,
        media_type,
        data: base64::engine::general_purpose::STANDARD.encode(&bytes),
    })
}

/// Load every image a message names, in order, or refuse the whole message.
pub fn load_all_in(paths: &[String], dir: &Path) -> Result<Vec<ImageInput>, String> {
    if paths.len() > MAX_IMAGES {
        return Err(format!(
            "a message may carry at most {MAX_IMAGES} images ({} given)",
            paths.len()
        ));
    }
    paths.iter().map(|path| load_in(path, dir)).collect()
}

/// [`load_all_in`] against the real upload directory.
pub fn load_all(paths: &[String]) -> Result<Vec<ImageInput>, String> {
    load_all_in(paths, &upload_dir())
}

/// The text-marker fallback for a transport that cannot take images: one
/// `[Image: <path>]` line per image ahead of the text — the same shape the
/// native client sends for providers without structured image input.
pub fn with_markers(text: String, images: &[ImageInput]) -> String {
    if images.is_empty() {
        return text;
    }
    let mut out = String::new();
    for image in images {
        out.push_str(&format!("[Image: {}]\n", image.path.display()));
    }
    out.push_str(&text);
    out
}

/// Claude stream: put the images ahead of the text in a user frame's content,
/// as inline base64 image blocks.
pub fn prepend_claude_blocks(frame: &mut Value, images: &[ImageInput]) {
    if images.is_empty() {
        return;
    }
    let Some(content) = frame
        .pointer_mut("/message/content")
        .and_then(Value::as_array_mut)
    else {
        return;
    };
    let blocks = images.iter().map(|image| {
        json!({
            "type": "image",
            "source": { "type": "base64", "media_type": image.media_type, "data": image.data }
        })
    });
    content.splice(0..0, blocks);
}

/// Codex app-server v2 `UserInput` items for one message: the images as
/// `localImage` (codex reads the file itself), then the text.
pub fn codex_input(text: &str, images: &[PathBuf]) -> Value {
    let mut items: Vec<Value> = images
        .iter()
        .map(|path| json!({ "type": "localImage", "path": path }))
        .collect();
    items.push(json!({ "type": "text", "text": text }));
    Value::Array(items)
}

#[cfg(test)]
mod tests {
    use super::*;

    const PNG: &[u8] = b"\x89PNG\r\n\x1a\nrest-of-a-png";

    fn dir() -> PathBuf {
        let dir = crate::testtmp::dir().join(format!("uploads-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn put(dir: &Path, name: &str, bytes: &[u8]) -> String {
        let path = dir.join(name);
        std::fs::write(&path, bytes).unwrap();
        path.to_string_lossy().into_owned()
    }

    #[test]
    fn only_hub_named_uploads_inside_the_directory_are_accepted() {
        let dir = dir();
        let good = put(&dir, "m-1700000000000-0a1b2c3d.png", PNG);
        let image = load_in(&good, &dir).unwrap();
        assert_eq!(image.media_type, "image/png");
        assert_eq!(image.path, dir.join("m-1700000000000-0a1b2c3d.png"));
        assert_eq!(
            base64::engine::general_purpose::STANDARD
                .decode(&image.data)
                .unwrap(),
            PNG
        );
        // Not the hub's naming, a pdf, relative, or outside the directory.
        let other = put(&dir, "photo.png", PNG);
        let pdf = put(&dir, "m-1700000000000-0a1b2c3d.pdf", b"%PDF-1.4");
        let outside = dir.join("nested");
        std::fs::create_dir_all(&outside).unwrap();
        let nested = put(&outside, "m-1700000000000-0a1b2c3d.png", PNG);
        let escape = format!(
            "{}/../{}/m-1700000000000-0a1b2c3d.png",
            dir.display(),
            dir.file_name().unwrap().to_string_lossy()
        );
        for refused in [
            other,
            pdf,
            nested,
            "m-1700000000000-0a1b2c3d.png".to_string(),
            dir.join("m-1700000000000-ffffffff.png")
                .to_string_lossy()
                .into_owned(),
            "/etc/passwd".to_string(),
        ] {
            let err = load_in(&refused, &dir).unwrap_err();
            assert!(err.contains("not a file uploaded"), "{refused}: {err}");
        }
        // `..` that resolves back into the directory names the same upload.
        assert!(load_in(&escape, &dir).is_ok());
    }

    #[cfg(unix)]
    #[test]
    fn a_symlink_planted_in_the_upload_directory_is_refused() {
        let dir = dir();
        let target = put(&dir, "m-1700000000000-00000000.png", PNG);
        let link = dir.join("m-1700000000000-11111111.png");
        std::os::unix::fs::symlink(&target, &link).unwrap();
        let err = load_in(&link.to_string_lossy(), &dir).unwrap_err();
        assert!(err.contains("not a file uploaded"), "{err}");
    }

    #[test]
    fn bytes_must_be_a_supported_image_and_within_the_caps() {
        let dir = dir();
        for (bytes, kind) in [
            (&b"\xFF\xD8\xFF\xE0jpeg"[..], "image/jpeg"),
            (&b"GIF89a...."[..], "image/gif"),
            (&b"RIFF\0\0\0\0WEBPVP8 "[..], "image/webp"),
        ] {
            let path = put(&dir, "m-1-abcdef01.png", bytes);
            assert_eq!(load_in(&path, &dir).unwrap().media_type, kind);
        }
        let text = put(&dir, "m-2-abcdef01.png", b"not an image");
        assert!(load_in(&text, &dir).unwrap_err().contains("not a PNG"));
        let mut big = PNG.to_vec();
        big.resize(MAX_IMAGE_BYTES as usize + 1, 0);
        let big = put(&dir, "m-3-abcdef01.png", &big);
        assert!(load_in(&big, &dir)
            .unwrap_err()
            .contains("at most 3.75 MiB"));
        let one = put(&dir, "m-4-abcdef01.png", PNG);
        let many = vec![one.clone(); MAX_IMAGES + 1];
        assert!(load_all_in(&many, &dir).unwrap_err().contains("at most 8"));
        assert_eq!(load_all_in(&many[..MAX_IMAGES], &dir).unwrap().len(), 8);
    }

    #[test]
    fn providers_get_blocks_or_items_ahead_of_the_text_and_others_get_markers() {
        let image = ImageInput {
            path: PathBuf::from("/tmp/u/m-1-abcdef01.png"),
            media_type: "image/png",
            data: "AAAA".into(),
        };
        let mut frame = json!({ "type": "user",
            "message": { "role": "user", "content": [ { "type": "text", "text": "look" } ] } });
        prepend_claude_blocks(&mut frame, std::slice::from_ref(&image));
        assert_eq!(
            frame["message"]["content"],
            json!([
                { "type": "image", "source": { "type": "base64", "media_type": "image/png", "data": "AAAA" } },
                { "type": "text", "text": "look" }
            ])
        );
        assert_eq!(
            codex_input("look", std::slice::from_ref(&image.path)),
            json!([
                { "type": "localImage", "path": "/tmp/u/m-1-abcdef01.png" },
                { "type": "text", "text": "look" }
            ])
        );
        assert_eq!(
            with_markers("look".into(), std::slice::from_ref(&image)),
            "[Image: /tmp/u/m-1-abcdef01.png]\nlook"
        );
        assert_eq!(with_markers("look".into(), &[]), "look");
        assert!(!format!("{image:?}").contains("AAAA"));
    }
}
