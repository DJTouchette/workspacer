//! Where a chat link goes. Pure classification shared by Markdown links, HTML
//! cards, image thumbnails and tool file targets, so every surface agrees.
//!
//! Files always name a path on the machine that runs the session. They are
//! read through the connected hub (which owns those sessions), never from the
//! client's own disk, and only `http`/`https` ever reach the system browser.
use anyhow::{Result, ensure};

/// Text previews: the hub reads up to 5 MiB; the viewer keeps 1 MiB of text
/// and the code editor's comfortable line budget.
pub const MAX_TEXT_BYTES: usize = 1024 * 1024;
pub const MAX_TEXT_LINES: usize = 50_000;
/// `fs.readImage` refuses larger sources; matched here for the message.
pub const MAX_IMAGE_BYTES: usize = 2 * 1024 * 1024;
/// Displayed images are re-encoded no larger than this on either side.
pub const MAX_IMAGE_SIDE: u32 = 2560;

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Link {
    /// An `http`/`https` URL for the system browser.
    Web(String),
    /// A file on the session's machine.
    File(FileTarget),
    /// A same-document anchor (`#heading`); nothing to open.
    Anchor,
    /// Recognised but not opened; the message says why.
    Refused(String),
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum FileKind {
    Image,
    Text,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FileTarget {
    /// Absolute path on the session's machine.
    pub path: String,
    /// 1-based line and column, when the link carried them.
    pub line: Option<u32>,
    pub column: Option<u32>,
    pub kind: FileKind,
}

impl FileTarget {
    pub fn name(&self) -> &str {
        self.path
            .rsplit(['/', '\\'])
            .find(|s| !s.is_empty())
            .unwrap_or(&self.path)
    }
    /// Highlighter language for the viewer.
    pub fn language(&self) -> &'static str {
        language(&self.path)
    }
}

/// Classify a Markdown/HTML link target written by an agent in `cwd`.
pub fn classify(cwd: &str, raw: &str) -> Link {
    let raw = raw.trim().trim_matches(['<', '>']).trim();
    if raw.is_empty() || raw.starts_with('#') {
        return Link::Anchor;
    }
    if raw.contains(['\0', '\n', '\r']) || raw.len() > 4096 {
        return Link::Refused("This link is malformed and was not opened.".into());
    }
    if let Some(scheme) = scheme(raw) {
        return match scheme.to_ascii_lowercase().as_str() {
            "http" | "https" => web(raw),
            "file" => file_url(cwd, raw),
            other => Link::Refused(format!(
                "{other}: links are not opened from chat. Only web links (http, https) and files open."
            )),
        };
    }
    let (path, fragment) = match raw.split_once('#') {
        Some((path, fragment)) => (path, Some(fragment)),
        None => (raw, None),
    };
    let path = percent_decode(path);
    let (path, mut line, mut column) = strip_location(&path);
    if let Some((l, c)) = fragment.and_then(fragment_location) {
        line = Some(l);
        column = c;
    }
    local(cwd, path, line, column)
}

/// A tool's own file argument (`file_path`, `path`): a plain path, never a URL,
/// with an optional 1-based line (e.g. a Read tool's `offset`).
pub fn tool_file(cwd: &str, path: &str, line: Option<u32>) -> Link {
    let path = path.trim();
    if path.is_empty() || path.contains(['\0', '\n', '\r']) || path.len() > 4096 {
        return Link::Refused("This tool did not name a usable file path.".into());
    }
    local(cwd, path, line.filter(|l| *l > 0), None)
}

fn local(cwd: &str, path: &str, line: Option<u32>, column: Option<u32>) -> Link {
    if path.is_empty() {
        return Link::Anchor;
    }
    if path == "~" || path.starts_with("~/") || path.starts_with("~\\") {
        return Link::Refused(format!(
            "“{path}” is relative to a home folder, which the viewer cannot resolve. Ask for an absolute path."
        ));
    }
    let cwd = cwd.trim();
    if !crate::transcript::absolute(path) && !crate::transcript::absolute(cwd) {
        return Link::Refused(format!(
            "“{path}” is relative and this session has no working directory to resolve it against."
        ));
    }
    let path = normalize(&crate::transcript::resolve_path(cwd, path));
    let kind = if image(&path) {
        FileKind::Image
    } else {
        FileKind::Text
    };
    Link::File(FileTarget {
        path,
        line,
        column: column.filter(|_| line.is_some()),
        kind,
    })
}

/// `scheme:` per RFC 3986, at least two characters so `C:\` stays a path, and
/// never followed by a digit so `README.md:12` stays a file location.
fn scheme(raw: &str) -> Option<&str> {
    let (scheme, rest) = raw.split_once(':')?;
    let mut chars = scheme.chars();
    (scheme.len() > 1
        && !rest.starts_with(|c: char| c.is_ascii_digit())
        && chars.next()?.is_ascii_alphabetic()
        && chars.all(|c| c.is_ascii_alphanumeric() || matches!(c, '+' | '-' | '.')))
    .then_some(scheme)
}

fn web(raw: &str) -> Link {
    match url::Url::parse(raw) {
        Ok(url)
            if matches!(url.scheme(), "http" | "https")
                && url.host_str().is_some_and(|h| !h.is_empty()) =>
        {
            Link::Web(url.into())
        }
        _ => Link::Refused("This web link is malformed and was not opened.".into()),
    }
}

/// `file:///abs`, `file://localhost/abs` and `file:///C:/abs` name the
/// session's machine like any other path; another host is never guessed at.
fn file_url(cwd: &str, raw: &str) -> Link {
    let rest = &raw[5..];
    let Some(rest) = rest.strip_prefix("//") else {
        return Link::Refused("This file link is malformed and was not opened.".into());
    };
    let (host, path) = match rest.find('/') {
        Some(i) => (&rest[..i], &rest[i..]),
        None => (rest, ""),
    };
    if !host.is_empty() && !host.eq_ignore_ascii_case("localhost") {
        return Link::Refused(format!(
            "file:// links on another machine ({host}) are not opened."
        ));
    }
    let (path, fragment) = match path.split_once('#') {
        Some((path, fragment)) => (path, Some(fragment)),
        None => (path, None),
    };
    let path = path.split('?').next().unwrap_or(path);
    let mut path = percent_decode(path);
    // `/C:/repo` → `C:/repo`
    let bytes = path.as_bytes();
    if bytes.len() > 3 && bytes[0] == b'/' && bytes[1].is_ascii_alphabetic() && bytes[2] == b':' {
        path.remove(0);
    }
    if !crate::transcript::absolute(&path) {
        return Link::Refused("This file link has no absolute path and was not opened.".into());
    }
    let (line, column) = fragment.and_then(fragment_location).unzip();
    local(cwd, &path, line, column.flatten())
}

/// `path:12`, `path:12:4` and `path:12-20` (ranges open at their first line).
fn strip_location(path: &str) -> (&str, Option<u32>, Option<u32>) {
    let number = |s: &str| {
        (!s.is_empty() && s.len() <= 9 && s.bytes().all(|b| b.is_ascii_digit()))
            .then(|| s.parse::<u32>().ok())
            .flatten()
    };
    let range = |s: &str| match s.split_once('-') {
        Some((a, b)) if number(b).is_some() => number(a),
        _ => number(s),
    };
    if let Some((head, last)) = path.rsplit_once(':') {
        if let Some((file, line)) = head.rsplit_once(':')
            && let (Some(line), Some(column)) = (number(line), number(last))
            && !file.is_empty()
            && !drive_only(file)
        {
            return (file, Some(line).filter(|l| *l > 0), Some(column).filter(|c| *c > 0));
        }
        if let Some(line) = range(last)
            && !head.is_empty()
            && !drive_only(head)
        {
            return (head, Some(line).filter(|l| *l > 0), None);
        }
    }
    (path, None, None)
}

fn drive_only(s: &str) -> bool {
    s.len() == 1 && s.as_bytes()[0].is_ascii_alphabetic()
}

/// GitHub-style `#L12`, `#L12C4`, `#L12-L20`; a bare heading slug is no line.
fn fragment_location(fragment: &str) -> Option<(u32, Option<u32>)> {
    let rest = fragment.strip_prefix(['L', 'l'])?;
    let rest = rest.split('-').next().unwrap_or(rest);
    let (line, column) = match rest.split_once(['C', 'c']) {
        Some((line, column)) => (line, column.parse::<u32>().ok()),
        None => (rest, None),
    };
    let line = line.parse::<u32>().ok().filter(|l| *l > 0)?;
    Some((line, column.filter(|c| *c > 0)))
}

/// `%XX` sequences only; anything that does not decode to UTF-8 stays literal.
fn percent_decode(s: &str) -> String {
    if !s.contains('%') {
        return s.into();
    }
    let bytes = s.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%'
            && let Some(hex) = s.get(i + 1..i + 3)
            && let Ok(byte) = u8::from_str_radix(hex, 16)
        {
            out.push(byte);
            i += 3;
        } else {
            out.push(bytes[i]);
            i += 1;
        }
    }
    String::from_utf8(out)
        .ok()
        .filter(|d| !d.contains('\0'))
        .unwrap_or_else(|| s.into())
}

/// Collapse `.` and `name/..` lexically for a readable title. The hub still
/// canonicalizes before reading, so this never widens what can be opened.
pub fn normalize(path: &str) -> String {
    let windows = path.contains('\\') || path.as_bytes().get(1) == Some(&b':');
    let sep = if windows { '\\' } else { '/' };
    let (prefix, rest) = if let Some(rest) = path.strip_prefix("\\\\") {
        ("\\\\".to_owned(), rest)
    } else if path.as_bytes().get(1) == Some(&b':') {
        (format!("{}{sep}", &path[..2]), &path[2..])
    } else if let Some(rest) = path.strip_prefix(['/', '\\']) {
        (sep.to_string(), rest)
    } else {
        (String::new(), path)
    };
    let mut parts: Vec<&str> = vec![];
    for part in rest.split(['/', '\\']) {
        match part {
            "" | "." => {}
            ".." if parts.last().is_some_and(|p| *p != "..") => {
                parts.pop();
            }
            ".." if !prefix.is_empty() => {}
            _ => parts.push(part),
        }
    }
    format!("{prefix}{}", parts.join(&sep.to_string()))
}

/// Raster formats both the hub and the native decoder accept. SVG is source.
pub fn image(path: &str) -> bool {
    path.rsplit_once('.').is_some_and(|(_, ext)| {
        matches!(
            ext.to_ascii_lowercase().as_str(),
            "png" | "jpg" | "jpeg" | "gif" | "webp" | "bmp"
        )
    })
}

/// Highlighter language by file name, wider than tool cards need.
pub fn language(path: &str) -> &'static str {
    let name = path.rsplit(['/', '\\']).next().unwrap_or(path);
    match name.to_ascii_lowercase().as_str() {
        "makefile" | "gnumakefile" => return "make",
        "cmakelists.txt" => return "cmake",
        "cargo.lock" => return "toml",
        _ => {}
    }
    let ext = name
        .rsplit_once('.')
        .map(|(_, e)| e.to_ascii_lowercase())
        .unwrap_or_default();
    match ext.as_str() {
        "scss" => "css",
        "mdx" | "markdown" => "markdown",
        "graphql" | "gql" => "graphql",
        "proto" => "proto",
        "zig" => "zig",
        "scala" | "sc" => "scala",
        "mk" => "make",
        "cmake" => "cmake",
        "svg" | "xml" => "html",
        "cc" | "cxx" | "hh" => "cpp",
        "erb" => "erb",
        "ejs" => "ejs",
        "mts" | "cts" => "typescript",
        _ => crate::tool_preview::language(name),
    }
}

/// Bound a text preview before it reaches the editor.
pub fn check_text(contents: &str) -> Result<()> {
    ensure!(
        contents.len() <= MAX_TEXT_BYTES,
        "This file is {} — previews show text files up to 1 MiB.",
        size(contents.len() as u64)
    );
    let lines = contents.lines().count();
    ensure!(
        lines <= MAX_TEXT_LINES,
        "This file has {lines} lines — previews show up to {MAX_TEXT_LINES}."
    );
    Ok(())
}

/// Turn a hub/OS read failure into something a person can act on.
pub fn read_error(target: &FileTarget, error: &str) -> String {
    let lower = error.to_ascii_lowercase();
    let what = match target.kind {
        FileKind::Image => "image",
        FileKind::Text => "file",
    };
    if lower.contains("unknown method") || lower.contains("not offered") {
        return "The connected hub can't read files for previews.".into();
    }
    if lower.contains("no such file")
        || lower.contains("os error 2)")
        || lower.contains("os error 3)")
        || lower.contains("cannot find the")
        || lower.contains("not found")
    {
        format!("No {what} at this path on the session's machine.")
    } else if lower.contains("permission denied")
        || lower.contains("os error 13)")
        || lower.contains("access is denied")
        || lower.contains("os error 5)")
    {
        format!("The session's machine denied access to this {what}.")
    } else if lower.contains("not a regular file") || lower.contains("choose a regular file") {
        "This is a folder or special file, not something that can be previewed.".into()
    } else if lower.contains("binary") || lower.contains("not valid utf-8") {
        "This file is not UTF-8 text, so it can't be shown as source.".into()
    } else if lower.contains("not a previewable browser image") {
        "This image format can't be previewed.".into()
    } else if lower.contains("source pixels")
        || lower.contains("dimensions")
        || lower.contains("limit")
    {
        "This image is too large to preview.".into()
    } else if lower.contains("(max ") || lower.contains("up to ") {
        match target.kind {
            FileKind::Image => "This image is larger than the 2 MiB preview limit.".into(),
            FileKind::Text => "This file is larger than the 1 MiB preview limit.".into(),
        }
    } else {
        format!("Couldn't open this {what}: {error}")
    }
}

pub fn size(bytes: u64) -> String {
    match bytes {
        b if b >= 1024 * 1024 => format!("{:.1} MiB", b as f64 / (1024. * 1024.)),
        b if b >= 1024 => format!("{:.0} KiB", b as f64 / 1024.),
        b => format!("{b} bytes"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn file(path: &str, line: Option<u32>, column: Option<u32>, kind: FileKind) -> Link {
        Link::File(FileTarget {
            path: path.into(),
            line,
            column,
            kind,
        })
    }

    #[test]
    fn web_links_are_only_http_and_https() {
        assert_eq!(
            classify("/repo", "https://example.com/a?b=1#c"),
            Link::Web("https://example.com/a?b=1#c".into())
        );
        assert_eq!(
            classify("/repo", "<HTTP://Example.com>"),
            Link::Web("http://example.com/".into())
        );
        for refused in [
            "mailto:user@example.com",
            "javascript:alert(1)",
            "vscode://file/a.rs",
            "ssh://host",
            "data:text/html,hi",
            "https://",
        ] {
            assert!(
                matches!(classify("/repo", refused), Link::Refused(_)),
                "{refused} must not open"
            );
        }
        assert_eq!(classify("/repo", "#section"), Link::Anchor);
        assert_eq!(classify("/repo", "  "), Link::Anchor);
    }

    #[test]
    fn relative_files_resolve_against_the_session_cwd_with_line_anchors() {
        assert_eq!(
            classify("/remote/repo", "docs/README.md:12:3"),
            file("/remote/repo/docs/README.md", Some(12), Some(3), FileKind::Text)
        );
        assert_eq!(
            classify("/remote/repo/", "./src/../src/lib.rs:40-52"),
            file("/remote/repo/src/lib.rs", Some(40), None, FileKind::Text)
        );
        assert_eq!(
            classify("/remote/repo", "src/main.rs#L7"),
            file("/remote/repo/src/main.rs", Some(7), None, FileKind::Text)
        );
        assert_eq!(
            classify("/remote/repo", "src/main.rs#L7C2-L9"),
            file("/remote/repo/src/main.rs", Some(7), Some(2), FileKind::Text)
        );
        assert_eq!(
            classify("/remote/repo", "/absolute/README.md#usage"),
            file("/absolute/README.md", None, None, FileKind::Text)
        );
        assert_eq!(
            classify("/remote/repo", "my%20notes.md"),
            file("/remote/repo/my notes.md", None, None, FileKind::Text)
        );
        assert_eq!(
            classify("/repo", "a.rs:0"),
            file("/repo/a.rs", None, None, FileKind::Text)
        );
        assert_eq!(
            classify("/repo", "README.md:12"),
            file("/repo/README.md", Some(12), None, FileKind::Text)
        );
    }

    #[test]
    fn windows_paths_keep_drive_letters_and_separators() {
        assert_eq!(
            classify("C:\\repo", "src\\main.rs:42"),
            file("C:\\repo\\src\\main.rs", Some(42), None, FileKind::Text)
        );
        assert_eq!(
            classify("/repo", "<C:\\repo\\a.rs:7:1>"),
            file("C:\\repo\\a.rs", Some(7), Some(1), FileKind::Text)
        );
        assert_eq!(
            classify("/repo", "file:///C:/repo/shot.PNG"),
            file("C:\\repo\\shot.PNG", None, None, FileKind::Image)
        );
        assert_eq!(normalize("\\\\server\\share\\.\\a\\..\\b"), "\\\\server\\share\\b");
    }

    #[test]
    fn images_route_to_the_image_viewer_and_svg_stays_source() {
        assert_eq!(
            classify("/repo", "out/screen.png"),
            file("/repo/out/screen.png", None, None, FileKind::Image)
        );
        assert_eq!(
            classify("/repo", "file:///tmp/a%20b.jpeg"),
            file("/tmp/a b.jpeg", None, None, FileKind::Image)
        );
        assert!(matches!(
            classify("/repo", "logo.svg"),
            Link::File(FileTarget { kind: FileKind::Text, .. })
        ));
    }

    #[test]
    fn unresolvable_or_foreign_paths_are_refused_with_a_reason() {
        let refused = |link| matches!(link, Link::Refused(message) if !message.is_empty());
        assert!(refused(classify("", "README.md")));
        assert!(refused(classify("/repo", "~/notes.md")));
        assert!(refused(classify("/repo", "file://build-box/srv/a.rs")));
        assert!(refused(classify("/repo", "file:relative.rs")));
        assert!(refused(classify("/repo", "a\0b")));
        assert!(refused(classify("/repo", &"a".repeat(5000))));
        // Absolute paths need no cwd.
        assert!(matches!(classify("", "/etc/hosts"), Link::File(_)));
    }

    #[test]
    fn tool_targets_share_resolution_without_url_parsing() {
        assert_eq!(
            tool_file("/repo", "src/a:b.rs", Some(30)),
            file("/repo/src/a:b.rs", Some(30), None, FileKind::Text)
        );
        assert_eq!(
            tool_file("/repo", "/tmp/shot.png", Some(0)),
            file("/tmp/shot.png", None, None, FileKind::Image)
        );
        assert!(matches!(tool_file("", "rel.rs", None), Link::Refused(_)));
    }

    #[test]
    fn text_previews_are_bounded_by_size_and_lines() {
        assert!(check_text("fn main() {}\n").is_ok());
        let wide = "x".repeat(MAX_TEXT_BYTES + 1);
        assert!(check_text(&wide).unwrap_err().to_string().contains("1 MiB"));
        let tall = "\n".repeat(MAX_TEXT_LINES + 1);
        assert!(check_text(&tall).unwrap_err().to_string().contains("lines"));
    }

    #[test]
    fn read_errors_are_actionable() {
        let text = FileTarget {
            path: "/repo/a.rs".into(),
            line: None,
            column: None,
            kind: FileKind::Text,
        };
        let image = FileTarget {
            kind: FileKind::Image,
            ..text.clone()
        };
        assert!(read_error(&text, "No such file or directory (os error 2)").starts_with("No file"));
        assert!(read_error(&text, "Permission denied (os error 13)").contains("denied"));
        assert!(read_error(&text, "file appears to be binary").contains("UTF-8"));
        assert!(read_error(&text, "file is 6000000 bytes (max 5242880)").contains("1 MiB"));
        assert!(
            read_error(&image, "choose an unchanged regular file up to 2097152 bytes")
                .contains("2 MiB")
        );
        assert!(read_error(&text, "not a regular file: /repo").contains("folder"));
        assert!(read_error(&text, "weird").contains("weird"));
    }

    #[test]
    fn viewer_languages_cover_common_names() {
        assert_eq!(language("/r/Makefile"), "make");
        assert_eq!(language("/r/src/lib.rs"), "rust");
        assert_eq!(language("/r/a.tsx"), "tsx");
        assert_eq!(language("/r/README"), "text");
        assert_eq!(language("/r/icon.svg"), "html");
    }
}
