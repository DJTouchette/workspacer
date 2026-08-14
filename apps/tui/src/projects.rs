//! What a project looks like at a glance.
//!
//! Every row in the agent list is the same shape, so a fleet spanning several
//! repos is a wall of identical rows and the only way to tell which repo an
//! agent belongs to is to read its path. This resolves a directory to a small
//! visual identity — initials in a stable colour — so that stops being true.
//!
//! The important property: it works with NO configuration. An unconfigured
//! project still gets stable initials and a stable colour derived from its own
//! path, so the fleet is legible the first time you look at it, and the TUI
//! stays useful when it can't reach `config.projects` at all (`--direct`, or a
//! hub that never answers). `config.projects` only records the parts a human
//! chose to override.
//!
//! TWIN: `apps/desktop/src/renderer/src/lib/projectIdentity.ts` and
//! `projectKey.ts`. The palette, the FNV-1a hash and the initials rules are
//! ported exactly, and the golden vectors in the tests below are pinned against
//! that implementation — a change to either side that isn't made on both fails
//! here. The one deliberate divergence is documented on [`resolve_project`].
//!
//! Icons are deliberately NOT rendered. `config.projects` can carry an emoji
//! (`icon`) or a downloaded favicon (`favicon`/`iconFile`); a terminal can draw
//! neither reliably — an image not at all, and an emoji occupies one or two
//! cells depending on the terminal, the font and the emoji, which would ragged
//! the column the mark exists to keep alignable. The initials always render in
//! exactly two columns.

use std::collections::HashMap;

use ratatui::style::Color;
use serde::Deserialize;

/// One `config.projects` entry — the parts of it a terminal can use.
///
/// The desktop's `ProjectIdentity` also carries `favicon` / `iconFile` (images),
/// `favourite` / `lastOpened` (picker ordering) and `plugins` (per-project
/// plugin settings). None of those mean anything here, and serde ignores
/// unknown keys, so this reads the same document without knowing about them.
#[derive(Debug, Clone, Default, Deserialize)]
pub struct ProjectIdentity {
    /// Display name. Absent/blank falls back to the directory's basename.
    #[serde(default)]
    pub label: Option<String>,
    /// Explicit badge tint. Absent/blank falls back to the derived palette.
    #[serde(default)]
    pub color: Option<String>,
}

/// `config.projects`: normalized cwd → the human's overrides for it.
pub type Projects = HashMap<String, ProjectIdentity>;

/// Pull `projects` out of a whole config document (what the hub's `config.get`
/// capability hands back).
///
/// A missing or non-object `projects` is normal (no project has ever been
/// configured) and yields an empty map. An individual entry that doesn't
/// deserialize is skipped rather than discarding the whole map — one hand-typo'd
/// entry must not cost every other project its identity.
pub fn from_config(doc: &serde_json::Value) -> Projects {
    let Some(map) = doc.get("projects").and_then(|p| p.as_object()) else {
        return Projects::new();
    };
    map.iter()
        .filter_map(|(k, v)| {
            let entry: ProjectIdentity = serde_json::from_value(v.clone()).ok()?;
            Some((k.clone(), entry))
        })
        .collect()
}

/// The palette derived marks are drawn from. Deliberately a fixed list rather
/// than a free hue rotation: these sit beside the status colours the sidebar
/// already uses (`ok`/`warn`/`bad`/`accent`) that carry meaning, so a derived
/// tint must never land close enough to be mistaken for one. Hues are spread and
/// kept mid-saturation so they read as identity, not state.
///
/// Order is load-bearing — it is the index space of [`fnv1a`] — and must match
/// the desktop's `PALETTE` entry for entry.
const PALETTE: [&str; 8] = [
    "#6b8afd", // indigo
    "#c084fc", // violet
    "#f472b6", // pink
    "#fb923c", // orange — distinct from the warn amber
    "#2dd4bf", // teal
    "#38bdf8", // sky
    "#a3a3f5", // periwinkle
    "#e879a6", // rose
];

/// A stable 32-bit hash of a string (FNV-1a). Same path → same colour, on every
/// machine and across restarts, which is the whole point of deriving it.
///
/// Hashes UTF-16 code units, not bytes or chars: the twin is JavaScript and
/// feeds it `charCodeAt`, so anything else would agree on ASCII paths and drift
/// silently on a path with an accent in it.
fn fnv1a(s: &str) -> u32 {
    let mut h: u32 = 0x811c_9dc5;
    for unit in s.encode_utf16() {
        h ^= unit as u32;
        h = h.wrapping_mul(0x0100_0193);
    }
    h
}

/// The stable config key for a workspace directory: backslashes normalized to
/// forward slashes, trailing separators dropped.
///
/// Normalization, not canonicalization — it does not resolve symlinks or `..`,
/// so two genuinely different spellings of one directory still key differently.
/// Case is handled separately, on lookup: see [`resolve_project_key`].
pub fn project_key(cwd: &str) -> String {
    let normalized = cwd.replace('\\', "/");
    normalized.trim_end_matches('/').to_string()
}

/// The key to read for `cwd`, honouring an existing entry that differs only by
/// case.
///
/// Windows and macOS paths are case-insensitive and the product does not spell
/// them consistently (an agent cwd arrives as `c:/users/me/repo` where the
/// config was written `C:/Users/me/repo`). Those are one directory, so keying
/// them separately would silently strand a project's configured identity.
///
/// Lookup-time rather than write-time on purpose: lowercasing inside
/// [`project_key`] would be wrong on Linux, where `~/Repo` and `~/repo` really
/// are different directories.
pub fn resolve_project_key(map: &Projects, cwd: &str) -> String {
    resolve_project_key_for(map, cwd, std::env::consts::OS)
}

/// [`resolve_project_key`] with the platform as a parameter, so the
/// case-insensitive branch is testable on any host — the same shape as
/// [`crate::config::config_dir_for`].
pub fn resolve_project_key_for(map: &Projects, cwd: &str, os: &str) -> String {
    let key = project_key(cwd);
    if map.contains_key(&key) {
        return key;
    }
    // Only where the filesystem is case-insensitive: doing this on Linux would
    // merge two directories that legitimately differ.
    if !matches!(os, "windows" | "macos") {
        return key;
    }
    let lowered = key.to_lowercase();
    map.keys()
        .find(|existing| existing.to_lowercase() == lowered)
        .cloned()
        .unwrap_or(key)
}

/// The last path segment — the name a human calls the project.
pub fn basename_of(dir: &str) -> String {
    project_key(dir)
        .rsplit('/')
        .next()
        .unwrap_or_default()
        .to_string()
}

/// One or two characters for a name. A hyphenated, dotted or underscored name
/// gives up its word initials (`work-spacer` → `WS`), which distinguishes
/// sibling repos that share a prefix far better than the first two letters would
/// (`api-gateway` and `api-worker` are `AG` and `AW`, not both `AP`).
pub fn initials_of(name: &str) -> String {
    // JS splits on `[\s._-]+`; `char::is_whitespace` is the Rust spelling of `\s`.
    let words: Vec<&str> = name
        .split(|c: char| c.is_whitespace() || c == '.' || c == '_' || c == '-')
        .filter(|w| !w.is_empty())
        .collect();
    let Some(first) = words.first() else {
        return "?".to_string();
    };
    if words.len() == 1 {
        // A camelCase single word still has a second word inside it. ASCII-only,
        // like the twin's `/^([a-z]+)([A-Z][a-z]*)/`.
        let mut chars = first.chars();
        let lead = chars.next().filter(char::is_ascii_lowercase);
        if let Some(lead) = lead {
            if let Some(upper) = chars.find(|c| !c.is_ascii_lowercase()) {
                if upper.is_ascii_uppercase() {
                    return format!("{lead}{upper}").to_uppercase();
                }
            }
        }
        return first.chars().take(2).collect::<String>().to_uppercase();
    }
    let second = words[1];
    match (first.chars().next(), second.chars().next()) {
        (Some(a), Some(b)) => format!("{a}{b}").to_uppercase(),
        _ => "?".to_string(),
    }
}

/// What to actually draw for a directory.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResolvedProject {
    /// Display name — the configured label, else the directory's basename.
    pub label: String,
    /// One or two characters, drawn in [`tint`](Self::tint).
    pub initials: String,
    /// The badge tint, as a colour string [`crate::theme::parse_color`] accepts.
    pub color: String,
}

impl ResolvedProject {
    /// The mark as a list draws it: the initials padded to a fixed two columns,
    /// plus a separating space. Fixed width on purpose — a one-letter project
    /// (`w` → `W`) beside a two-letter one would otherwise shift every name
    /// after it by a column.
    pub fn mark(&self) -> String {
        let pad = 2usize.saturating_sub(self.initials.chars().count());
        format!("{}{} ", self.initials, " ".repeat(pad))
    }

    /// The mark's colour. [`resolve_project`] only ever stores a string this
    /// parses, so the fallback is unreachable; `Reset` is the inert answer if it
    /// ever isn't.
    pub fn tint(&self) -> Color {
        crate::theme::parse_color(&self.color).unwrap_or(Color::Reset)
    }
}

/// A configured string that is present and not just whitespace. Blank counts as
/// unset on both sides of the seam.
fn nonblank(v: Option<&String>) -> Option<&str> {
    v.map(|s| s.trim()).filter(|s| !s.is_empty())
}

/// Resolve a directory to what should be drawn for it. `projects` is
/// `config.projects`; a missing entry is normal and fully supported — that is
/// the whole design, since the TUI often cannot reach the config at all.
///
/// Returns `None` only for an empty directory (a session claudemon reported no
/// cwd for): there is no project to name, and a `?` mark would be noise.
///
/// One divergence from the twin: a configured `color` the terminal cannot render
/// (a CSS spelling like `rgb(...)` or a `var(--x)` token, all legal in the
/// desktop) is treated as unset and the derived colour is used, rather than
/// carried through to a renderer that would drop it. Blank counts as unset on
/// both sides.
pub fn resolve_project(dir: &str, projects: &Projects) -> Option<ResolvedProject> {
    if dir.is_empty() {
        return None;
    }
    let key = resolve_project_key(projects, dir);
    let entry = projects.get(&key);

    let base = basename_of(dir);
    let label = nonblank(entry.and_then(|e| e.label.as_ref()))
        .map(str::to_string)
        .unwrap_or(base);
    // Derived from the KEY, not the label: renaming a project should not
    // re-colour it out from under you.
    let derived = PALETTE[fnv1a(&key) as usize % PALETTE.len()];
    let color = nonblank(entry.and_then(|e| e.color.as_ref()))
        .filter(|c| crate::theme::parse_color(c).is_some())
        .unwrap_or(derived);

    Some(ResolvedProject {
        // Initials follow the LABEL, so renaming a project renames its mark too.
        initials: initials_of(&label),
        label,
        color: color.to_string(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn resolve(dir: &str) -> ResolvedProject {
        resolve_project(dir, &Projects::new()).expect("a non-empty dir resolves")
    }

    fn configured(key: &str, label: Option<&str>, color: Option<&str>) -> Projects {
        let mut m = Projects::new();
        m.insert(
            key.to_string(),
            ProjectIdentity {
                label: label.map(str::to_string),
                color: color.map(str::to_string),
            },
        );
        m
    }

    /// The golden vectors, pinned against the desktop twin
    /// (`projectIdentity.ts`). These are the whole contract: if either side's
    /// palette order, hash or initials rule drifts, one of these fails rather
    /// than the same repo quietly wearing two different colours in two clients.
    #[test]
    fn golden_marks_match_the_desktop_twin() {
        let cases: &[(&str, &str, &str)] = &[
            // Sibling repos sharing a prefix stay distinct in BOTH channels —
            // this is the pair the whole initials rule exists for.
            ("/w/api-gateway", "AG", "#fb923c"),
            ("/w/api-worker", "AW", "#c084fc"),
            ("/w/workspacer", "WO", "#c084fc"),
            ("/w/a", "A", "#e879a6"),
            ("/w/b", "B", "#f472b6"),
            ("/w/c", "C", "#38bdf8"),
            ("/w/d", "D", "#6b8afd"),
            ("/w/e", "E", "#fb923c"),
        ];
        for (dir, initials, color) in cases {
            let p = resolve(dir);
            assert_eq!(p.initials, *initials, "initials of {dir}");
            assert_eq!(p.color, *color, "colour of {dir}");
        }
    }

    /// The hash itself, pinned separately from the palette so a palette edit and
    /// a hash bug can't be mistaken for each other.
    #[test]
    fn fnv1a_matches_the_javascript_twin() {
        assert_eq!(fnv1a(""), 2_166_136_261, "the FNV offset basis");
        assert_eq!(fnv1a("/w/api-gateway"), 3_406_843_115);
        assert_eq!(fnv1a("/w/api-worker"), 3_548_312_177);
        assert_eq!(fnv1a("/home/djtouchette/Work/worky/workspacer"), 445_766_688);
    }

    /// The colour follows the PATH and the initials follow the LABEL. That split
    /// is deliberate: renaming a project should rename its mark without
    /// re-colouring it out from under you.
    #[test]
    fn a_label_renames_the_mark_but_never_recolours_it() {
        let derived = resolve("/w/api-gateway");
        let named = resolve_project(
            "/w/api-gateway",
            &configured("/w/api-gateway", Some("Public API"), None),
        )
        .unwrap();
        assert_eq!(named.label, "Public API");
        assert_eq!(named.initials, "PA", "initials follow the label");
        assert_eq!(named.color, derived.color, "colour follows the path");
    }

    #[test]
    fn an_explicit_colour_wins_and_a_blank_one_does_not() {
        let set = resolve_project(
            "/w/api-gateway",
            &configured("/w/api-gateway", None, Some("#123456")),
        )
        .unwrap();
        assert_eq!(set.color, "#123456");
        assert_eq!(set.tint(), Color::Rgb(0x12, 0x34, 0x56));

        // Blank counts as unset on both sides of the seam.
        for blank in ["", "   "] {
            let p = resolve_project(
                "/w/api-gateway",
                &configured("/w/api-gateway", Some(blank), Some(blank)),
            )
            .unwrap();
            assert_eq!(p.label, "api-gateway", "blank label is unset");
            assert_eq!(p.color, "#fb923c", "blank colour is unset");
        }
    }

    /// The documented divergence: the desktop can render a CSS colour, a
    /// terminal cannot. Falling back to the derived tint keeps a mark on screen
    /// instead of dropping the colour (or the whole span) on the floor.
    #[test]
    fn a_colour_the_terminal_cannot_render_falls_back_to_the_derived_one() {
        let p = resolve_project(
            "/w/api-gateway",
            &configured("/w/api-gateway", None, Some("rgb(1, 2, 3)")),
        )
        .unwrap();
        assert_eq!(p.color, "#fb923c");
        // A colour it CAN render, that isn't hex, still wins.
        let named = resolve_project(
            "/w/api-gateway",
            &configured("/w/api-gateway", None, Some("cyan")),
        )
        .unwrap();
        assert_eq!(named.tint(), Color::Cyan);
    }

    #[test]
    fn initials_split_on_every_word_separator() {
        assert_eq!(initials_of("api-gateway"), "AG");
        assert_eq!(initials_of("work_spacer"), "WS");
        assert_eq!(initials_of("my.project"), "MP");
        assert_eq!(initials_of("Public API"), "PA");
        // Three words still yields two — the mark is two columns.
        assert_eq!(initials_of("a-b-c"), "AB");
        // Repeated separators are one boundary, leading/trailing ones vanish.
        assert_eq!(initials_of("--api--gateway--"), "AG");
    }

    #[test]
    fn a_single_word_gives_its_first_two_letters_unless_it_is_camel_case() {
        assert_eq!(initials_of("workspacer"), "WO");
        assert_eq!(initials_of("myProject"), "MP");
        // Only a LOWERCASE lead counts as camelCase; PascalCase is one word.
        assert_eq!(initials_of("MyProject"), "MY");
        assert_eq!(initials_of("HTTPServer"), "HT");
        // A single character is a one-column mark, not a padded lie.
        assert_eq!(initials_of("w"), "W");
        // Digits after the lead are not a second word.
        assert_eq!(initials_of("s3bucket"), "S3");
    }

    #[test]
    fn an_empty_name_is_a_question_mark_and_an_empty_dir_is_nothing_at_all() {
        assert_eq!(initials_of(""), "?");
        assert_eq!(initials_of("---"), "?");
        assert_eq!(initials_of("   "), "?");
        // A session with no cwd has no project to name.
        assert!(resolve_project("", &Projects::new()).is_none());
    }

    /// The mark is a fixed two columns plus a space, so names stay aligned down
    /// the list whether a project's initials are one character or two.
    #[test]
    fn the_mark_is_always_three_columns() {
        assert_eq!(resolve("/w/api-gateway").mark(), "AG ");
        assert_eq!(resolve("/w/a").mark(), "A  ");
        assert_eq!(resolve("/w/api-gateway").mark().chars().count(), 3);
        assert_eq!(resolve("/w/a").mark().chars().count(), 3);
    }

    #[test]
    fn keys_normalize_separators_and_trailing_slashes() {
        assert_eq!(project_key("C:\\Users\\me\\repo"), "C:/Users/me/repo");
        assert_eq!(project_key("/w/repo/"), "/w/repo");
        assert_eq!(project_key("/w/repo///"), "/w/repo");
        assert_eq!(project_key("/w/repo"), "/w/repo");
        // A trailing separator must not change the identity — same colour.
        assert_eq!(resolve("/w/api-gateway/").color, "#fb923c");
        assert_eq!(basename_of("/w/api-gateway/"), "api-gateway");
        assert_eq!(basename_of("repo"), "repo");
        assert_eq!(basename_of(""), "");
    }

    /// A cwd that differs from the configured key only by case is the same
    /// directory on Windows and macOS, and a different one on Linux.
    #[test]
    fn a_case_only_variant_is_adopted_only_where_the_filesystem_is() {
        let map = configured("C:/Users/me/repo", Some("Repo"), None);
        for os in ["windows", "macos"] {
            assert_eq!(
                resolve_project_key_for(&map, "c:/users/me/repo", os),
                "C:/Users/me/repo",
                "{os} paths are case-insensitive"
            );
        }
        assert_eq!(
            resolve_project_key_for(&map, "c:/users/me/repo", "linux"),
            "c:/users/me/repo",
            "on Linux those are two directories"
        );
        // An exact hit always wins, on every platform.
        for os in ["windows", "macos", "linux"] {
            assert_eq!(
                resolve_project_key_for(&map, "C:/Users/me/repo", os),
                "C:/Users/me/repo"
            );
        }
    }

    #[test]
    fn from_config_reads_the_projects_map_and_tolerates_everything_else() {
        let doc = serde_json::json!({
            "ui": { "mode": "fleet" },
            "projects": {
                "/w/api-gateway": { "label": "Gateway", "color": "#112233" },
                // Unknown keys are the desktop's business, not ours.
                "/w/other": { "favourite": true, "lastOpened": 1, "iconFile": "x.png" },
                // One malformed entry must not cost the others their identity.
                "/w/broken": { "label": 42 }
            }
        });
        let projects = from_config(&doc);
        assert_eq!(projects.len(), 2, "the malformed entry is skipped");
        assert_eq!(
            resolve_project("/w/api-gateway", &projects).unwrap().color,
            "#112233"
        );
        // …and the skipped one still resolves, derived.
        assert_eq!(resolve_project("/w/broken", &projects).unwrap().initials, "BR");

        // No projects key at all is the normal, unconfigured case.
        assert!(from_config(&serde_json::json!({ "ui": {} })).is_empty());
        assert!(from_config(&serde_json::json!({ "projects": [] })).is_empty());
        assert!(from_config(&serde_json::Value::Null).is_empty());
    }
}
