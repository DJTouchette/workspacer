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
        let mut path = fallback_path.to_owned();
        let mut lines = String::new();
        for line in patch.lines() {
            if matches!(line, "*** Begin Patch" | "*** End Patch") {
                continue;
            }
            let next = ["*** Update File: ", "*** Add File: ", "*** Delete File: "]
                .iter()
                .find_map(|prefix| line.strip_prefix(prefix));
            if let Some(next) = next {
                self.diff(&path, &lines);
                path = next.into();
                if self.target.is_empty() {
                    self.target = path.clone();
                }
                lines.clear();
                lines.push_str(line);
                lines.push('\n');
            } else {
                lines.push_str(line);
                lines.push('\n');
            }
        }
        self.diff(&path, &lines);
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
        _ => {}
    }
    if !preview.description.is_empty() {
        preview.title = one_line(&preview.description);
    }
    if preview.target.is_empty() {
        preview.target = one_line(field(&input, &["query", "pattern", "url", "prompt"]));
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

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

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
