//! Projects as the connected hub knows them.
//!
//! The canonical registry is the hub's shared `config.yaml` `projects` map,
//! the same one the desktop sidebar, Spawn dialog and Settings → Projects use
//! (`apps/desktop/src/renderer/src/lib/projectRegistry.ts`). It is keyed by a
//! normalized directory and carries identity (`label`, `color`, `icon`) plus
//! `favourite` (pinned) and `lastOpened` (epoch ms). Older configs still list
//! pins and recency in `directories.favourites` / `directories.recent`; those
//! are read, never written, exactly as the desktop does.
//!
//! Two other sources are folded in without becoming stores of their own:
//! directories the live fleet is working in, and this device's older
//! hub-scoped bookmarks (`native-settings.json`), which predate shared
//! projects and stay visible so nothing a user saved disappears.
//!
//! `projects` is replaced WHOLESALE by `config.save` (see
//! `services/hub-rs/src/services/config.rs` merge_patch), so every write is
//! built from a fresh `config.get` and checked against what the hub returns:
//! a save that could not take the config lock returns the old config rather
//! than an error, and must not be reported as saved. Within this process the
//! fresh-read → save → readback round is serialized per hub (see
//! `Backend::project_write`); another process writing the same config between
//! our read and save is the shared, pre-existing config read/save race.
use anyhow::{Result, bail, ensure};
use serde_json::{Map, Value, json};

use crate::{launch::absolute_directory, model::Session};

/// The key a directory has in `config.projects`: forward slashes, no trailing
/// separator. Mirrors the desktop's `projectKey`, except that a filesystem
/// root stays a root instead of collapsing to an empty key.
pub fn project_key(cwd: &str) -> String {
    let key = cwd.trim().replace('\\', "/");
    let trimmed = key.trim_end_matches('/');
    if trimmed.is_empty() {
        return if key.is_empty() { key } else { "/".into() };
    }
    // `C:/` is a root; `C:` alone is a drive-relative path, not the same place.
    if trimmed.len() == 2 && trimmed.ends_with(':') && key.len() > 2 {
        return format!("{trimmed}/");
    }
    trimmed.to_owned()
}

/// Drive-letter and UNC paths name a case-insensitive filesystem. POSIX-shaped
/// paths may be case-sensitive (Linux) and are only ever matched exactly.
fn windows_shaped(path: &str) -> bool {
    let bytes = path.as_bytes();
    path.starts_with("//")
        || (bytes.len() >= 2 && bytes[0].is_ascii_alphabetic() && bytes[1] == b':')
}

/// Whether two directory spellings name the same project.
pub fn same_dir(a: &str, b: &str) -> bool {
    let (a, b) = (project_key(a), project_key(b));
    a == b || (windows_shaped(&a) && a.eq_ignore_ascii_case(&b))
}

/// Prefer the canonical key, otherwise an equivalent imported spelling.
/// This never renames keys or merges potentially conflicting metadata.
pub fn resolve_key(map: &Map<String, Value>, cwd: &str) -> String {
    let key = project_key(cwd);
    if map.contains_key(&key) {
        return key;
    }
    map.keys()
        .find(|existing| same_dir(existing, &key))
        .cloned()
        .unwrap_or(key)
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Source {
    /// An entry (or legacy pin/recent) in the hub's shared registry.
    Hub,
    /// Only in this device's older native bookmarks for this hub.
    Device,
    /// Only known because sessions on the hub run there.
    Sessions,
}

#[derive(Clone, Debug, PartialEq)]
pub struct KnownProject {
    pub path: String,
    pub label: Option<String>,
    /// `#rrggbb` from the registry, when the user picked one.
    pub color: Option<u32>,
    /// An emoji or one or two letters from the registry.
    pub icon: Option<String>,
    pub favourite: bool,
    /// Epoch ms; legacy `recent` order maps onto negative ranks below it.
    pub last_opened: Option<i64>,
    pub sessions: usize,
    pub live_sessions: usize,
    pub source: Source,
    /// Carries configuration beyond pin/recency (label, scripts, workflow…),
    /// so this client never deletes it.
    pub configured: bool,
}

impl KnownProject {
    pub fn title(&self) -> &str {
        self.label
            .as_deref()
            .filter(|l| !l.trim().is_empty())
            .unwrap_or_else(|| basename(&self.path))
    }

    /// The mark drawn for the project: the configured icon, else initials.
    pub fn mark(&self) -> String {
        if let Some(icon) = self
            .icon
            .as_deref()
            .map(str::trim)
            .filter(|i| !i.is_empty())
        {
            return icon.chars().take(2).collect();
        }
        initials(self.title())
    }

    /// Only an entry that holds nothing but a pin/recency stamp, or a device
    /// bookmark, may be forgotten here; anything else belongs to Settings.
    pub fn removable(&self) -> bool {
        self.source == Source::Device || (self.source == Source::Hub && !self.configured)
    }

    pub fn matches(&self, query: &str) -> bool {
        let haystack = format!("{} {}", self.title(), self.path).to_lowercase();
        query
            .split_whitespace()
            .all(|term| haystack.contains(&term.to_lowercase()))
    }
}

pub fn basename(path: &str) -> &str {
    path.rsplit(['/', '\\'])
        .find(|s| !s.is_empty())
        .unwrap_or(path)
}

/// Up to two letters: the first of the first two words, or the first two
/// characters of a single word. `workspacer` → `WO`, `my-app` → `MA`.
pub fn initials(title: &str) -> String {
    let words: Vec<&str> = title
        .split(|c: char| !c.is_alphanumeric())
        .filter(|w| !w.is_empty())
        .collect();
    let letters: String = match words.as_slice() {
        [] => "?".into(),
        [one] => one.chars().take(2).collect(),
        [a, b, ..] => a.chars().take(1).chain(b.chars().take(1)).collect(),
    };
    letters.to_uppercase()
}

/// A stable hue in degrees for a path, so an uncoloured project keeps the
/// same tint everywhere it is drawn.
pub fn hue(path: &str) -> f32 {
    let hash = project_key(path)
        .bytes()
        .fold(2166136261u32, |h, b| (h ^ b as u32).wrapping_mul(16777619));
    (hash % 360) as f32
}

fn parse_color(value: &Value) -> Option<u32> {
    let hex = value.as_str()?.trim().strip_prefix('#')?;
    match hex.len() {
        6 => u32::from_str_radix(hex, 16).ok(),
        3 => {
            let v = u32::from_str_radix(hex, 16).ok()?;
            let (r, g, b) = ((v >> 8) & 0xf, (v >> 4) & 0xf, v & 0xf);
            Some((r * 17) << 16 | (g * 17) << 8 | (b * 17))
        }
        _ => None,
    }
}

/// Fields whose presence means the entry is only a pin/recency record.
const IDENTITY_ONLY: [&str; 2] = ["favourite", "lastOpened"];

/// The slice of `config.get` the client keeps: the registry, its legacy
/// arrays, and which directories have scripts/widgets configured. Plugin
/// settings and every other section stay on the hub.
pub fn registry(config: &Value) -> Value {
    let configured: Vec<Value> = ["scripts", "widgets"]
        .iter()
        .filter_map(|k| config[*k].as_object())
        .flat_map(|m| m.keys().map(|k| json!(project_key(k))))
        .collect();
    json!({
        "projects": config["projects"].as_object().cloned().unwrap_or_default(),
        "favourites": config["directories"]["favourites"].as_array().cloned().unwrap_or_default(),
        "recent": config["directories"]["recent"].as_array().cloned().unwrap_or_default(),
        "configured": configured,
    })
}

fn upsert(rows: &mut Vec<KnownProject>, path: &str, source: Source) -> usize {
    if let Some(ix) = rows.iter().position(|row| same_dir(&row.path, path)) {
        return ix;
    }
    rows.push(KnownProject {
        path: project_key(path),
        label: None,
        color: None,
        icon: None,
        favourite: false,
        last_opened: None,
        sessions: 0,
        live_sessions: 0,
        source,
        configured: false,
    });
    rows.len() - 1
}

fn strings(value: &Value) -> impl Iterator<Item = &str> {
    value
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(Value::as_str)
        .filter(|s| !s.trim().is_empty())
}

/// Every project this client can offer, pinned first, then most recently
/// opened, then directories the fleet is busy in.
pub fn list(
    registry: Option<&Value>,
    sessions: &[Session],
    device: &[String],
) -> Vec<KnownProject> {
    let mut rows: Vec<KnownProject> = Vec::new();
    if let Some(registry) = registry {
        let projects = registry["projects"]
            .as_object()
            .cloned()
            .unwrap_or_default();
        let legacy_favourites: Vec<String> =
            strings(&registry["favourites"]).map(project_key).collect();
        let configured: Vec<String> = strings(&registry["configured"]).map(project_key).collect();
        for (key, entry) in &projects {
            if project_key(key).is_empty() {
                continue;
            }
            let ix = upsert(&mut rows, key, Source::Hub);
            let row = &mut rows[ix];
            row.label = row.label.clone().or_else(|| {
                entry["label"]
                    .as_str()
                    .map(str::trim)
                    .filter(|l| !l.is_empty())
                    .map(Into::into)
            });
            row.color = row.color.or_else(|| parse_color(&entry["color"]));
            row.icon = row
                .icon
                .clone()
                .or_else(|| entry["icon"].as_str().map(Into::into));
            row.favourite |= entry["favourite"]
                .as_bool()
                .unwrap_or_else(|| legacy_favourites.iter().any(|f| same_dir(f, key)));
            row.last_opened = row
                .last_opened
                .max(entry["lastOpened"].as_f64().map(|ms| ms as i64));
            row.configured |= entry
                .as_object()
                .is_none_or(|m| m.keys().any(|k| !IDENTITY_ONLY.contains(&k.as_str())))
                || configured.iter().any(|c| same_dir(c, key));
        }
        for path in &legacy_favourites {
            let ix = upsert(&mut rows, path, Source::Hub);
            if !projects.keys().any(|k| same_dir(k, path)) {
                rows[ix].favourite = true;
            }
        }
        for (rank, path) in strings(&registry["recent"]).enumerate() {
            let ix = upsert(&mut rows, path, Source::Hub);
            if rows[ix].last_opened.is_none() {
                rows[ix].last_opened = Some(-(rank as i64) - 1);
            }
        }
        for path in &configured {
            let ix = upsert(&mut rows, path, Source::Hub);
            rows[ix].configured = true;
        }
    }
    for path in device.iter().filter(|p| !p.trim().is_empty()) {
        upsert(&mut rows, path, Source::Device);
    }
    for session in sessions.iter().filter(|s| !s.cwd.is_empty()) {
        let ix = upsert(&mut rows, &session.cwd, Source::Sessions);
        rows[ix].sessions += 1;
        if !session.stopped() {
            rows[ix].live_sessions += 1;
        }
    }
    rows.sort_by(|a, b| {
        b.favourite
            .cmp(&a.favourite)
            .then(
                b.last_opened
                    .unwrap_or(i64::MIN)
                    .cmp(&a.last_opened.unwrap_or(i64::MIN)),
            )
            .then(b.live_sessions.cmp(&a.live_sessions))
            .then(b.sessions.cmp(&a.sessions))
            .then_with(|| a.title().to_lowercase().cmp(&b.title().to_lowercase()))
            .then_with(|| a.path.cmp(&b.path))
    });
    rows
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Patch {
    /// Pin (`true`) or unpin. Writes an explicit boolean so a legacy pin can
    /// be undone without rewriting the legacy array.
    Pin(bool),
    /// Record that the project was just used, at this epoch ms.
    Touch(i64),
    /// Forget a pin/recency-only entry. Refused for configured projects.
    Remove,
}

/// The `config.save` patch for one project change, built against the hub's
/// current config. Returns the WHOLE `projects` map because the hub replaces
/// it wholesale.
pub fn patch(config: &Value, dir: &str, change: &Patch) -> Result<Value> {
    ensure!(
        absolute_directory(dir),
        "Enter an absolute project directory on the hub's machine"
    );
    let mut projects = config["projects"].as_object().cloned().unwrap_or_default();
    let key = resolve_key(&projects, dir);
    let mut partial = json!({});
    match change {
        Patch::Pin(_) | Patch::Touch(_) => {
            // Keep every imported alias and its metadata; update only the
            // identity field on all aliases so list/readback cannot disagree.
            let mut keys: Vec<String> = projects
                .keys()
                .filter(|k| same_dir(k, dir))
                .cloned()
                .collect();
            if keys.is_empty() {
                keys.push(key.clone());
            }
            let latest = keys
                .iter()
                .filter_map(|k| projects.get(k).and_then(|e| e["lastOpened"].as_i64()))
                .max();
            for key in keys {
                let entry = projects.entry(key).or_insert_with(|| json!({}));
                ensure!(
                    entry.is_object(),
                    "The hub's entry for this project is not editable"
                );
                match change {
                    Patch::Pin(pinned) => entry["favourite"] = json!(pinned),
                    Patch::Touch(now) => {
                        entry["lastOpened"] = json!(latest.unwrap_or(*now).max(*now))
                    }
                    Patch::Remove => unreachable!(),
                }
            }
        }
        Patch::Remove => {
            let registry = registry(config);
            let configured = strings(&registry["configured"]).any(|c| same_dir(c, dir));
            let protected = projects
                .iter()
                .filter(|(k, _)| same_dir(k, dir))
                .any(|(_, entry)| {
                    entry
                        .as_object()
                        .is_none_or(|m| m.keys().any(|k| !IDENTITY_ONLY.contains(&k.as_str())))
                });
            if configured || protected {
                bail!(
                    "This project has settings on the hub; manage it in Workspacer Settings → Projects"
                );
            }
            projects.retain(|k, _| !same_dir(k, dir));
            // A forgotten directory would otherwise reappear from the legacy
            // arrays, which are the one thing a removal must still rewrite.
            for list in ["recent", "favourites"] {
                if let Some(paths) = config["directories"][list].as_array() {
                    let kept: Vec<Value> = paths
                        .iter()
                        .filter(|p| p.as_str().is_none_or(|p| !same_dir(p, &key)))
                        .cloned()
                        .collect();
                    if kept.len() != paths.len() {
                        partial["directories"][list] = Value::Array(kept);
                    }
                }
            }
        }
    }
    partial["projects"] = Value::Object(projects);
    Ok(partial)
}

/// Whether the config the hub returned actually holds the change. The hub
/// answers a save it skipped (lock held, write failed) with the unchanged
/// config, so success is read back rather than assumed.
pub fn verify(saved: &Value, dir: &str, change: &Patch) -> Result<()> {
    let projects = saved["projects"].as_object().cloned().unwrap_or_default();
    let entries: Vec<&Value> = projects
        .iter()
        .filter(|(k, _)| same_dir(k, dir))
        .map(|(_, v)| v)
        .collect();
    let held = match change {
        Patch::Pin(pinned) => {
            !entries.is_empty()
                && entries
                    .iter()
                    .all(|e| e["favourite"].as_bool() == Some(*pinned))
        }
        Patch::Touch(now) => {
            !entries.is_empty()
                && entries
                    .iter()
                    .all(|e| e["lastOpened"].as_f64().is_some_and(|at| at >= *now as f64))
        }
        // Every normalized map alias and legacy trace must be absent.
        Patch::Remove => {
            entries.is_empty()
                && ["favourites", "recent"]
                    .iter()
                    .all(|list| strings(&saved["directories"][*list]).all(|p| !same_dir(p, dir)))
        }
    };
    ensure!(
        held,
        "The hub did not save this change. Its config may be locked by another writer; try again."
    );
    Ok(())
}

/// What the hub reported about a directory before launch.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Inspection {
    Missing(String),
    Repository {
        branch: Option<String>,
        changes: usize,
    },
    NotRepository,
    /// The folder exists but git could not be read (git missing, permissions).
    GitUnknown(String),
}

pub fn parse_inspection(value: &Value) -> Option<Inspection> {
    if value["exists"] == false {
        return Some(Inspection::Missing(
            value["error"]
                .as_str()
                .unwrap_or("Folder not found")
                .to_owned(),
        ));
    }
    if value["exists"] != true {
        return None;
    }
    if let Some(git) = value.get("git").filter(|g| g.is_object()) {
        return Some(Inspection::Repository {
            branch: git["branch"].as_str().map(Into::into),
            changes: git["changes"].as_u64().unwrap_or(0) as usize,
        });
    }
    let error = value["gitError"].as_str().unwrap_or_default();
    Some(if error.contains("not inside a git work tree") {
        Inspection::NotRepository
    } else {
        Inspection::GitUnknown(error.to_owned())
    })
}

/// One directory listing from the hub, for browsing a remote filesystem.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Listing {
    pub path: String,
    pub parent: String,
    pub home: String,
    pub dirs: Vec<String>,
}

pub fn parse_listing(value: &Value) -> Option<Listing> {
    let text = |k: &str| value[k].as_str().unwrap_or_default().to_owned();
    let path = text("path");
    (!path.is_empty()).then(|| Listing {
        parent: text("parent"),
        home: text("home"),
        dirs: strings(&value["dirs"]).take(2000).map(Into::into).collect(),
        path,
    })
}

/// A child directory of a listing, joined with the hub's own separator.
pub fn child(parent: &str, name: &str) -> String {
    let sep = if parent.contains('\\') && !parent.contains('/') {
        '\\'
    } else {
        '/'
    };
    if parent.ends_with(['/', '\\']) {
        format!("{parent}{name}")
    } else {
        format!("{parent}{sep}{name}")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn session(id: &str, cwd: &str, stopped: bool) -> Session {
        let mut s = Session {
            id: id.into(),
            cwd: cwd.into(),
            ..Default::default()
        };
        if stopped {
            s.state = "stopped".into();
        }
        s
    }

    #[test]
    fn keys_match_the_desktop_registry_and_keep_roots() {
        assert_eq!(project_key("/work/app/"), "/work/app");
        assert_eq!(project_key("C:\\Work\\App\\"), "C:/Work/App");
        assert_eq!(project_key("/"), "/");
        assert_eq!(project_key("C:\\"), "C:/");
        assert_eq!(project_key(""), "");
        assert!(same_dir("C:\\Work\\App", "c:/work/app"));
        assert!(
            !same_dir("/work/App", "/work/app"),
            "POSIX case is significant"
        );
        let map: Map<String, Value> =
            serde_json::from_value(json!({"C:/Work/App": {"label":"App"}})).unwrap();
        assert_eq!(resolve_key(&map, "c:\\work\\app"), "C:/Work/App");
        assert_eq!(resolve_key(&map, "/work/app"), "/work/app");
    }

    #[test]
    fn list_unions_registry_legacy_device_and_fleet_in_a_useful_order() {
        let config = json!({
            "projects": {
                "/work/api": {"lastOpened": 2000, "label": "API", "color": "#336699"},
                "/work/web": {"lastOpened": 3000},
                "/work/old": {"favourite": false},
                "/work/pinned": {"favourite": true, "lastOpened": 10}
            },
            "directories": {"favourites": ["/work/old", "/work/legacy-pin/"], "recent": ["/work/recent-a", "/work/recent-b"]},
            "scripts": {"/work/web": {"build": "make"}},
            "plugins": {"secret-looking": "stays on the hub"}
        });
        let registry = registry(&config);
        assert!(registry.get("plugins").is_none());
        let rows = list(
            Some(&registry),
            &[
                session("a", "/work/web", false),
                session("b", "/fleet/only", false),
                session("c", "/work/web", true),
            ],
            &["/device/bookmark".into(), "/work/api".into()],
        );
        let order: Vec<&str> = rows.iter().map(|r| r.path.as_str()).collect();
        assert_eq!(
            order,
            [
                "/work/pinned",
                "/work/legacy-pin",
                "/work/web",
                "/work/api",
                "/work/recent-a",
                "/work/recent-b",
                "/fleet/only",
                // Idle and never opened: alphabetical by title.
                "/device/bookmark",
                "/work/old",
            ]
        );
        let web = &rows[2];
        assert_eq!((web.sessions, web.live_sessions), (2, 1));
        assert!(web.configured, "scripts make a project configured");
        assert!(!web.removable());
        let api = &rows[3];
        assert_eq!(api.title(), "API");
        assert_eq!(api.color, Some(0x336699));
        assert_eq!(
            api.source,
            Source::Hub,
            "device duplicate folds into the hub entry"
        );
        assert!(api.configured, "a label is configuration");
        assert!(!rows[8].favourite, "explicit false shadows a legacy pin");
        assert!(rows[8].removable());
        assert_eq!(rows[7].source, Source::Device);
        assert_eq!(rows[6].source, Source::Sessions);
        assert!(!rows[6].removable());
        assert!(
            rows.iter()
                .find(|r| r.path == "/work/api")
                .unwrap()
                .matches("api work")
        );
        assert!(!rows[0].matches("api"));
    }

    #[test]
    fn list_without_a_registry_still_offers_device_and_fleet_projects() {
        let rows = list(
            None,
            &[session("a", "/fleet/x", false)],
            &["/device/y".into()],
        );
        assert_eq!(rows.len(), 2);
        assert_eq!(
            rows[0].path, "/fleet/x",
            "live work outranks an idle bookmark"
        );
    }

    #[test]
    fn patches_send_the_whole_map_and_preserve_other_projects() {
        let config = json!({"projects": {
            "/keep": {"label": "Keep", "workflowId": "wf"},
            "C:/Work/App": {"lastOpened": 1}
        }});
        let pin = patch(&config, "c:\\work\\app", &Patch::Pin(true)).unwrap();
        assert_eq!(pin["projects"]["/keep"]["workflowId"], "wf");
        assert_eq!(
            pin["projects"]["C:/Work/App"],
            json!({"lastOpened": 1, "favourite": true})
        );
        assert!(pin.get("directories").is_none());
        let touch = patch(&config, "/new/project/", &Patch::Touch(42)).unwrap();
        assert_eq!(touch["projects"]["/new/project"]["lastOpened"], 42);
        assert!(patch(&config, "relative", &Patch::Pin(true)).is_err());
        assert!(
            patch(&config, "/keep", &Patch::Remove).is_err(),
            "configured entries stay"
        );
    }

    #[test]
    fn remove_forgets_identity_only_entries_and_their_legacy_traces() {
        let config = json!({
            "projects": {"/gone": {"favourite": true, "lastOpened": 5}, "/other": {}},
            "directories": {"recent": ["/gone/", "/other"], "favourites": ["/x"]},
            "widgets": {"/boarded": {}}
        });
        let removed = patch(&config, "/gone", &Patch::Remove).unwrap();
        assert!(removed["projects"].get("/gone").is_none());
        assert!(removed["projects"].get("/other").is_some());
        assert_eq!(removed["directories"]["recent"], json!(["/other"]));
        assert!(
            removed["directories"].get("favourites").is_none(),
            "untouched arrays are not rewritten"
        );
        assert!(patch(&config, "/boarded", &Patch::Remove).is_err());
    }

    #[test]
    fn verify_refuses_a_save_the_hub_silently_skipped() {
        let before = json!({"projects": {}});
        let change = Patch::Pin(true);
        assert!(verify(&before, "/a", &change).is_err());
        let after = json!({"projects": {"/a": {"favourite": true}}});
        assert!(verify(&after, "/a/", &change).is_ok());
        assert!(verify(&after, "/a", &Patch::Remove).is_err());
        assert!(verify(&before, "/a", &Patch::Remove).is_ok());
        // A legacy trace (any spelling) still lists the project: not removed.
        for list in ["favourites", "recent"] {
            let legacy = json!({"projects": {}, "directories": {list: ["/a/"]}});
            assert!(verify(&legacy, "/a", &Patch::Remove).is_err(), "{list}");
        }
        assert!(
            verify(
                &json!({"projects":{"/a":{"lastOpened":7.0}}}),
                "/a",
                &Patch::Touch(7)
            )
            .is_ok()
        );
    }

    /// The hub answers a save it skipped with the unchanged config. A
    /// legacy-only project has no map entry even then, so absence from the
    /// map alone is no proof; the project must be gone from what it lists.
    #[test]
    fn legacy_only_removal_is_verified_against_every_legacy_trace() {
        let config = json!({
            "projects": {"/keep": {"label": "Keep", "favourite": true}},
            "directories": {"favourites": ["/legacy", "/keep"], "recent": ["/legacy/", "/other"]},
            "plugins": {"x": 1}
        });
        let listed = |c: &Value| {
            list(Some(&registry(c)), &[], &[])
                .iter()
                .any(|p| p.path == "/legacy")
        };
        assert!(listed(&config));
        let partial = patch(&config, "/legacy", &Patch::Remove).unwrap();
        assert_eq!(partial["directories"]["favourites"], json!(["/keep"]));
        assert_eq!(partial["directories"]["recent"], json!(["/other"]));
        assert_eq!(partial["projects"]["/keep"], config["projects"]["/keep"]);

        // Skipped save: the hub returns the config it already had.
        let refused = verify(&config, "/legacy", &Patch::Remove).unwrap_err();
        assert!(refused.to_string().contains("did not save"), "{refused}");

        // Applied save, as the hub's merge_patch would leave it.
        let mut saved = config.clone();
        saved["projects"] = partial["projects"].clone();
        saved["directories"] = partial["directories"].clone();
        assert!(verify(&saved, "/legacy", &Patch::Remove).is_ok());
        assert!(!listed(&saved));
        assert_eq!(saved["projects"]["/keep"]["label"], "Keep");
        assert_eq!(saved["plugins"], config["plugins"]);

        // A half-applied save (map written, a legacy array not) is refused too.
        let mut partial_save = saved.clone();
        partial_save["directories"]["recent"] = json!(["/legacy"]);
        assert!(verify(&partial_save, "/legacy", &Patch::Remove).is_err());

        // Configured metadata is never forgotten from here, legacy or not.
        let protected = json!({
            "projects": {},
            "directories": {"favourites": ["/scripted"]},
            "scripts": {"/scripted": {"build": "make"}}
        });
        assert!(patch(&protected, "/scripted", &Patch::Remove).is_err());
    }

    #[test]
    fn imported_aliases_remove_every_trace_and_verify_skipped_saves() {
        for (path, aliases) in [
            ("/a", vec!["/a", "/a/", "/a//"]),
            (
                "c:/work/app",
                vec!["C:\\Work\\App\\", "c:/work/app/", "C:/WORK/App"],
            ),
            (
                "//host/share/app",
                vec!["\\\\HOST\\share\\app\\", "//host/share/app/"],
            ),
        ] {
            let mut config = json!({"projects":{"/keep":{"label":"Keep"}},
                "directories":{"recent":aliases,"favourites":aliases}});
            for alias in &aliases {
                config["projects"][*alias] = json!({"lastOpened":5});
            }
            assert!(verify(&config, path, &Patch::Remove).is_err());
            let removed = patch(&config, path, &Patch::Remove).unwrap();
            assert_eq!(removed["projects"], json!({"/keep":{"label":"Keep"}}));
            assert!(verify(&removed, path, &Patch::Remove).is_ok());
            let mut half = removed.clone();
            half["projects"][aliases[0]] = json!({});
            assert!(verify(&half, path, &Patch::Remove).is_err());
        }
    }

    #[test]
    fn alias_identity_updates_preserve_conflicting_metadata_and_protection() {
        let config = json!({"projects": {
            "/a":{"label":"Canonical","lastOpened":100},
            "/a/":{"label":"Imported","workflowId":"wf","lastOpened":200},
            "/a//":{"favourite":true}
        }});
        let rows = list(Some(&registry(&config)), &[], &[]);
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].title(), "Canonical");
        assert!(rows[0].configured && !rows[0].removable());
        assert_eq!(rows[0].last_opened, Some(200));
        assert!(patch(&config, "/a", &Patch::Remove).is_err());
        let unpin = patch(&config, "/a", &Patch::Pin(false)).unwrap();
        assert!(verify(&unpin, "/a", &Patch::Pin(false)).is_ok());
        assert!(!list(Some(&registry(&unpin)), &[], &[])[0].favourite);
        let touch = patch(&unpin, "/a", &Patch::Touch(50)).unwrap();
        assert!(verify(&touch, "/a", &Patch::Touch(50)).is_ok());
        for alias in ["/a", "/a/", "/a//"] {
            assert_eq!(touch["projects"][alias]["lastOpened"], 200);
        }
        assert_eq!(touch["projects"]["/a"]["label"], "Canonical");
        assert_eq!(touch["projects"]["/a/"]["label"], "Imported");
        assert_eq!(touch["projects"]["/a/"]["workflowId"], "wf");
        for field in ["scripts", "widgets"] {
            let protected = json!({"projects":{"/a/":{}},field:{"/a//":{}}});
            assert!(patch(&protected, "/a", &Patch::Remove).is_err());
        }
        assert!(
            patch(
                &json!({"projects":{"/a":{},"/a/":null}}),
                "/a",
                &Patch::Remove
            )
            .is_err()
        );
    }

    #[test]
    fn inspection_distinguishes_missing_plain_and_repository_folders() {
        assert_eq!(parse_inspection(&json!(null)), None);
        assert_eq!(
            parse_inspection(&json!({"exists": false, "error": "No such file"})),
            Some(Inspection::Missing("No such file".into()))
        );
        assert_eq!(
            parse_inspection(&json!({"exists": true, "git": {"branch": "main", "changes": 2}})),
            Some(Inspection::Repository {
                branch: Some("main".into()),
                changes: 2
            })
        );
        assert_eq!(
            parse_inspection(
                &json!({"exists": true, "gitError": "cwd is not inside a git work tree"})
            ),
            Some(Inspection::NotRepository)
        );
        assert!(matches!(
            parse_inspection(&json!({"exists": true, "gitError": "git: not found"})),
            Some(Inspection::GitUnknown(_))
        ));
    }

    #[test]
    fn identity_marks_and_listing_paths() {
        assert_eq!(initials("workspacer"), "WO");
        assert_eq!(initials("my-app"), "MA");
        assert_eq!(initials("…"), "?");
        assert_eq!(hue("/a/b"), hue("/a/b/"));
        let listing = parse_listing(
            &json!({"path":"/home/me","parent":"/home","home":"/home/me","dirs":["a","",7]}),
        )
        .unwrap();
        assert_eq!(listing.dirs, ["a"]);
        assert_eq!(child("/", "etc"), "/etc");
        assert_eq!(child("/home/me", "src"), "/home/me/src");
        assert_eq!(child("C:\\Users", "me"), "C:\\Users\\me");
        assert!(parse_listing(&json!({})).is_none());
    }
}
