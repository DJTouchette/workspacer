//! `sessions.kept`: the sessions the native client had open when it closed.
//!
//! Native remembers, per hub, which sessions it saw open
//! (`Settings::kept_open` in `native-settings.json`) and shows the ones its
//! closing stopped as "Paused", resumable by sending. That set lives on the
//! host, in the user's preference directory, so a phone could not see it. This
//! read-only exposure lets `/m-next` show the same Paused sessions instead of
//! keeping a list of its own.
//!
//! Every scope's set is merged: native keys them by hub identity (a bus URL or
//! a local data dir), and session ids are provider UUIDs, so an id kept for a
//! different hub simply never resolves here. The answer discloses only session
//! ids (and when native first saw each open) — the same ids agent.snapshot
//! already carries — so it sits in the view tier like `sessionArchive.get`.
//! A missing or unreadable file is an empty set, never an error.
use serde_json::{Value, json};
use std::path::{Path, PathBuf};

/// Native reads at most this much; a settings file is a few KiB.
const MAX_BYTES: u64 = 1024 * 1024;
/// Native caps the ids it reads back per fleet read at a similar size.
const MAX_SESSIONS: usize = 200;

/// Where native keeps its settings (`apps/native/src/appearance.rs`
/// `preference_path`, with `native-settings.json` as the file name).
pub fn settings_path() -> Option<PathBuf> {
    let non_empty = |name: &str| {
        std::env::var_os(name)
            .filter(|v| !v.is_empty())
            .map(PathBuf::from)
    };
    let home = || non_empty("HOME").or_else(|| non_empty("USERPROFILE"));
    let base = if cfg!(target_os = "windows") {
        non_empty("APPDATA").or_else(|| home().map(|h| h.join("AppData/Roaming")))
    } else {
        non_empty("XDG_CONFIG_HOME").or_else(|| home().map(|h| h.join(".config")))
    }?;
    Some(base.join("workspacer").join("native-settings.json"))
}

/// `{sessions: [{sessionId, since}]}`, newest first, from `path`.
pub fn read(path: Option<&Path>) -> Value {
    let Some(path) = path else {
        return json!({"sessions": []});
    };
    let text = std::fs::File::open(path).ok().and_then(|file| {
        use std::io::Read;
        let mut text = String::new();
        file.take(MAX_BYTES).read_to_string(&mut text).ok()?;
        Some(text)
    });
    let settings: Value = text
        .and_then(|text| serde_json::from_str(&text).ok())
        .unwrap_or(Value::Null);
    let mut kept: std::collections::BTreeMap<String, i64> = Default::default();
    for scope in settings["kept_open"]
        .as_object()
        .into_iter()
        .flat_map(|m| m.values())
    {
        for (id, since) in scope.as_object().into_iter().flatten() {
            if id.is_empty() || id.len() > 128 || id.contains(['/', '\\', '\0']) {
                continue;
            }
            let since = since.as_i64().unwrap_or(0);
            let entry = kept.entry(id.clone()).or_insert(since);
            *entry = (*entry).max(since);
        }
    }
    let mut rows: Vec<(String, i64)> = kept.into_iter().collect();
    rows.sort_by(|a, b| b.1.cmp(&a.1).then_with(|| a.0.cmp(&b.0)));
    rows.truncate(MAX_SESSIONS);
    json!({"sessions": rows.into_iter().map(|(id, since)| json!({"sessionId": id, "since": since})).collect::<Vec<_>>()})
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn merges_every_scope_newest_first_and_drops_unsafe_ids() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("native-settings.json");
        std::fs::write(
            &path,
            json!({"text_size": 15, "kept_open": {
                "ws://127.0.0.1:7811/bus": {"a": 100, "b": 300},
                "rust-local:/data": {"a": 200, "../x": 1, "": 5},
            }})
            .to_string(),
        )
        .unwrap();
        assert_eq!(
            read(Some(&path)),
            json!({"sessions": [{"sessionId": "b", "since": 300}, {"sessionId": "a", "since": 200}]})
        );
    }

    #[test]
    fn a_missing_or_malformed_file_is_an_empty_set() {
        let dir = tempfile::tempdir().unwrap();
        let missing = dir.path().join("none.json");
        assert_eq!(read(Some(&missing)), json!({"sessions": []}));
        std::fs::write(&missing, "{not json").unwrap();
        assert_eq!(read(Some(&missing)), json!({"sessions": []}));
        std::fs::write(&missing, r#"{"kept_open": ["wrong shape"]}"#).unwrap();
        assert_eq!(read(Some(&missing)), json!({"sessions": []}));
        assert_eq!(read(None), json!({"sessions": []}));
    }
}
