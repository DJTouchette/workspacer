//! Resume-picker metadata from the same 8KiB transcript heads as Go/desktop.
use crate::Options;
use claudemon::session::transcript::project_dir_name as directory_name;
use serde_json::{Value, json};
use std::{io::Read, path::Path};
fn summary(bytes: &[u8]) -> String {
    let mut first = String::new();
    for line in bytes.split(|byte| *byte == b'\n') {
        let Ok(value) = serde_json::from_slice::<Value>(line) else {
            continue;
        };
        if value["type"] == "summary"
            && let Some(text) = value["summary"].as_str().filter(|text| !text.is_empty())
        {
            return text.chars().take(100).collect();
        }
        if first.is_empty() && value["type"] == "user" {
            let content = &value["message"]["content"];
            let text = content.as_str().map(str::to_owned).unwrap_or_else(|| {
                content
                    .as_array()
                    .into_iter()
                    .flatten()
                    .filter(|block| block["type"] == "text")
                    .filter_map(|block| block["text"].as_str())
                    .collect::<Vec<_>>()
                    .join("\n")
            });
            first = text
                .chars()
                .take(100)
                .collect::<String>()
                .replace('\n', " ");
        }
    }
    first
}
pub fn list(home: &Path, cwd: &str) -> Value {
    let Some(name) = directory_name(cwd) else {
        return json!([]);
    };
    let Ok(entries) = std::fs::read_dir(home.join(".claude/projects").join(name)) else {
        return json!([]);
    };
    let mut candidates = Vec::new();
    for entry in entries.flatten() {
        let name = entry.file_name().to_string_lossy().into_owned();
        let Some(id) = name
            .strip_suffix(".jsonl")
            .filter(|id| !id.starts_with("agent-"))
        else {
            continue;
        };
        let Ok(metadata) = entry.metadata() else {
            continue;
        };
        if !metadata.is_file() {
            continue;
        }
        let Ok(modified) = metadata.modified() else {
            continue;
        };
        candidates.push((modified, id.to_owned(), entry.path()));
    }
    // Go sorts the millisecond wire timestamps after os.ReadDir's filename
    // ordering. Sub-millisecond differences must not reorder equal wire dates.
    let millis = |time| chrono::DateTime::<chrono::Utc>::from(time).timestamp_millis();
    candidates.sort_by(|a, b| millis(b.0).cmp(&millis(a.0)).then_with(|| a.1.cmp(&b.1)));
    candidates.truncate(20);
    let rows:Vec<_>=candidates.into_iter().map(|(modified,id,path)|{
        let read=||->std::io::Result<Vec<u8>>{
            let mut options=std::fs::OpenOptions::new();options.read(true);
            #[cfg(unix)]{use std::os::unix::fs::OpenOptionsExt;options.custom_flags(libc::O_NONBLOCK);}
            let file=options.open(path)?;if !file.metadata()?.is_file(){return Ok(Vec::new());}
            let mut bytes=Vec::new();file.take(8192).read_to_end(&mut bytes)?;Ok(bytes)
        };
        let text=summary(&read().unwrap_or_default());let timestamp:chrono::DateTime<chrono::Utc>=modified.into();
        json!({"sessionId":id,"timestamp":timestamp.to_rfc3339_opts(chrono::SecondsFormat::Millis,true),"summary":if text.is_empty(){id}else{text}})
    }).collect();
    json!(rows)
}
pub(crate) fn install(options: Options, home: std::path::PathBuf) -> Options {
    options.handler("claude.sessionsForDir", move |_, params| {
        let home = home.clone();
        async move {
            let cwd = match params.get("cwd") {
                None | Some(Value::Null) => "",
                Some(value) => value
                    .as_str()
                    .ok_or_else(|| anyhow::anyhow!("cwd must be text"))?,
            }
            .to_owned();
            tokio::task::spawn_blocking(move || Ok(list(&home, &cwd))).await?
        }
    })
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn project_dir_names_match_reference_corpus() {
        let corpus: Value = serde_json::from_str(include_str!(
            "../../../../contracts/path-containment-cases.json"
        ))
        .unwrap();
        let cases = corpus["projectDirNames"]["cases"].as_array().unwrap();
        assert!(cases.len() >= 8);
        let mut accepted = 0;
        let mut refused = 0;
        for case in cases {
            if case["expect"].is_null() {
                refused += 1;
            } else {
                accepted += 1;
            }
            assert_eq!(
                directory_name(case["cwd"].as_str().unwrap())
                    .map(Value::String)
                    .unwrap_or(Value::Null),
                case["expect"],
                "{case}"
            );
        }
        assert!(
            accepted > 0 && refused > 0,
            "projectDirNames lost a verdict class"
        );
    }
    #[test]
    fn summary_names_win_and_unicode_clipping_preserves_scalar_boundaries() {
        let first=json!({"type":"user","message":{"content":[{"type":"text","text":"🦀".repeat(99)},{"type":"image"},{"type":"text","text":"end"}]}}).to_string();
        let result = summary(first.as_bytes());
        assert_eq!(result.chars().count(), 100);
        assert!(result.ends_with(' '));
        assert_eq!(
            summary(
                format!("{first}\n{{\"type\":\"summary\",\"summary\":\"Actual title\"}}\n{{broken")
                    .as_bytes()
            ),
            "Actual title"
        );
    }
    #[test]
    fn legacy_clip_vectors_count_scalars_for_both_user_text_and_summary() {
        for (text, expected) in [
            (
                format!("{}é", "a".repeat(99)),
                format!("{}é", "a".repeat(99)),
            ),
            (
                format!("a{}", "😀".repeat(150)),
                format!("a{}", "😀".repeat(99)),
            ),
            ("😀".repeat(150), "😀".repeat(100)),
        ] {
            for entry in [
                json!({"type":"summary","summary":text}),
                json!({"type":"user","message":{"content":text}}),
            ] {
                let actual = summary(entry.to_string().as_bytes());
                assert_eq!(actual, expected);
                assert_eq!(actual.chars().count(), 100);
            }
        }
    }
    #[test]
    fn equal_wire_timestamps_keep_filename_order_and_head_reads_are_bounded() {
        let home = tempfile::tempdir().unwrap();
        let dir = home.path().join(".claude/projects/-repo");
        std::fs::create_dir_all(&dir).unwrap();
        for (name, nanos, body) in [
            (
                "a",
                1000,
                json!({"type":"user","message":{"content":"first\nline"}}).to_string(),
            ),
            (
                "z",
                9000,
                format!(
                    "{}\n{}",
                    " ".repeat(8192),
                    json!({"type":"summary","summary":"past peek budget"})
                ),
            ),
        ] {
            let path = dir.join(format!("{name}.jsonl"));
            std::fs::write(&path, body).unwrap();
            std::fs::File::options()
                .write(true)
                .open(path)
                .unwrap()
                .set_times(
                    std::fs::FileTimes::new()
                        .set_modified(std::time::UNIX_EPOCH + std::time::Duration::new(100, nanos)),
                )
                .unwrap();
        }
        std::fs::create_dir(dir.join("directory.jsonl")).unwrap();
        std::fs::write(dir.join("agent-hidden.jsonl"), "{}").unwrap();
        std::fs::write(dir.join("ignored.txt"), "{}").unwrap();
        let rows = list(home.path(), "/repo");
        assert_eq!(rows.as_array().unwrap().len(), 2);
        assert_eq!(
            rows[0],
            json!({"sessionId":"a","timestamp":"1970-01-01T00:01:40.000Z","summary":"first line"})
        );
        assert_eq!(rows[1]["sessionId"], "z");
        assert_eq!(rows[1]["summary"], "z");
        assert_eq!(rows[1]["timestamp"], rows[0]["timestamp"]);
    }
    #[test]
    fn real_picker_reads_only_selected_directory_and_newest_twenty() {
        let home = tempfile::tempdir().unwrap();
        let dir = home.path().join(".claude/projects/-repo");
        std::fs::create_dir_all(&dir).unwrap();
        for i in 0..25 {
            let path = dir.join(format!("session-{i:02}.jsonl"));
            std::fs::write(
                &path,
                format!("{{\"type\":\"user\",\"message\":{{\"content\":\"task {i}\"}}}}\n"),
            )
            .unwrap();
            let file = std::fs::OpenOptions::new().write(true).open(path).unwrap();
            file.set_times(
                std::fs::FileTimes::new()
                    .set_modified(std::time::UNIX_EPOCH + std::time::Duration::from_secs(i + 100)),
            )
            .unwrap();
        }
        std::fs::write(dir.join("agent-subagent.jsonl"), "{}").unwrap();
        std::fs::write(
            home.path().join(".claude/history.jsonl"),
            "{\"type\":\"summary\",\"summary\":\"outside\"}",
        )
        .unwrap();
        let rows = list(home.path(), "/repo");
        assert_eq!(rows.as_array().unwrap().len(), 20);
        assert_eq!(rows[0]["sessionId"], "session-24");
        assert_eq!(rows[19]["summary"], "task 5");
        assert_eq!(list(home.path(), ".."), json!([]));
        assert_eq!(list(home.path(), ""), json!([]));
    }
}
