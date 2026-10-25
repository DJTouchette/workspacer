//! File paths written in plain chat text (`see src/lib.rs:12`) and inline code
//! (`` `package.json` ``), so they open like a written Markdown link. A port of
//! the desktop's `lib/filePathDetect.ts`; keep the two in step.
//!
//! Precision beats recall: a false positive turns prose into a dead link, so
//! prose needs a separator (absolute, `./`-relative, or several segments
//! ending in a file-like name), and only a whole code span with a well-known
//! code extension or file name may be a single segment. Detection is lexical;
//! [`crate::links::classify`] resolves the span (and its `:line[:col]`) on
//! click, and the hub's read decides whether the file exists.
use std::ops::Range;

/// Longest token that can still be a path. Also the scanner's bail-out: the
/// text is agent-written, so nothing does per-character work past this.
const MAX_TOKEN: usize = 512;
/// Prose longer than this is not scanned at all.
const MAX_TEXT: usize = 50_000;
/// Clause punctuation peeled off a token's end ("(see src/a.rs).").
const MAX_TRAILING: usize = 8;

/// Extensions that make a bare single-segment name path-like. TLDs (`com`,
/// `org`, `ai`, …) are left out so domains never link.
const SINGLE_SEGMENT_EXTS: &[&str] = &[
    "ts", "tsx", "js", "jsx", "mjs", "cjs", "json", "jsonc", "yaml", "yml", "toml", "ini", "md",
    "markdown", "txt", "log", "lock", "rs", "go", "py", "rb", "java", "kt", "swift", "c", "h",
    "cc", "cpp", "hpp", "cs", "php", "ex", "exs", "erl", "hs", "lua", "zig", "css", "scss", "less",
    "html", "htm", "svg", "xml", "sql", "sh", "bash", "zsh", "fish", "ps1", "bat", "cmd", "gradle",
    "proto", "graphql", "tf", "nix", "env", "conf", "cfg", "png", "jpg", "jpeg", "gif", "webp",
    "csproj", "sln",
];

/// Extensionless files worth linking by name.
const BARE_NAMES: &[&str] = &[
    "Makefile",
    "makefile",
    "Dockerfile",
    "Justfile",
    "justfile",
    "Rakefile",
    "Gemfile",
    "Procfile",
    "LICENSE",
    "CHANGELOG",
    "Vagrantfile",
];

/// TLDs that mark a first segment as a domain (`github.com/x/y.ts`).
const DOMAIN_TLDS: &[&str] = &[
    "com", "org", "net", "io", "dev", "ai", "co", "app", "sh", "gg", "xyz", "me", "us", "uk", "ca",
    "de", "fr", "edu", "gov",
];

/// Byte ranges of `text` that are file paths. `code` is a whole inline code
/// span: it links only if all of it (trimmed) is one path.
pub fn spans(text: &str, code: bool) -> Vec<Range<usize>> {
    if code {
        let start = text.len() - text.trim_start().len();
        let token = text.trim();
        return std::iter::once(start..start + token.len())
            .filter(|_| path_like(token, true))
            .collect();
    }
    if text.len() > MAX_TEXT || !text.contains(['/', '\\']) {
        return Vec::new();
    }
    let mut out = Vec::new();
    let mut rest = text;
    let mut offset = 0;
    while !rest.is_empty() {
        let run_start = rest
            .find(|c: char| !c.is_whitespace())
            .unwrap_or(rest.len());
        let run_len = rest[run_start..]
            .find(char::is_whitespace)
            .unwrap_or(rest.len() - run_start);
        let run = &rest[run_start..run_start + run_len];
        if run.contains(['/', '\\'])
            && let Some(range) = token_in(run)
        {
            let base = offset + run_start;
            out.push(base + range.start..base + range.end);
        }
        offset += run_start + run_len;
        rest = &rest[run_start + run_len..];
    }
    out
}

/// The path inside one whitespace-free run, after leading brackets/quotes and
/// (only if needed) trailing clause punctuation.
fn token_in(run: &str) -> Option<Range<usize>> {
    let start = run.len()
        - run
            .trim_start_matches(['(', '[', '\'', '"', '`', '{'])
            .len();
    let mut token = &run[start..];
    if token.len() > MAX_TOKEN + MAX_TRAILING {
        return None;
    }
    for _ in 0..=MAX_TRAILING {
        if path_like(token, false) {
            return Some(start..start + token.len());
        }
        token =
            token.strip_suffix(['.', ',', ';', '!', '?', ')', ']', '}', '\'', '"', '`', ':'])?;
    }
    None
}

/// Whether a whole whitespace-free token is a file path, with an optional
/// `:line[:col]` suffix. `code` allows `package.json`-style single names.
fn path_like(token: &str, code: bool) -> bool {
    if token.is_empty()
        || token.len() > MAX_TOKEN
        || token.contains(char::is_whitespace)
        || token.contains("://")
        || token.starts_with("www.")
        || token.starts_with('-')
        || token.contains("${")
        || token.contains(['*', '<', '>', '|', '"'])
    {
        return false;
    }
    let path = strip_line(token);
    if path.is_empty() || path.ends_with(['/', '\\']) {
        return false;
    }
    let drive = drive_prefix(path);
    if path.contains(':') && !drive {
        return false;
    }
    let base = path.rsplit(['/', '\\']).next().unwrap_or(path);
    let separated = path.contains(['/', '\\']);
    if path.starts_with('/') || path.starts_with("\\\\") || drive {
        return separated && file_like(base);
    }
    if path.starts_with("./") || path.starts_with("../") {
        return file_like(base);
    }
    if separated {
        let first = path.split(['/', '\\']).next().unwrap_or("");
        return path
            .starts_with(|c: char| c.is_alphanumeric() || matches!(c, '_' | '.' | '@' | '-'))
            && !domain(first)
            && file_like(base);
    }
    if !code {
        return false;
    }
    BARE_NAMES.contains(&path)
        || extension(path).is_some_and(|ext| {
            path.len() > ext.len() + 1
                && SINGLE_SEGMENT_EXTS
                    .iter()
                    .any(|e| e.eq_ignore_ascii_case(ext))
        })
}

/// `token` without a trailing `:line` or `:line:col` (1–6 digits each).
fn strip_line(token: &str) -> &str {
    let digits = |s: &str| !s.is_empty() && s.len() <= 6 && s.bytes().all(|b| b.is_ascii_digit());
    let Some((head, last)) = token.rsplit_once(':') else {
        return token;
    };
    if !digits(last) {
        return token;
    }
    match head.rsplit_once(':') {
        Some((path, line)) if digits(line) => path,
        _ => head,
    }
}

fn drive_prefix(path: &str) -> bool {
    let b = path.as_bytes();
    b.len() > 2 && b[0].is_ascii_alphabetic() && b[1] == b':' && matches!(b[2], b'/' | b'\\')
}

/// A known extensionless name, or a letter-led alphanumeric extension of up
/// to eight characters (so `v1.2` is not a file).
fn file_like(base: &str) -> bool {
    BARE_NAMES.contains(&base) || extension(base).is_some()
}

fn extension(name: &str) -> Option<&str> {
    let (_, ext) = name.rsplit_once('.')?;
    (!ext.is_empty()
        && ext.len() <= 8
        && ext.starts_with(|c: char| c.is_ascii_alphabetic())
        && ext.bytes().all(|b| b.is_ascii_alphanumeric()))
    .then_some(ext)
}

fn domain(segment: &str) -> bool {
    segment.rsplit_once('.').is_some_and(|(name, tld)| {
        !name.is_empty()
            && name
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'_' | b'-'))
            && DOMAIN_TLDS.iter().any(|t| t.eq_ignore_ascii_case(tld))
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn prose(text: &str) -> Vec<&str> {
        spans(text, false).into_iter().map(|r| &text[r]).collect()
    }
    fn code(text: &str) -> Option<&str> {
        spans(text, true).first().map(|r| &text[r.clone()])
    }

    #[test]
    fn prose_links_separated_paths_and_keeps_line_suffixes() {
        assert_eq!(
            prose("Edit src/ui/chat.rs:42 and /etc/hosts.conf, then ./run.sh."),
            ["src/ui/chat.rs:42", "/etc/hosts.conf", "./run.sh"]
        );
        assert_eq!(prose("(see ../docs/README.md)."), ["../docs/README.md"]);
        assert_eq!(prose("at src/lib.rs:12:5: error"), ["src/lib.rs:12:5"]);
        assert_eq!(prose(r"open C:\repo\app.cs now"), [r"C:\repo\app.cs"]);
        assert_eq!(
            prose("`apps/native/Cargo.toml`"),
            ["apps/native/Cargo.toml"]
        );
        assert_eq!(prose("app/[id]/page.tsx"), ["app/[id]/page.tsx"]);
        assert_eq!(prose("assets/logo.png"), ["assets/logo.png"]);
        assert_eq!(prose("docker/Dockerfile"), ["docker/Dockerfile"]);
    }

    #[test]
    fn prose_never_links_words_urls_domains_or_directories() {
        for text in [
            "package.json alone",
            "notifications.post and v1.2",
            "https://example.com/a.ts",
            "www.example.com/a.ts",
            "github.com/owner/repo.git",
            "src/ui/ and and/or",
            "--flag=a/b.rs",
            "src/**/*.rs",
            "${HOME}/a.rs",
            "~/notes.md",
            "scheme:a/b.rs",
            "1/2 cups",
        ] {
            assert!(prose(text).is_empty(), "{text:?} -> {:?}", prose(text));
        }
    }

    #[test]
    fn code_spans_must_be_one_path() {
        assert_eq!(code("package.json"), Some("package.json"));
        assert_eq!(code(" Makefile "), Some("Makefile"));
        assert_eq!(code("src/main.rs:9"), Some("src/main.rs:9"));
        assert_eq!(code("Program.cs"), Some("Program.cs"));
        for text in [
            "example.com",
            "cargo test",
            "Vec<String>",
            "foo.bar()",
            ".env",
            "x.y",
        ] {
            assert_eq!(code(text), None, "{text:?}");
        }
    }

    #[test]
    fn hostile_input_stays_bounded() {
        let long = "a/".repeat(40_000);
        assert!(prose(&long).is_empty());
        let run = format!("{}.rs", "x/".repeat(400));
        assert!(prose(&run).is_empty(), "longer than a path can be");
        assert!(prose("é/ü.rs and ü/a.rs").len() == 2);
    }
}
