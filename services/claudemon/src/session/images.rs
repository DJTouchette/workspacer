//! Real image input for managed sends: the files a client uploaded through
//! the hub's `files.upload`, attached to `agents.sendMessage` as `images`.
//!
//! Only an upload can be attached. A client names a path, and [`confine_in`]
//! accepts it only when it is a file the upload landing pad wrote: directly
//! inside [`upload_dir`] (itself a real, private directory of this user —
//! the same checks the hub's upload handler makes before writing), under the
//! hub-composed `m-<millis>-<8 hex>.<ext>` basename with an image extension.
//! Anything else is refused, so `images` can never name an arbitrary file.
//!
//! What is done with an accepted upload depends on who receives it
//! ([`ImageUse`]):
//!
//! - **Claude stream** reads the bytes inline: the file is opened ONCE,
//!   without following a symlink (`O_NOFOLLOW`, or a reparse point refused
//!   on Windows), checked as a regular file of bounded size on that same
//!   handle, read through a bounded reader, sniffed (PNG, JPEG, GIF or WebP
//!   by magic number) and its header dimensions checked. A file swapped for
//!   a symlink, a device or a growing file between the name check and the
//!   read is refused, never read unboundedly.
//! - **Codex** reads a path itself, later. The same validated bytes are
//!   copied to a fresh file the daemon owns (`0600`, a new name in the
//!   private upload directory), and Codex is given that copy, never the
//!   client's file; the driver deletes it once the turn that carried it ends
//!   (or the session does).
//! - **Everything else** (PTY sessions, text-only drivers, other engines)
//!   only ever sees `[Image: <path>]` markers ([`with_markers`]), so only the
//!   confinement applies — no bytes are read.
//!
//! Caps: at most [`MAX_IMAGES`] per message, [`MAX_IMAGE_BYTES`] per image
//! (the Claude API's 5 MB ceiling applies to the base64 text, so the file
//! may be three quarters of that), [`MAX_INLINE_BYTES`] of inline image data
//! per Claude message, and [`MAX_DIMENSION`] pixels on a side. Nothing is
//! downscaled: an image over a cap is refused with a message saying so, and
//! the native client sends such an image as a path marker instead.
use base64::Engine;
use serde_json::{json, Value};
use std::io::Read;
use std::path::{Path, PathBuf};

/// Most images one message may carry.
pub const MAX_IMAGES: usize = 8;
/// Largest image file accepted: 5 MiB of base64 (the Claude API's per-image
/// limit is on the encoded data) is 3.75 MiB of file.
pub const MAX_IMAGE_BYTES: u64 = 5 * 1024 * 1024 / 4 * 3;
/// Most inline image data one Claude message carries: 20 MiB of base64.
pub const MAX_INLINE_BYTES: u64 = 20 * 1024 * 1024 / 4 * 3;
/// Largest width or height accepted, read from the image header.
pub const MAX_DIMENSION: u32 = 8000;
/// Extensions an attachable upload may carry (`files.upload` also accepts
/// `pdf`, which is not an image and keeps the text marker).
const EXTENSIONS: [&str; 5] = ["png", "jpg", "jpeg", "gif", "webp"];
/// Prefix of the daemon's own per-message copies for Codex.
const COPY_PREFIX: &str = "codex-";

/// How a session takes the images of a message.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ImageUse {
    /// Inline base64 image blocks (Claude stream).
    Inline,
    /// A daemon-owned copy whose path the provider reads (Codex).
    Copy,
    /// `[Image: <path>]` markers only (PTY, text-only drivers, other engines).
    Markers,
}

/// One accepted image.
#[derive(Clone, PartialEq, Eq)]
pub struct ImageInput {
    /// The client's upload: what a marker names.
    pub source: PathBuf,
    /// What the provider reads: the daemon's own copy for [`ImageUse::Copy`],
    /// otherwise the upload itself.
    pub path: PathBuf,
    /// Empty for [`ImageUse::Markers`].
    pub media_type: &'static str,
    /// Base64 of the bytes, for [`ImageUse::Inline`] only.
    pub data: String,
}

impl ImageInput {
    /// Whether [`Self::path`] is a daemon-owned copy to delete after use.
    pub fn owned_copy(&self) -> bool {
        self.path != self.source
    }
}

impl std::fmt::Debug for ImageInput {
    // Never print megabytes of base64 into a log line.
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ImageInput")
            .field("source", &self.source)
            .field("path", &self.path)
            .field("media_type", &self.media_type)
            .field("inline_bytes", &(self.data.len() / 4 * 3))
            .finish()
    }
}

/// Where `files.upload` writes: one private directory per OS user under the
/// system temp directory. The hub's upload handler uses this same function.
pub fn upload_dir() -> PathBuf {
    #[cfg(test)]
    if let Some(dir) = TEST_UPLOAD_DIR.lock().unwrap().clone() {
        return dir;
    }
    #[cfg(unix)]
    let uid = nix::unistd::getuid().as_raw().to_string();
    #[cfg(not(unix))]
    let uid = "-1";
    std::env::temp_dir().join(format!("workspacer-uploads-{uid}"))
}

/// Tests point the daemon at a throwaway upload directory instead of the
/// real one (see [`tests::UploadDir`]).
#[cfg(test)]
static TEST_UPLOAD_DIR: std::sync::Mutex<Option<PathBuf>> = std::sync::Mutex::new(None);

/// The upload directory must be a real directory (not a symlink) private to
/// this user — what the hub's upload handler requires before it writes. On
/// Windows, where there is no mode to read, it must lie inside this user's
/// own profile (`%LOCALAPPDATA%` or `%USERPROFILE%`). Returns its canonical
/// path.
pub fn check_dir(dir: &Path) -> Result<PathBuf, String> {
    let refuse = |why: &str| format!("upload directory {}: {why}", dir.display());
    let meta = std::fs::symlink_metadata(dir).map_err(|e| refuse(&e.to_string()))?;
    if !meta.is_dir() || meta.file_type().is_symlink() {
        return Err(refuse("not a regular directory"));
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        if meta.uid() != nix::unistd::getuid().as_raw() || meta.mode() & 0o077 != 0 {
            return Err(refuse("not private to this user"));
        }
    }
    let canonical = dir.canonicalize().map_err(|e| refuse(&e.to_string()))?;
    #[cfg(windows)]
    {
        let inside = ["LOCALAPPDATA", "USERPROFILE"].iter().any(|var| {
            std::env::var_os(var)
                .and_then(|root| Path::new(&root).canonicalize().ok())
                .is_some_and(|root| canonical.starts_with(root))
        });
        if !inside {
            return Err(refuse("not inside this user's profile"));
        }
    }
    Ok(canonical)
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

/// Accept `path` only as an upload directly inside `dir` (which must pass
/// [`check_dir`]), without reading it. Returns the path joined onto `dir`
/// (the caller's own spelling of the upload, never a symlink target).
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
    let root = check_dir(dir)?;
    let parent = given.parent().ok_or_else(refuse)?;
    if parent.canonicalize().ok().as_deref() != Some(root.as_path()) {
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

/// Open `path` once without following a final symlink (or reparse point),
/// and without blocking on a FIFO.
fn open_nofollow(path: &Path) -> std::io::Result<std::fs::File> {
    let mut options = std::fs::OpenOptions::new();
    options.read(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.custom_flags(nix::libc::O_NOFOLLOW | nix::libc::O_NONBLOCK);
    }
    #[cfg(windows)]
    {
        use std::os::windows::fs::OpenOptionsExt;
        // FILE_FLAG_OPEN_REPARSE_POINT: open a link itself, never its target.
        options.custom_flags(0x0020_0000);
    }
    options.open(path)
}

/// Read one confined upload through a single handle: regular file, at most
/// [`MAX_IMAGE_BYTES`], a supported image within [`MAX_DIMENSION`].
fn read_checked(resolved: &Path) -> Result<(Vec<u8>, &'static str), String> {
    let name = resolved
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_default();
    let file = open_nofollow(resolved).map_err(|e| format!("image {name}: {e}"))?;
    let meta = file.metadata().map_err(|e| format!("image {name}: {e}"))?;
    #[cfg(windows)]
    {
        use std::os::windows::fs::MetadataExt;
        // FILE_ATTRIBUTE_REPARSE_POINT
        if meta.file_attributes() & 0x400 != 0 {
            return Err(format!("image {name} is not a regular file"));
        }
    }
    if !meta.file_type().is_file() {
        return Err(format!("image {name} is not a regular file"));
    }
    if meta.len() > MAX_IMAGE_BYTES {
        return Err(too_big(&name, meta.len()));
    }
    let mut bytes = Vec::with_capacity(meta.len() as usize);
    file.take(MAX_IMAGE_BYTES + 1)
        .read_to_end(&mut bytes)
        .map_err(|e| format!("image {name}: {e}"))?;
    if bytes.len() as u64 > MAX_IMAGE_BYTES {
        return Err(too_big(&name, bytes.len() as u64));
    }
    let media_type = sniff(&bytes)
        .ok_or_else(|| format!("image {name} is not a PNG, JPEG, GIF or WebP image"))?;
    let (width, height) = dimensions(&bytes, media_type)
        .ok_or_else(|| format!("image {name}: its dimensions could not be read"))?;
    if width > MAX_DIMENSION || height > MAX_DIMENSION {
        return Err(format!(
            "image {name} is {width}×{height} px; images may be at most {MAX_DIMENSION} px on a side (resize it, or attach it as a file)"
        ));
    }
    Ok((bytes, media_type))
}

fn too_big(name: &str, size: u64) -> String {
    format!(
        "image {name} is {:.1} MiB; images may be at most {:.2} MiB (resize it, or attach it as a file)",
        size as f64 / 1048576.0,
        MAX_IMAGE_BYTES as f64 / 1048576.0
    )
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

/// Width and height from the image header.
fn dimensions(b: &[u8], media_type: &str) -> Option<(u32, u32)> {
    let be16 = |i: usize| Some(u16::from_be_bytes([*b.get(i)?, *b.get(i + 1)?]) as u32);
    let le16 = |i: usize| Some(u16::from_le_bytes([*b.get(i)?, *b.get(i + 1)?]) as u32);
    let le24 = |i: usize| {
        Some(u32::from_le_bytes([
            *b.get(i)?,
            *b.get(i + 1)?,
            *b.get(i + 2)?,
            0,
        ]))
    };
    match media_type {
        "image/png" => {
            let be32 = |i: usize| Some(u32::from_be_bytes(b.get(i..i + 4)?.try_into().ok()?));
            (b.get(12..16)? == b"IHDR").then_some(())?;
            Some((be32(16)?, be32(20)?))
        }
        "image/gif" => Some((le16(6)?, le16(8)?)),
        "image/webp" => match b.get(12..16)? {
            b"VP8 " => Some((le16(26)? & 0x3fff, le16(28)? & 0x3fff)),
            b"VP8L" => {
                let bits = u32::from_le_bytes(b.get(21..25)?.try_into().ok()?);
                Some(((bits & 0x3fff) + 1, ((bits >> 14) & 0x3fff) + 1))
            }
            b"VP8X" => Some((le24(24)? + 1, le24(27)? + 1)),
            _ => None,
        },
        "image/jpeg" => {
            let mut i = 2;
            loop {
                while *b.get(i)? == 0xFF && *b.get(i + 1)? == 0xFF {
                    i += 1;
                }
                if *b.get(i)? != 0xFF {
                    return None;
                }
                let marker = *b.get(i + 1)?;
                match marker {
                    0xC0..=0xCF if !matches!(marker, 0xC4 | 0xC8 | 0xCC) => {
                        return Some((be16(i + 7)?, be16(i + 5)?));
                    }
                    0x01 | 0xD0..=0xD9 => i += 2,
                    _ => i += 2 + be16(i + 2)? as usize,
                }
            }
        }
        _ => None,
    }
}

/// Write the validated bytes to a fresh daemon-owned file in `dir` (created
/// new, `0600`), named so it can never be mistaken for an upload.
fn write_copy(dir: &Path, bytes: &[u8], ext: &str) -> Result<PathBuf, String> {
    use std::io::Write;
    let path = dir.join(format!(
        "{COPY_PREFIX}{}.{ext}",
        uuid::Uuid::new_v4().simple()
    ));
    let mut options = std::fs::OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    let mut file = options
        .open(&path)
        .map_err(|e| format!("image copy: {e}"))?;
    if let Err(e) = file.write_all(bytes).and_then(|_| file.sync_all()) {
        let _ = std::fs::remove_file(&path);
        return Err(format!("image copy: {e}"));
    }
    Ok(path)
}

/// Prepare every image a message names, in order, for `usage`, against the
/// upload directory `dir` — or refuse the whole message. Blocking: run it off
/// the async workers.
pub fn prepare_in(
    paths: &[String],
    usage: ImageUse,
    dir: &Path,
) -> Result<Vec<ImageInput>, String> {
    if paths.len() > MAX_IMAGES {
        return Err(format!(
            "a message may carry at most {MAX_IMAGES} images ({} given)",
            paths.len()
        ));
    }
    let mut out: Vec<ImageInput> = Vec::with_capacity(paths.len());
    let mut inline = 0u64;
    let result = paths.iter().try_for_each(|path| {
        let source = confine_in(path, dir)?;
        if usage == ImageUse::Markers {
            out.push(ImageInput {
                path: source.clone(),
                source,
                media_type: "",
                data: String::new(),
            });
            return Ok(());
        }
        let (bytes, media_type) = read_checked(&source)?;
        let image = if usage == ImageUse::Inline {
            inline += bytes.len() as u64;
            if inline > MAX_INLINE_BYTES {
                return Err(format!(
                    "these images total more than {} MiB once encoded; send fewer at once (or attach them as files)",
                    MAX_INLINE_BYTES / 3 * 4 / 1048576
                ));
            }
            ImageInput {
                path: source.clone(),
                source,
                media_type,
                data: base64::engine::general_purpose::STANDARD.encode(&bytes),
            }
        } else {
            let ext = media_type.trim_start_matches("image/");
            ImageInput {
                path: write_copy(dir, &bytes, ext)?,
                source,
                media_type,
                data: String::new(),
            }
        };
        out.push(image);
        Ok(())
    });
    match result {
        Ok(()) => Ok(out),
        Err(error) => {
            discard(&out);
            Err(error)
        }
    }
}

/// [`prepare_in`] against the real upload directory.
pub fn prepare(paths: &[String], usage: ImageUse) -> Result<Vec<ImageInput>, String> {
    prepare_in(paths, usage, &upload_dir())
}

/// Delete the daemon-owned copies among `images` (Codex); uploads are left.
pub fn discard(images: &[ImageInput]) {
    for image in images.iter().filter(|i| i.owned_copy()) {
        let _ = std::fs::remove_file(&image.path);
    }
}

/// Delete daemon-owned copies by path.
pub fn discard_paths(paths: &[PathBuf]) {
    for path in paths {
        let _ = std::fs::remove_file(path);
    }
}

/// The text-marker fallback for a transport that cannot take images: one
/// `[Image: <upload path>]` line per image ahead of the text — the same shape
/// the native client sends for providers without structured image input.
pub fn with_markers(text: String, images: &[ImageInput]) -> String {
    let sources: Vec<&Path> = images.iter().map(|i| i.source.as_path()).collect();
    with_path_markers(text, &sources)
}

/// [`with_markers`] for bare paths.
pub fn with_path_markers(text: String, paths: &[&Path]) -> String {
    if paths.is_empty() {
        return text;
    }
    let mut out = String::new();
    for path in paths {
        out.push_str(&format!("[Image: {}]\n", path.display()));
    }
    out.push_str(&text);
    out
}

/// Claude stream: put the images ahead of the text in a user frame's content,
/// as inline base64 image blocks. An image without inline data (never
/// prepared for inline use) is skipped.
pub fn prepend_claude_blocks(frame: &mut Value, images: &[ImageInput]) {
    let Some(content) = frame
        .pointer_mut("/message/content")
        .and_then(Value::as_array_mut)
    else {
        return;
    };
    let blocks: Vec<Value> = images
        .iter()
        .filter(|image| !image.data.is_empty())
        .map(|image| {
            json!({
                "type": "image",
                "source": { "type": "base64", "media_type": image.media_type, "data": image.data }
            })
        })
        .collect();
    content.splice(0..0, blocks);
}

/// Codex app-server v2 `UserInput` items for one message: the images as
/// `localImage` (codex reads the file itself), then the text.
pub fn codex_input(text: &str, images: &[ImageInput]) -> Value {
    let mut items: Vec<Value> = images
        .iter()
        .map(|image| json!({ "type": "localImage", "path": image.path }))
        .collect();
    items.push(json!({ "type": "text", "text": text }));
    Value::Array(items)
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;

    /// A 1×1 PNG header (IHDR included).
    pub(crate) fn png(width: u32, height: u32) -> Vec<u8> {
        let mut b = b"\x89PNG\r\n\x1a\n\0\0\0\x0dIHDR".to_vec();
        b.extend_from_slice(&width.to_be_bytes());
        b.extend_from_slice(&height.to_be_bytes());
        b.extend_from_slice(b"\x08\x06\0\0\0rest");
        b
    }

    /// Route [`upload_dir`] to a private temp directory for one test, and
    /// remove it afterwards. Holding it serializes the tests that use it.
    pub(crate) struct UploadDir {
        pub path: PathBuf,
        _serial: std::sync::MutexGuard<'static, ()>,
    }

    static SERIAL: std::sync::Mutex<()> = std::sync::Mutex::new(());

    impl UploadDir {
        pub(crate) fn new() -> Self {
            let serial = SERIAL.lock().unwrap_or_else(|e| e.into_inner());
            let path = dir();
            *TEST_UPLOAD_DIR.lock().unwrap() = Some(path.clone());
            Self {
                path,
                _serial: serial,
            }
        }
    }

    impl Drop for UploadDir {
        fn drop(&mut self) {
            *TEST_UPLOAD_DIR.lock().unwrap() = None;
            let _ = std::fs::remove_dir_all(&self.path);
        }
    }

    /// A private upload directory, like the hub's.
    pub(crate) fn dir() -> PathBuf {
        let dir = crate::testtmp::dir().join(format!("uploads-{}", uuid::Uuid::new_v4()));
        let mut builder = std::fs::DirBuilder::new();
        builder.recursive(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::DirBuilderExt;
            builder.mode(0o700);
        }
        builder.create(&dir).unwrap();
        dir
    }

    pub(crate) fn put(dir: &Path, name: &str, bytes: &[u8]) -> String {
        let path = dir.join(name);
        std::fs::write(&path, bytes).unwrap();
        path.to_string_lossy().into_owned()
    }

    fn load(path: &str, dir: &Path) -> Result<ImageInput, String> {
        prepare_in(&[path.to_owned()], ImageUse::Inline, dir).map(|mut v| v.remove(0))
    }

    #[test]
    fn only_hub_named_uploads_inside_the_directory_are_accepted() {
        let dir = dir();
        let good = put(&dir, "m-1700000000000-0a1b2c3d.png", &png(1, 1));
        let image = load(&good, &dir).unwrap();
        assert_eq!(image.media_type, "image/png");
        assert_eq!(image.path, dir.join("m-1700000000000-0a1b2c3d.png"));
        assert!(!image.owned_copy());
        assert_eq!(
            base64::engine::general_purpose::STANDARD
                .decode(&image.data)
                .unwrap(),
            png(1, 1)
        );
        // Not the hub's naming, a pdf, relative, or outside the directory.
        let other = put(&dir, "photo.png", &png(1, 1));
        let pdf = put(&dir, "m-1700000000000-0a1b2c3d.pdf", b"%PDF-1.4");
        let outside = dir.join("nested");
        std::fs::create_dir_all(&outside).unwrap();
        let nested = put(&outside, "m-1700000000000-0a1b2c3d.png", &png(1, 1));
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
            let err = load(&refused, &dir).unwrap_err();
            assert!(err.contains("not a file uploaded"), "{refused}: {err}");
        }
        // `..` that resolves back into the directory names the same upload.
        assert!(load(&escape, &dir).is_ok());
    }

    #[cfg(unix)]
    #[test]
    fn the_directory_must_be_a_private_real_directory_of_this_user() {
        use std::os::unix::fs::PermissionsExt;
        let dir = dir();
        let good = put(&dir, "m-1-abcdef01.png", &png(1, 1));
        std::fs::set_permissions(&dir, std::fs::Permissions::from_mode(0o755)).unwrap();
        assert!(confine_in(&good, &dir).unwrap_err().contains("not private"));
        std::fs::set_permissions(&dir, std::fs::Permissions::from_mode(0o700)).unwrap();
        assert!(confine_in(&good, &dir).is_ok());
        // A symlinked upload directory is refused even when its target is fine.
        let link = dir.with_extension("link");
        std::os::unix::fs::symlink(&dir, &link).unwrap();
        let through = link.join("m-1-abcdef01.png").to_string_lossy().into_owned();
        assert!(confine_in(&through, &link)
            .unwrap_err()
            .contains("not a regular directory"));
    }

    #[cfg(unix)]
    #[test]
    fn a_symlink_or_device_swapped_in_after_the_name_check_is_never_read() {
        let dir = dir();
        let target = put(&dir, "m-1700000000000-00000000.png", &png(1, 1));
        let link = dir.join("m-1700000000000-11111111.png");
        std::os::unix::fs::symlink(&target, &link).unwrap();
        // The name check alone already refuses a planted link …
        let err = load(&link.to_string_lossy(), &dir).unwrap_err();
        assert!(err.contains("not a file uploaded"), "{err}");
        // … and the single open refuses one swapped in after it passed
        // (here: a link to /dev/zero, which would otherwise read forever).
        let swapped = dir.join("m-1700000000000-22222222.png");
        std::os::unix::fs::symlink("/dev/zero", &swapped).unwrap();
        let err = read_checked(&swapped).unwrap_err();
        assert!(err.contains("image m-1700000000000-22222222.png"), "{err}");
        // A FIFO neither blocks the open nor passes as a regular file.
        let fifo = dir.join("m-1700000000000-33333333.png");
        assert!(std::process::Command::new("mkfifo")
            .arg(&fifo)
            .status()
            .unwrap()
            .success());
        assert!(read_checked(&fifo)
            .unwrap_err()
            .contains("not a regular file"));
    }

    #[test]
    fn a_file_that_grows_past_the_cap_is_read_only_up_to_it() {
        use std::io::Write;
        let dir = dir();
        let mut bytes = png(1, 1);
        bytes.resize(MAX_IMAGE_BYTES as usize, 0);
        let path = put(&dir, "m-1-abcdef02.png", &bytes);
        let resolved = confine_in(&path, &dir).unwrap();
        // Open the handle, then grow the file underneath it: the bounded
        // reader stops one byte past the cap and refuses.
        let file = open_nofollow(&resolved).unwrap();
        std::fs::OpenOptions::new()
            .append(true)
            .open(&resolved)
            .unwrap()
            .write_all(&[0; 4096])
            .unwrap();
        let mut read = Vec::new();
        file.take(MAX_IMAGE_BYTES + 1)
            .read_to_end(&mut read)
            .unwrap();
        assert_eq!(read.len() as u64, MAX_IMAGE_BYTES + 1);
        assert!(read_checked(&resolved)
            .unwrap_err()
            .contains("at most 3.75 MiB"));
    }

    #[test]
    fn bytes_must_be_a_supported_image_and_within_the_caps() {
        let dir = dir();
        let mut jpeg = b"\xFF\xD8\xFF\xE0\x00\x04ab\xFF\xC0\x00\x11\x08".to_vec();
        jpeg.extend_from_slice(&30u16.to_be_bytes()); // height
        jpeg.extend_from_slice(&40u16.to_be_bytes()); // width
        let mut gif = b"GIF89a".to_vec();
        gif.extend_from_slice(&[5, 0, 6, 0]);
        let mut webp = b"RIFF\0\0\0\0WEBPVP8X\0\0\0\0\0\0\0\0".to_vec();
        webp.extend_from_slice(&[9, 0, 0, 19, 0, 0]);
        for (bytes, kind, size) in [
            (jpeg, "image/jpeg", (40, 30)),
            (gif, "image/gif", (5, 6)),
            (webp, "image/webp", (10, 20)),
            (png(7, 8), "image/png", (7, 8)),
        ] {
            assert_eq!(sniff(&bytes), Some(kind));
            assert_eq!(dimensions(&bytes, kind), Some(size), "{kind}");
            let path = put(&dir, "m-1-abcdef01.png", &bytes);
            assert_eq!(load(&path, &dir).unwrap().media_type, kind);
        }
        let text = put(&dir, "m-2-abcdef01.png", b"not an image");
        assert!(load(&text, &dir).unwrap_err().contains("not a PNG"));
        let mut big = png(1, 1);
        big.resize(MAX_IMAGE_BYTES as usize + 1, 0);
        let big = put(&dir, "m-3-abcdef01.png", &big);
        assert!(load(&big, &dir).unwrap_err().contains("at most 3.75 MiB"));
        let wide = put(&dir, "m-5-abcdef01.png", &png(MAX_DIMENSION + 1, 10));
        assert!(load(&wide, &dir).unwrap_err().contains("px on a side"));
        let one = put(&dir, "m-4-abcdef01.png", &png(1, 1));
        let many = vec![one.clone(); MAX_IMAGES + 1];
        assert!(prepare_in(&many, ImageUse::Inline, &dir)
            .unwrap_err()
            .contains("at most 8"));
        assert_eq!(
            prepare_in(&many[..MAX_IMAGES], ImageUse::Inline, &dir)
                .unwrap()
                .len(),
            8
        );
    }

    #[test]
    fn a_claude_message_caps_its_total_inline_data() {
        let dir = dir();
        let mut bytes = png(1, 1);
        bytes.resize(MAX_IMAGE_BYTES as usize, 0);
        let path = put(&dir, "m-1-abcdef03.png", &bytes);
        // Four full-size images fit 15 MiB; five do not, for inline use only.
        let paths = vec![path.clone(); 5];
        assert!(prepare_in(&paths[..4], ImageUse::Inline, &dir).is_ok());
        assert!(prepare_in(&paths, ImageUse::Inline, &dir)
            .unwrap_err()
            .contains("total more than 20 MiB"));
        let copies = prepare_in(&paths, ImageUse::Copy, &dir).unwrap();
        discard(&copies);
    }

    #[test]
    fn codex_gets_a_private_copy_and_markers_read_nothing() {
        let dir = dir();
        let path = put(&dir, "m-1-abcdef04.png", &png(2, 2));
        let copies = prepare_in(std::slice::from_ref(&path), ImageUse::Copy, &dir).unwrap();
        let copy = &copies[0];
        assert!(copy.owned_copy());
        assert!(copy.data.is_empty(), "no base64 for a path reader");
        assert_eq!(copy.path.parent(), Some(dir.as_path()));
        assert!(copy
            .path
            .file_name()
            .unwrap()
            .to_string_lossy()
            .starts_with(COPY_PREFIX));
        assert_eq!(std::fs::read(&copy.path).unwrap(), png(2, 2));
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = std::fs::metadata(&copy.path).unwrap().permissions().mode();
            assert_eq!(mode & 0o777, 0o600);
        }
        // A copy is never itself attachable.
        assert!(confine_in(&copy.path.to_string_lossy(), &dir).is_err());
        discard(&copies);
        assert!(!copy.path.exists());
        assert!(Path::new(&path).exists(), "the upload itself is left");
        // Markers only confine: an unreadable "image" still passes.
        let junk = put(&dir, "m-1-abcdef05.png", b"not an image at all");
        let marked = prepare_in(std::slice::from_ref(&junk), ImageUse::Markers, &dir).unwrap();
        assert_eq!(
            with_markers("look".into(), &marked),
            format!("[Image: {}]\nlook", dir.join("m-1-abcdef05.png").display())
        );
    }

    #[test]
    fn providers_get_blocks_or_items_ahead_of_the_text_and_others_get_markers() {
        let image = ImageInput {
            source: PathBuf::from("/tmp/u/m-1-abcdef01.png"),
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
            codex_input("look", std::slice::from_ref(&image)),
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
