//! R5.3 The design lint: surface files style through the system, never
//! locally. Scans every v2 surface under `src/ui/` (any file that reads
//! v2 tokens or components: `is_v2()`, `_v2`, `p.tokens`, `primitives::`,
//! `system::`) and counts, outside comments and `#[cfg(test)]` modules:
//!
//! | Rule | Matches |
//! |---|---|
//! | `raw-colour` | a colour literal: `rgb(0x…)`, `rgba(0x…)`, `hsla(<number>…)`, any `0xRRGGBB` / `0xRRGGBBAA` |
//! | `raw-px` | a pixel literal: `px(<number>)` (sizes come from `p.tokens`) |
//! | `shadow` | `.shadow(…)` / `.shadow_*()` (use `system::Level` / `elevated`) |
//! | `border-colour` | `.border_color(…)` (use a component, or `Level::edge`) |
//!
//! `rgb(p.text)` is fine: it converts a palette colour. The component layer
//! itself (`primitives.rs`, `system/`) is exempt; `tokens.rs` and
//! `appearance.rs` live outside `src/ui/`.
//!
//! [`ALLOWED`] is the one allowlist: per file and rule, how many hits
//! remain, with the reason. The test fails when a file has **more** hits
//! than allowed (a new violation) and when it has **fewer** (lower the
//! count, or delete the entry at zero), so the list only shrinks. Run with
//! `DESIGN_LINT_PRINT=1` to print the current table.
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub(crate) enum Rule {
    RawColour,
    RawPx,
    Shadow,
    BorderColour,
}

/// Why an entry may stay for now.
const TO_MIGRATE: &str =
    "v2-only surface: move onto system components and tokens (R5.4), then delete";
const MIXED: &str = "shared v1/v2 file: the v1 branch stays pixel-identical; \
     migrate the v2 branch's share (R5.4) and lower the count";

/// Paths (relative to the crate) the lint never scans, with the reason.
const EXEMPT: &[(&str, &str)] = &[
    (
        "src/ui/primitives.rs",
        "the component layer: the one place that styles",
    ),
    (
        "src/ui/system/",
        "the component layer: the one place that styles",
    ),
    ("src/ui/tests/", "UI tests, not surfaces"),
    ("src/ui/tests.rs", "UI tests, not surfaces"),
];

/// One allowlisted count.
struct Allowed {
    file: &'static str,
    rule: Rule,
    count: usize,
    reason: &'static str,
}

const fn a(file: &'static str, rule: Rule, count: usize, reason: &'static str) -> Allowed {
    Allowed {
        file,
        rule,
        count,
        reason,
    }
}

use Rule::{BorderColour as B, RawColour as C, RawPx as P, Shadow as S};

/// The allowlist, grouped by file. Migration agents: delete or lower
/// entries as surfaces move onto the system; never raise one.
#[rustfmt::skip]
const ALLOWED: &[Allowed] = &[
    // ── To migrate: v2-only surfaces ─────────────────────────────────────
    a("src/ui/capsule.rs", P, 1, TO_MIGRATE),
    a("src/ui/capsule.rs", S, 1, TO_MIGRATE),
    a("src/ui/capsule.rs", B, 2, TO_MIGRATE),
    a("src/ui/composer_v2.rs", S, 2, TO_MIGRATE),
    a("src/ui/composer_v2.rs", B, 1, TO_MIGRATE),
    a("src/ui/file_viewer/v2.rs", B, 1, TO_MIGRATE),
    a("src/ui/fleet.rs", S, 1, TO_MIGRATE),
    a("src/ui/fleet.rs", B, 4, TO_MIGRATE),
    a("src/ui/model_popover.rs", P, 2, TO_MIGRATE),
    a("src/ui/model_popover.rs", B, 3, TO_MIGRATE),
    a("src/ui/palette.rs", P, 1, TO_MIGRATE),
    a("src/ui/palette.rs", S, 1, TO_MIGRATE),
    a("src/ui/palette.rs", B, 3, TO_MIGRATE),
    a("src/ui/review/v2.rs", P, 1, TO_MIGRATE),
    a("src/ui/review/v2.rs", B, 6, TO_MIGRATE),
    a("src/ui/review/v2/page.rs", B, 3, TO_MIGRATE),
    a("src/ui/shell.rs", B, 5, TO_MIGRATE),
    a("src/ui/tasks/v2.rs", B, 1, TO_MIGRATE),
    a("src/ui/terminal/v2.rs", B, 1, TO_MIGRATE),
    a("src/ui/transcript_v2.rs", P, 2, TO_MIGRATE),
    a("src/ui/transcript_v2.rs", S, 1, TO_MIGRATE),
    a("src/ui/transcript_v2.rs", B, 4, TO_MIGRATE),
    // ── To migrate: shared v1/v2 files (the v1 share stays) ──────────────
    a("src/ui/async_questions.rs", P, 4, MIXED),
    a("src/ui/async_questions.rs", S, 1, MIXED),
    a("src/ui/async_questions.rs", B, 2, MIXED),
    a("src/ui/attachments.rs", P, 8, MIXED),
    a("src/ui/attachments.rs", B, 3, MIXED),
    a("src/ui/cache.rs", P, 2, MIXED),
    a("src/ui/cache.rs", B, 2, MIXED),
    a("src/ui/children.rs", P, 28, MIXED),
    a("src/ui/children.rs", S, 2, MIXED),
    a("src/ui/children.rs", B, 5, MIXED),
    a("src/ui/chrome.rs", C, 4, MIXED),
    a("src/ui/chrome.rs", P, 53, MIXED),
    a("src/ui/chrome.rs", S, 4, MIXED),
    a("src/ui/chrome.rs", B, 6, MIXED),
    a("src/ui/cleanup.rs", B, 2, MIXED),
    a("src/ui/conversation.rs", P, 1, MIXED),
    a("src/ui/features.rs", P, 29, MIXED),
    a("src/ui/features.rs", S, 1, MIXED),
    a("src/ui/features.rs", B, 6, MIXED),
    a("src/ui/file_viewer.rs", P, 32, MIXED),
    a("src/ui/file_viewer.rs", S, 2, MIXED),
    a("src/ui/file_viewer.rs", B, 8, MIXED),
    a("src/ui/fleet_card.rs", P, 21, MIXED),
    a("src/ui/fleet_card.rs", B, 4, MIXED),
    a("src/ui/gauge.rs", P, 7, MIXED),
    a("src/ui/handoff.rs", P, 13, MIXED),
    a("src/ui/handoff.rs", S, 1, MIXED),
    a("src/ui/handoff.rs", B, 1, MIXED),
    a("src/ui/hosts.rs", P, 8, MIXED),
    a("src/ui/hosts.rs", B, 5, MIXED),
    a("src/ui/jobs.rs", P, 27, MIXED),
    a("src/ui/jobs.rs", S, 1, MIXED),
    a("src/ui/jobs.rs", B, 3, MIXED),
    a("src/ui/launch.rs", P, 32, MIXED),
    a("src/ui/launch.rs", S, 2, MIXED),
    a("src/ui/launch.rs", B, 11, MIXED),
    a("src/ui/link_menu.rs", P, 7, MIXED),
    a("src/ui/link_menu.rs", S, 3, MIXED),
    a("src/ui/link_menu.rs", B, 3, MIXED),
    a("src/ui/markdown.rs", P, 3, MIXED),
    a("src/ui/markdown.rs", B, 1, MIXED),
    a("src/ui/navigation.rs", P, 11, MIXED),
    a("src/ui/navigation.rs", B, 2, MIXED),
    a("src/ui/plan.rs", P, 15, MIXED),
    a("src/ui/plan.rs", S, 2, MIXED),
    a("src/ui/plan.rs", B, 2, MIXED),
    a("src/ui/portrait.rs", P, 5, MIXED),
    a("src/ui/portrait.rs", S, 1, MIXED),
    a("src/ui/project_links.rs", P, 4, MIXED),
    a("src/ui/project_links.rs", B, 6, MIXED),
    a("src/ui/projects.rs", P, 19, MIXED),
    a("src/ui/projects.rs", B, 4, MIXED),
    a("src/ui/questions.rs", P, 17, MIXED),
    a("src/ui/questions.rs", S, 4, MIXED),
    a("src/ui/questions.rs", B, 12, MIXED),
    a("src/ui/queue.rs", P, 11, MIXED),
    a("src/ui/queue.rs", S, 2, MIXED),
    a("src/ui/queue.rs", B, 2, MIXED),
    a("src/ui/recent.rs", P, 32, MIXED),
    a("src/ui/recent.rs", B, 1, MIXED),
    a("src/ui/remote.rs", C, 2, MIXED),
    a("src/ui/remote.rs", P, 5, MIXED),
    a("src/ui/review.rs", P, 18, MIXED),
    a("src/ui/review.rs", S, 1, MIXED),
    a("src/ui/review.rs", B, 2, MIXED),
    a("src/ui/scratch.rs", P, 12, MIXED),
    a("src/ui/scratch.rs", B, 2, MIXED),
    a("src/ui/session_tools.rs", P, 21, MIXED),
    a("src/ui/session_tools.rs", B, 5, MIXED),
    a("src/ui/settings.rs", C, 1, MIXED),
    a("src/ui/settings.rs", P, 36, MIXED),
    a("src/ui/settings.rs", B, 5, MIXED),
    a("src/ui/sidebar.rs", P, 42, MIXED),
    a("src/ui/sidebar.rs", B, 6, MIXED),
    a("src/ui/slash.rs", P, 3, MIXED),
    a("src/ui/slash.rs", B, 1, MIXED),
    a("src/ui/states.rs", P, 8, MIXED),
    a("src/ui/states.rs", B, 1, MIXED),
    a("src/ui/tasks.rs", P, 15, MIXED),
    a("src/ui/tasks.rs", B, 4, MIXED),
    a("src/ui/terminal.rs", C, 32, MIXED),
    a("src/ui/terminal.rs", P, 22, MIXED),
    a("src/ui/terminal.rs", B, 5, MIXED),
    a("src/ui/transcript.rs", P, 36, MIXED),
    a("src/ui/transcript.rs", B, 7, MIXED),
    a("src/ui/usage.rs", P, 31, MIXED),
    a("src/ui/usage.rs", S, 2, MIXED),
    a("src/ui/usage.rs", B, 7, MIXED),
    a("src/ui/work.rs", P, 25, MIXED),
    a("src/ui/work.rs", B, 3, MIXED),
];

const SURFACE_MARKERS: [&str; 5] = ["is_v2()", "_v2", "p.tokens", "primitives::", "system::"];

fn crate_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
}

fn rust_files(dir: &Path, out: &mut Vec<PathBuf>) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            rust_files(&path, out);
        } else if path.extension().is_some_and(|e| e == "rs") {
            out.push(path);
        }
    }
}

/// The source with `#[cfg(test)] mod …` (and everything after it) cut off.
fn without_tests(src: &str) -> &str {
    let mut offset = 0;
    let mut lines = src.split_inclusive('\n').peekable();
    while let Some(line) = lines.next() {
        if line.trim() == "#[cfg(test)]"
            && lines
                .peek()
                .is_some_and(|next| next.trim_start().starts_with("mod "))
        {
            return &src[..offset];
        }
        offset += line.len();
    }
    src
}

/// A line without its `//` comment (when no string opens before it).
fn code_of(line: &str) -> &str {
    if line.trim_start().starts_with("//") {
        return "";
    }
    match line.find("//") {
        Some(at) if !line[..at].contains('"') => &line[..at],
        _ => line,
    }
}

fn ident(c: u8) -> bool {
    c.is_ascii_alphanumeric() || c == b'_'
}

/// After `at`, optional spaces then an optional `-`, then a digit (or `0x`).
fn number_follows(s: &[u8], mut at: usize) -> bool {
    while at < s.len() && s[at] == b' ' {
        at += 1;
    }
    if at < s.len() && s[at] == b'-' {
        at += 1;
    }
    at < s.len() && s[at].is_ascii_digit()
}

/// Hits of each rule on one line of code.
fn scan(code: &str) -> BTreeMap<Rule, usize> {
    let s = code.as_bytes();
    let mut hits = BTreeMap::new();
    let mut i = 0;
    while i < s.len() {
        let rest = &code[i..];
        let boundary = i == 0 || !ident(s[i - 1]);
        let mut hit = None;
        for (name, rule) in [
            ("rgba(", Rule::RawColour),
            ("rgb(", Rule::RawColour),
            ("hsla(", Rule::RawColour),
            ("px(", Rule::RawPx),
        ] {
            if boundary && rest.starts_with(name) && number_follows(s, i + name.len()) {
                // Consume a `0x` too, so `rgb(0x…)` counts once.
                let mut len = name.len();
                while code[i + len..].starts_with(' ') {
                    len += 1;
                }
                if code[i + len..].starts_with("0x") {
                    len += 2;
                }
                hit = Some((rule, len));
                break;
            }
        }
        if hit.is_none() && boundary && rest.starts_with("0x") {
            let digits = rest[2..]
                .bytes()
                .take_while(|b| b.is_ascii_hexdigit())
                .count();
            let after = i + 2 + digits;
            if (digits == 6 || digits == 8) && (after >= s.len() || !ident(s[after])) {
                hit = Some((Rule::RawColour, 2 + digits));
            }
        }
        if hit.is_none() && rest.starts_with(".border_color(") {
            hit = Some((Rule::BorderColour, ".border_color(".len()));
        }
        if hit.is_none() && rest.starts_with(".shadow") {
            let tail = &rest[".shadow".len()..];
            let name = if let Some(t) = tail.strip_prefix('_') {
                1 + t
                    .bytes()
                    .take_while(|b| b.is_ascii_lowercase() || b.is_ascii_digit())
                    .count()
            } else {
                0
            };
            if tail[name..].starts_with('(') {
                hit = Some((Rule::Shadow, ".shadow".len() + name + 1));
            }
        }
        match hit {
            Some((rule, len)) => {
                *hits.entry(rule).or_insert(0) += 1;
                i += len;
            }
            None => i += rest.chars().next().map_or(1, char::len_utf8),
        }
    }
    hits
}

/// Every scanned surface's hits: (file, rule) → count.
fn survey() -> BTreeMap<(String, Rule), usize> {
    let root = crate_dir();
    let mut files = Vec::new();
    rust_files(&root.join("src/ui"), &mut files);
    let mut out = BTreeMap::new();
    for path in files {
        let rel = path
            .strip_prefix(&root)
            .unwrap()
            .to_string_lossy()
            .replace('\\', "/");
        if EXEMPT.iter().any(|(prefix, _)| rel.starts_with(prefix)) {
            continue;
        }
        let src = std::fs::read_to_string(&path).unwrap();
        if !SURFACE_MARKERS.iter().any(|m| src.contains(m)) {
            continue;
        }
        for line in without_tests(&src).lines() {
            for (rule, n) in scan(code_of(line)) {
                *out.entry((rel.clone(), rule)).or_insert(0) += n;
            }
        }
    }
    out
}

#[test]
fn the_scanner_sees_each_rule() {
    let hits = scan(
        r#"div().bg(rgb(0x1d2021)).w(px(12.)).h(px(t.space.md)).shadow(x).border_color(rgb(p.border)).shadow_md() hsla(0.5, 0., 0., 1.) let c = 0xffffff;"#,
    );
    assert_eq!(hits.get(&Rule::RawColour), Some(&3));
    assert_eq!(hits.get(&Rule::RawPx), Some(&1));
    assert_eq!(hits.get(&Rule::Shadow), Some(&2));
    assert_eq!(hits.get(&Rule::BorderColour), Some(&1));
    assert!(!scan("rgb(p.text) px(t.space.md) .shadow(elevation(p))").contains_key(&Rule::RawPx));
    assert_eq!(code_of("    // px(12.)"), "");
    assert_eq!(
        without_tests("fn a() {}\n#[cfg(test)]\nmod tests {\n px(1.)\n}\n"),
        "fn a() {}\n"
    );
}

#[test]
fn surfaces_style_only_through_the_design_system() {
    let actual = survey();
    let mut allowed: BTreeMap<(String, Rule), (usize, &str)> = BTreeMap::new();
    for entry in ALLOWED {
        let key = (entry.file.to_owned(), entry.rule);
        assert!(
            allowed.insert(key, (entry.count, entry.reason)).is_none(),
            "{} {:?} is listed twice",
            entry.file,
            entry.rule
        );
        assert!(
            entry.count > 0,
            "{} {:?}: delete a zero entry",
            entry.file,
            entry.rule
        );
    }
    if std::env::var_os("DESIGN_LINT_PRINT").is_some() {
        for ((file, rule), n) in &actual {
            let code = match rule {
                Rule::RawColour => "C",
                Rule::RawPx => "P",
                Rule::Shadow => "S",
                Rule::BorderColour => "B",
            };
            println!("    a(\"{file}\", {code}, {n}, REASON),");
        }
    }
    let mut problems = Vec::new();
    for ((file, rule), &n) in &actual {
        match allowed.get(&(file.clone(), *rule)) {
            None => problems.push(format!(
                "{file}: {n} new {rule:?} hit(s): use a system component or token \
                 (docs/design-system.md)"
            )),
            Some(&(max, _)) if n > max => problems.push(format!(
                "{file}: {rule:?} {n} hits, {max} allowed: a new violation"
            )),
            Some(&(max, _)) if n < max => problems.push(format!(
                "{file}: {rule:?} {n} hits, {max} allowed: lower the allowlist to {n}"
            )),
            _ => {}
        }
    }
    for ((file, rule), (max, _)) in &allowed {
        if !actual.contains_key(&(file.clone(), *rule)) {
            problems.push(format!(
                "{file}: {rule:?} no longer occurs ({max} allowed): delete the entry"
            ));
        }
    }
    assert!(
        problems.is_empty(),
        "design lint ({} problems):\n{}",
        problems.len(),
        problems.join("\n")
    );
}
