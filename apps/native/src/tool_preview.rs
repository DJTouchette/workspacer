//! Pure, bounded presentation of recorded tool arguments. Never reads live files.
use serde_json::Value;

#[derive(Debug, PartialEq)]
pub struct CodePreview {
    pub label: String,
    pub language: &'static str,
    pub text: String,
}

#[derive(Debug, Default)]
pub struct ToolPreview {
    pub title: String,
    pub description: String,
    pub target: String,
    pub working_directory: String,
    pub blocks: Vec<CodePreview>,
    pub added: usize,
    pub removed: usize,
    pub file_edit: bool,
}

fn field<'a>(value: &'a Value, keys: &[&str]) -> &'a str {
    keys.iter()
        .find_map(|k| {
            value
                .get(k)
                .and_then(Value::as_str)
                .filter(|s| !s.trim().is_empty())
        })
        .unwrap_or_default()
}

/// Decode argv or an unambiguous shell launcher for display only. The retained
/// arguments (and Copy all) stay exact; nothing here is evaluated or executed.
fn command_text(input: &Value) -> String {
    let command = input.get("command").or_else(|| input.get("cmd"));
    let Some(command) = command else {
        return String::new();
    };
    let words = if let Some(args) = command.as_array() {
        args.iter()
            .map(Value::as_str)
            .collect::<Option<Vec<_>>>()
            .map(|args| args.into_iter().map(str::to_owned).collect::<Vec<_>>())
    } else {
        command.as_str().and_then(|s| shell_words::split(s).ok())
    };
    if let Some(args) = &words {
        let shell = args
            .first()
            .map(|s| s.rsplit('/').next().unwrap_or(s))
            .unwrap_or_default();
        if args.len() == 3
            && matches!(shell, "bash" | "sh" | "zsh" | "dash")
            && matches!(args[1].as_str(), "-c" | "-lc" | "-cl")
        {
            return args[2].clone();
        }
    }
    command.as_str().map(str::to_owned).unwrap_or_else(|| {
        words
            .map(|args| {
                args.iter()
                    .map(|arg| shell_words::quote(arg).into_owned())
                    .collect::<Vec<_>>()
                    .join(" ")
            })
            .unwrap_or_else(|| command.to_string())
    })
}

fn command_title(command: &str) -> String {
    let words = shell_words::split(command).unwrap_or_default();
    let executable = words
        .first()
        .map(|s| s.rsplit('/').next().unwrap_or(s))
        .unwrap_or_default();
    if executable.is_empty() {
        return "Run command".into();
    }
    let subcommand = words.get(1).filter(|s| {
        matches!(
            executable,
            "cargo" | "git" | "npm" | "pnpm" | "yarn" | "go" | "make"
        ) && s
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | ':'))
    });
    format!(
        "Run {executable}{}",
        subcommand.map(|s| format!(" {s}")).unwrap_or_default()
    )
}

fn one_line(text: &str) -> String {
    text.split_whitespace().collect::<Vec<_>>().join(" ")
}

pub fn language(path: &str) -> &'static str {
    match path
        .rsplit('.')
        .next()
        .unwrap_or_default()
        .to_ascii_lowercase()
        .as_str()
    {
        "rs" => "rust",
        "go" => "go",
        "ts" => "typescript",
        "tsx" | "jsx" => "tsx",
        "js" | "mjs" | "cjs" => "javascript",
        "py" => "python",
        "rb" => "ruby",
        "ex" | "exs" => "elixir",
        "json" | "jsonc" => "json",
        "yaml" | "yml" => "yaml",
        "toml" => "toml",
        "html" | "htm" => "html",
        "css" => "css",
        "sql" => "sql",
        "sh" | "bash" | "zsh" => "bash",
        "c" | "h" => "c",
        "cpp" | "hpp" => "cpp",
        "cs" => "csharp",
        "java" => "java",
        "swift" => "swift",
        "md" => "markdown",
        "diff" | "patch" => "diff",
        _ => "text",
    }
}

/// A fence longer than every backtick run keeps arbitrary tool output literal.
pub fn fenced(language: &str, text: &str) -> String {
    let mut run = 0;
    let mut longest = 2;
    for c in text.chars() {
        if c == '`' {
            run += 1;
            longest = longest.max(run);
        } else {
            run = 0;
        }
    }
    let fence = "`".repeat(longest + 1);
    format!("{fence}{language}\n{text}\n{fence}")
}

impl ToolPreview {
    fn block(&mut self, label: impl Into<String>, language: &'static str, text: &str) {
        if !text.is_empty() {
            self.blocks.push(CodePreview {
                label: label.into(),
                language,
                text: text.into(),
            });
        }
    }

    fn diff(&mut self, path: &str, text: &str) {
        self.added += text
            .lines()
            .filter(|l| l.starts_with('+') && !l.starts_with("+++"))
            .count();
        self.removed += text
            .lines()
            .filter(|l| l.starts_with('-') && !l.starts_with("---"))
            .count();
        self.block(if path.is_empty() { "Patch" } else { path }, "diff", text);
    }

    fn patch(&mut self, fallback_path: &str, patch: &str) {
        // Use the same file boundaries as History and the changed-files footer,
        // including unified deletions whose new path is /dev/null.
        let tool = crate::transcript::Tool {
            name: "apply_patch".into(),
            input: serde_json::json!({"file_path":fallback_path,"diff":patch}).to_string(),
            ..Default::default()
        };
        let changes = tool.changes();
        if changes.is_empty() {
            self.diff(fallback_path, patch);
            return;
        }
        for change in changes {
            if self.target.is_empty() {
                self.target = change.path.clone();
            }
            self.diff(&change.path, &change.diff);
        }
    }
}

pub fn parse(name: &str, raw: &str, output: Option<&str>) -> ToolPreview {
    let input: Value = serde_json::from_str(raw).unwrap_or(Value::String(raw.into()));
    let short = name.rsplit("__").next().unwrap_or(name);
    let short = short.rsplit('.').next().unwrap_or(short);
    let normalized = short.to_ascii_lowercase();
    let path = field(&input, &["file_path", "filePath", "path", "filename"]);
    let mut preview = ToolPreview {
        title: short.into(),
        description: field(&input, &["description", "title", "summary"])
            .trim()
            .into(),
        target: path.into(),
        working_directory: field(&input, &["workdir", "cwd", "working_directory"]).into(),
        ..Default::default()
    };
    match normalized.as_str() {
        "edit" | "multiedit" | "str_replace_editor" => {
            preview.title = "Edit file".into();
            preview.file_edit = true;
            let edits = input.get("edits").and_then(Value::as_array);
            for edit in edits
                .map(|v| v.as_slice())
                .unwrap_or(std::slice::from_ref(&input))
            {
                let old = field(edit, &["old_string", "oldString", "old_str", "oldText"]);
                let new = field(edit, &["new_string", "newString", "new_str", "newText"]);
                preview.removed += old.lines().count();
                preview.added += new.lines().count();
                preview.block("Before", language(path), old);
                preview.block("After", language(path), new);
            }
        }
        "write" | "write_file" | "create_file" => {
            preview.title = "Write file".into();
            preview.file_edit = true;
            let content = field(&input, &["content", "text", "file_text"]);
            preview.block(path, language(path), content);
        }
        "apply_patch" | "patch" => {
            preview.title = "Edit files".into();
            preview.file_edit = true;
            if let Some(changes) = input.get("changes").and_then(Value::as_array) {
                for change in changes {
                    preview.diff(
                        field(change, &["path", "file_path"]),
                        field(change, &["diff", "patch"]),
                    );
                }
            } else {
                let patch = input
                    .as_str()
                    .unwrap_or_else(|| field(&input, &["patch", "diff", "input"]));
                preview.patch(path, patch);
            }
        }
        "read" | "read_file" => {
            preview.title = "Read file".into();
            if let Some(output) = output {
                // Claude's cat -n style prefixes aren't part of the source language.
                let text = output
                    .lines()
                    .map(|l| {
                        if let Some((number, code)) = l.split_once('\t')
                            && number.trim().parse::<usize>().is_ok()
                        {
                            return code;
                        }
                        if let Some((number, code)) = l.split_once('→')
                            && number.trim().parse::<usize>().is_ok()
                        {
                            return code;
                        }
                        l
                    })
                    .collect::<Vec<_>>()
                    .join("\n");
                preview.block(path, language(path), &text);
            }
        }
        "bash" | "shell" | "exec_command" | "run_command" | "terminal" => {
            let command = command_text(&input);
            preview.title = command_title(&command);
            preview.target = command
                .lines()
                .map(str::trim)
                .collect::<Vec<_>>()
                .join(" ↵ ");
            preview.block("Command", "bash", &command);
        }
        "grep" | "search" | "recon_grep" | "recon_search" => {
            preview.title = "Search".into();
            let query = field(&input, &["pattern", "query"]);
            if !query.is_empty() {
                preview.target = if path.is_empty() {
                    one_line(query)
                } else {
                    format!("{} in {path}", one_line(query))
                };
            }
        }
        "glob" => preview.title = "Find files".into(),
        "list" | "list_directory" => preview.title = "List directory".into(),
        _ => {}
    }
    if !preview.description.is_empty() {
        preview.title = one_line(&preview.description);
    }
    if preview.target.is_empty() {
        preview.target = one_line(field(
            &input,
            &["query", "pattern", "url", "symbol", "name", "prompt"],
        ));
    }
    if preview.target.is_empty() {
        if let Some(args) = input.get("args").and_then(Value::as_array) {
            preview.target = args
                .iter()
                .filter_map(Value::as_str)
                .collect::<Vec<_>>()
                .join(" ");
        } else {
            preview.target = one_line(field(&input, &["code", "input", "task", "message"]));
        }
    }
    if preview.blocks.is_empty() {
        let pretty = serde_json::to_string_pretty(&input).unwrap_or_else(|_| raw.into());
        preview.block("Arguments", "json", &pretty);
    }
    if !matches!(normalized.as_str(), "read" | "read_file")
        && let Some(output) = output.filter(|o| !o.is_empty())
    {
        let lang = if serde_json::from_str::<Value>(output).is_ok() {
            "json"
        } else {
            "text"
        };
        preview.block("Output", lang, output);
    }
    preview
}

/// A concise description derived only from recorded arguments.
pub fn overview(name: &str, input: &str) -> String {
    let preview = parse(name, input, None);
    let text = if preview.target.is_empty() || preview.target == preview.title {
        preview.title
    } else {
        format!("{} · {}", preview.title, preview.target)
    };
    let mut chars = text.chars();
    let short: String = chars.by_ref().take(180).collect();
    if chars.next().is_some() {
        format!("{short}…")
    } else {
        short
    }
}

/// One-line overview of a run of tool calls, as the desktop WorkCard header.
#[derive(Debug, Default, PartialEq)]
pub struct WorkSummary {
    pub text: String,
    pub added: usize,
    pub removed: usize,
    pub failed: usize,
    pub running: usize,
    /// First call to last result, only once every call has finished.
    pub duration_ms: Option<i64>,
}

pub fn summarize_work<'a>(rows: impl IntoIterator<Item = &'a crate::model::Row>) -> WorkSummary {
    let mut summary = WorkSummary::default();
    let mut files = std::collections::BTreeSet::new();
    let (mut commands, mut reads, mut searches, mut other) = (0, 0, 0, 0);
    let (mut start, mut end, mut finished) = (None::<i64>, None::<i64>, true);
    for row in rows {
        let Some(tool) = &row.tool else { continue };
        let preview = parse(&tool.name, &tool.input, None);
        summary.added += preview.added;
        summary.removed += preview.removed;
        if tool.is_error {
            summary.failed += 1;
        } else if !tool.complete {
            summary.running += 1;
        }
        match tool.category() {
            "Edit" => {
                files.insert(tool.target());
            }
            "Command" => commands += 1,
            "Read" => reads += 1,
            "Search" => searches += 1,
            _ => other += 1,
        }
        start = match (start, row.timestamp_ms) {
            (Some(a), Some(b)) => Some(a.min(b)),
            (a, b) => a.or(b),
        };
        match tool.completed_at_ms {
            Some(done) => end = Some(end.map_or(done, |e: i64| e.max(done))),
            None => finished = false,
        }
    }
    let plural =
        |n: usize, one: &str, many: &str| format!("{n} {}", if n == 1 { one } else { many });
    let mut parts = Vec::new();
    if !files.is_empty() {
        parts.push(plural(files.len(), "file changed", "files changed"));
    }
    if commands > 0 {
        parts.push(plural(commands, "command", "commands"));
    }
    if reads > 0 {
        parts.push(format!("read {reads}"));
    }
    if searches > 0 {
        parts.push(plural(searches, "search", "searches"));
    }
    if other > 0 {
        parts.push(plural(other, "other tool", "other tools"));
    }
    summary.text = parts.join(" · ");
    summary.duration_ms = match (finished, start, end) {
        (true, Some(start), Some(end)) if end >= start => Some(end - start),
        _ => None,
    };
    summary
}

/// Only regular adjacent tool calls group; orchestration remains independently visible.
pub fn group_span(
    rows: &std::collections::VecDeque<std::sync::Arc<crate::model::Row>>,
    ix: usize,
) -> Option<std::ops::Range<usize>> {
    let regular = |i: usize| {
        rows.get(i)
            .and_then(|r| r.tool.as_ref())
            .is_some_and(|t| !matches!(t.category(), "Subagent" | "Workflow" | "Skill"))
    };
    if !regular(ix) {
        return None;
    }
    let mut start = ix;
    while start > 0 && regular(start - 1) {
        start -= 1;
    }
    let mut end = ix + 1;
    while regular(end) {
        end += 1;
    }
    // Keep expansion bounded: a long burst must not build thousands of tool
    // cards inside one virtual-list item.
    start += ((ix - start) / 12) * 12;
    end = end.min(start + 12);
    Some(start..end)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn overview_uses_actions_and_recorded_targets() {
        assert_eq!(
            overview("Read", r#"{"file_path":"src/main.rs"}"#),
            "Read file · src/main.rs"
        );
        assert_eq!(
            overview(
                "Bash",
                r#"{"command":"cargo test","description":"Check the parser regressions"}"#
            ),
            "Check the parser regressions · cargo test"
        );
        assert_eq!(
            overview(
                "mcp__rivet__recon_search",
                r#"{"args":["timestamp_label"]}"#
            ),
            "Search · timestamp_label"
        );
        assert!(
            overview(
                "Search",
                &serde_json::json!({"query":"é".repeat(500)}).to_string()
            )
            .chars()
            .count()
                <= 181
        );
    }

    #[test]
    fn grouping_stops_at_prose_and_dispatches() {
        use crate::{model::Row, transcript::Tool};
        use std::{collections::VecDeque, sync::Arc};
        let tool = |name: &str| {
            Arc::new(Row {
                tool: Some(Tool {
                    name: name.into(),
                    ..Default::default()
                }),
                ..Default::default()
            })
        };
        let rows = VecDeque::from(vec![
            tool("Read"),
            tool("Bash"),
            tool("Grep"),
            Arc::new(Row::default()),
            tool("Read"),
            tool("Read"),
            tool("Task"),
        ]);
        assert_eq!(group_span(&rows, 0), Some(0..3));
        assert_eq!(group_span(&rows, 2), Some(0..3));
        assert_eq!(group_span(&rows, 4), Some(4..6));
        assert_eq!(group_span(&rows, 3), None);
        assert_eq!(group_span(&rows, 6), None);
        let long = (0..40).map(|_| tool("Read")).collect();
        assert_eq!(group_span(&long, 11), Some(0..12));
        assert_eq!(group_span(&long, 12), Some(12..24));
        assert_eq!(group_span(&long, 39), Some(36..40));
    }

    #[test]
    fn work_summary_counts_like_the_desktop_card() {
        use crate::{model::Row, transcript::Tool};
        let row = |name: &str, input: serde_json::Value, at: i64, done: Option<i64>| Row {
            timestamp_ms: Some(at),
            tool: Some(Tool {
                name: name.into(),
                input: input.to_string(),
                complete: done.is_some(),
                completed_at_ms: done,
                ..Default::default()
            }),
            ..Default::default()
        };
        let rows = [
            row(
                "Edit",
                json!({"file_path":"a.rs","old_string":"x","new_string":"y\nz"}),
                10,
                Some(20),
            ),
            row(
                "Edit",
                json!({"file_path":"a.rs","old_string":"q","new_string":"r"}),
                20,
                Some(30),
            ),
            row("Bash", json!({"command":"cargo test"}), 30, Some(900)),
            row("Read", json!({"file_path":"b.rs"}), 40, Some(50)),
            row("Grep", json!({"pattern":"fn"}), 50, Some(60)),
        ];
        let summary = summarize_work(rows.iter());
        assert_eq!(
            summary.text,
            "1 file changed · 1 command · read 1 · 1 search"
        );
        assert_eq!((summary.added, summary.removed), (3, 2));
        assert_eq!(summary.duration_ms, Some(890));
        let running = [row("Bash", json!({"command":"sleep 9"}), 0, None)];
        let summary = summarize_work(running.iter());
        assert_eq!((summary.running, summary.duration_ms), (1, None));
    }

    #[test]
    fn command_descriptions_and_wrapped_commands_are_readable() {
        let p = parse("Bash", &json!({"description":"Run the native test suite", "command":"/usr/bin/bash -lc 'cargo test --locked'", "cwd":"/work/project"}).to_string(), Some("passed"));
        assert_eq!(p.title, "Run the native test suite");
        assert_eq!(p.target, "cargo test --locked");
        assert_eq!(p.blocks[0].text, "cargo test --locked");
        assert_eq!(p.working_directory, "/work/project");
        assert_eq!(p.blocks[1].label, "Output");
    }

    #[test]
    fn command_arrays_preserve_quotes_and_complex_shell_commands() {
        let p = parse(
            "shell",
            &json!({"command":["bash","-c","printf '%s' 'hello world'"]}).to_string(),
            None,
        );
        assert_eq!(p.target, "printf '%s' 'hello world'");
        assert_eq!(p.title, "Run printf");
        let p = parse(
            "exec_command",
            &json!({"cmd":"cargo test\nprintf done"}).to_string(),
            None,
        );
        assert_eq!(p.target, "cargo test ↵ printf done");
        assert_eq!(p.blocks[0].text, "cargo test\nprintf done");
        let command = "bash -c 'echo first' && echo second";
        let p = parse("shell", &json!({"command":command}).to_string(), None);
        assert_eq!(p.blocks[0].text, command);
        let p = parse(
            "shell",
            &json!({"command":["rg","hello world","file name.rs"]}).to_string(),
            None,
        );
        assert_eq!(p.blocks[0].text, "rg 'hello world' 'file name.rs'");
    }

    #[test]
    fn claude_edits_keep_both_sides_with_file_language() {
        let p = parse(
            "Edit",
            &json!({"file_path":"src/app.rs","old_string":"let n = 1;", "new_string":"let n = 2;"})
                .to_string(),
            Some("Done"),
        );
        assert!(p.file_edit);
        assert_eq!((p.added, p.removed), (1, 1));
        assert_eq!(p.blocks[0].language, "rust");
        assert_eq!(p.blocks[1].text, "let n = 2;");
    }

    #[test]
    fn codex_multifile_changes_do_not_duplicate_combined_diff() {
        let p = parse("apply_patch", &json!({"diff":"duplicate", "changes":[{"path":"a.ts","diff":"@@\n-old\n+new"},{"path":"b.rs","diff":"@@\n+added"}]}).to_string(), None);
        assert_eq!(p.blocks.len(), 2);
        assert_eq!((p.added, p.removed), (2, 1));
        assert_eq!(p.blocks[1].label, "b.rs");
    }

    #[test]
    fn unified_patch_previews_keep_deleted_files_and_separate_file_sections() {
        let patch = "--- a/old.rs\n+++ /dev/null\n@@ -1 +0,0 @@\n-gone\n--- a/next.rs\n+++ b/next.rs\n@@ -1 +1 @@\n-before\n+after\n";
        let preview = parse(
            "functions.apply_patch",
            &serde_json::json!({"diff":patch}).to_string(),
            None,
        );
        assert_eq!(preview.target, "old.rs");
        assert_eq!((preview.added, preview.removed), (1, 2));
        assert_eq!(preview.blocks.len(), 2);
        assert_eq!(preview.blocks[0].label, "old.rs");
        assert!(preview.blocks[0].text.contains("+++ /dev/null"));
        assert!(!preview.blocks[0].text.contains("next.rs"));
        assert_eq!(preview.blocks[1].label, "next.rs");
        assert!(preview.blocks[1].text.starts_with("--- a/next.rs"));
    }

    #[test]
    fn raw_patch_and_read_results_are_presented_without_file_io() {
        let p = parse(
            "functions.apply_patch",
            &json!("*** Begin Patch\n*** Update File: app.py\n@@\n-old\n+new\n*** End Patch")
                .to_string(),
            None,
        );
        assert_eq!(p.target, "app.py");
        assert_eq!((p.added, p.removed), (1, 1));
        let read = parse(
            "Read",
            &json!({"file_path":"app.py"}).to_string(),
            Some("  1\tdef greet():\n  2→    return 'hi'"),
        );
        assert_eq!(read.blocks[0].language, "python");
        assert_eq!(read.blocks[0].text, "def greet():\n    return 'hi'");
    }

    #[test]
    fn unknown_malformed_and_markdown_like_output_remains_literal() {
        let p = parse("custom_tool", "{broken", Some("```\n# not a heading\n````"));
        assert_eq!(p.title, "custom_tool");
        assert_eq!(p.blocks.len(), 2);
        assert!(fenced("text", &p.blocks[1].text).starts_with("`````text\n"));
    }
    #[test]
    fn namespaced_tools_keep_main_camel_case_edit_previews() {
        let p = parse(
            "mcp__workspace__Edit",
            &json!({"file_path":"main.rs", "oldString":"before", "newString":"after"}).to_string(),
            None,
        );
        assert!(p.file_edit);
        assert_eq!(p.blocks[0].text, "before");
        assert_eq!(p.blocks[1].text, "after");
        assert_eq!(p.blocks[0].language, "rust");
    }
}
