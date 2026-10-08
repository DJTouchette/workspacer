use super::{
    text::{self, Failure},
    tools,
};
use crate::services::owned_process;
use claudemon::daemon::embedded::{Command as EngineCommand, EmbeddedClient};
use serde_json::{Value, json};
use std::{
    path::{Path, PathBuf},
    time::Duration,
};
use tokio::process::Command;
pub type Outcome<T> = Result<T, Failure>;
pub fn process_error(error: anyhow::Error) -> Failure {
    if error
        .downcast_ref::<std::io::Error>()
        .is_some_and(|e| e.kind() == std::io::ErrorKind::NotFound)
    {
        Failure::Missing
    } else if error.to_string().contains("timed out") {
        Failure::Timeout
    } else {
        Failure::Error
    }
}
pub async fn run(
    command: &mut Command,
    input: &str,
    limit: usize,
    timeout: Duration,
) -> Outcome<String> {
    let output = owned_process::capture_input(command, input.as_bytes(), limit, limit, timeout)
        .await
        .map_err(process_error)?;
    let stdout = String::from_utf8_lossy(&output.stdout).into_owned();
    if !output.status.success() {
        return Err(text::classify(&format!(
            "{}\n{}",
            String::from_utf8_lossy(&output.stderr),
            stdout
        )));
    }
    Ok(stdout)
}
pub fn binary(provider: &str, config: &Value) -> Option<PathBuf> {
    let requested = config["agents"]["binaries"][provider]
        .as_str()
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .unwrap_or(provider);
    tools::on_path(requested)
}
/// How long a one-shot may run and how much of its answer is kept.
#[derive(Clone, Copy, Debug)]
pub struct Limits {
    pub timeout: Duration,
    /// UTF-16 units kept from the answer.
    pub chars: usize,
    /// Bytes captured from a CLI's stdout (JSON event lines included).
    pub output: usize,
    /// Claude only: no built-in or MCP tools, no hooks, no persisted session.
    pub no_tools: bool,
}
impl Limits {
    pub const TITLE: Self = Self {
        timeout: Duration::from_secs(25),
        chars: 416,
        output: 64 * 1024,
        no_tools: false,
    };
    /// A handoff brief: a page of Markdown, written without tools.
    pub const BRIEF: Self = Self {
        timeout: Duration::from_secs(90),
        chars: 24_000,
        output: 512 * 1024,
        no_tools: true,
    };
}
pub async fn title(
    provider: &str,
    model: Option<&str>,
    config: &Value,
    home: &Path,
    engine: Option<&EmbeddedClient>,
    prompt: &str,
) -> Outcome<String> {
    oneshot(provider, model, config, home, engine, prompt, Limits::TITLE).await
}
/// One disposable turn with the configured harness: the prompt in, text out.
pub async fn oneshot(
    provider: &str,
    model: Option<&str>,
    config: &Value,
    home: &Path,
    engine: Option<&EmbeddedClient>,
    prompt: &str,
    limits: Limits,
) -> Outcome<String> {
    if !text::TITLE_PROVIDERS.contains(&provider) {
        return Err(Failure::Unsupported);
    }
    let binary = binary(provider, config).ok_or(Failure::Missing)?;
    let argv = tools::launcher(provider, &binary).map_err(|_| Failure::Unsupported)?;
    let model = model.map(str::to_owned);
    if provider == "claude" {
        let engine = engine.ok_or(Failure::Unsupported)?;
        let value = tokio::time::timeout(
            limits.timeout + Duration::from_secs(5),
            engine.request(EngineCommand::Request {
                method: "POST".into(),
                path: "/oneshot".into(),
                payload: Some(json!({"argv":argv,"model":model,"prompt":prompt,
                    "timeout_secs":limits.timeout.as_secs(),"no_tools":limits.no_tools})),
            }),
        )
        .await
        .map_err(|_| Failure::Timeout)?
        .map_err(|_| Failure::Error)?;
        if value["ok"] != true {
            return Err(text::classify(value["error"].as_str().unwrap_or("failed")));
        }
        return value["text"]
            .as_str()
            .map(|text| text::clip(text, limits.chars))
            .filter(|s| !s.trim().is_empty())
            .ok_or(Failure::Empty);
    }
    let mut command = Command::new(&argv[0]);
    command.args(&argv[1..]).current_dir(home);
    let input = match provider {
        "codex" => {
            command.args([
                "exec",
                "--skip-git-repo-check",
                "--ephemeral",
                "--sandbox",
                "read-only",
                "--color",
                "never",
                "--json",
            ]);
            if let Some(model) = &model {
                command.args(["--model", model]);
            }
            command.arg("-");
            prompt
        }
        "opencode" => {
            command.args(["run", "--pure", "--format", "json"]);
            if let Some(model) = &model {
                command.args(["--model", model]);
            }
            prompt
        }
        "pi" => {
            command.args(["--print", "--no-tools", "--no-session", "--mode", "text"]);
            if let Some(model) = &model {
                command.args(["--model", model]);
            }
            command.arg(prompt);
            ""
        }
        "copilot" => {
            command.args([
                "--silent",
                "--no-auto-update",
                "--no-remote",
                "--no-remote-export",
                "--no-ask-user",
                "--no-color",
                "--no-custom-instructions",
                "--available-tools=",
            ]);
            if let Some(model) = &model {
                command.args(["--model", model]);
            }
            command.args(["--prompt", prompt]);
            ""
        }
        _ => return Err(Failure::Unsupported),
    };
    // Launcher never uses cmd.exe: Pi/Copilot positional text has no shell grammar.
    let raw = run(&mut command, input, limits.output, limits.timeout).await?;
    let result = text::clip(text::extract(provider, &raw).trim(), limits.chars);
    if result.is_empty() {
        Err(Failure::Empty)
    } else {
        Ok(result)
    }
}
pub fn native(path: &Path) -> bool {
    use std::io::Read;
    let Ok(mut file) = std::fs::File::open(path) else {
        return false;
    };
    if !file.metadata().is_ok_and(|m| m.is_file()) {
        return false;
    }
    let mut magic = [0; 4];
    if file.read_exact(&mut magic).is_err() {
        return false;
    }
    matches!(
        magic,
        [0x7f, b'E', b'L', b'F']
            | [0xcf, 0xfa, 0xed, 0xfe]
            | [0xfe, 0xed, 0xfa, 0xcf]
            | [0xca, 0xfe, 0xba, 0xbe]
    )
}
pub async fn claude_ping(binary: &Path) -> Outcome<()> {
    if cfg!(windows) || !native(binary) {
        return Err(Failure::Unsupported);
    }
    let command = || {
        let mut c = Command::new(binary);
        c.current_dir(std::env::temp_dir()).envs([
            ("CLAUDE_CODE_SAFE_MODE", "1"),
            ("CLAUDE_CODE_MAX_RETRIES", "0"),
            ("CLAUDE_CODE_RETRY_WATCHDOG", "0"),
            ("CLAUDE_CODE_MAX_OUTPUT_TOKENS", "64"),
            ("MAX_THINKING_TOKENS", "0"),
            ("DISABLE_AUTOUPDATER", "1"),
            ("CLAUDE_CODE_DISABLE_NONESSENTIAL_TRAFFIC", "1"),
        ]);
        c
    };
    let help = run(command().arg("--help"), "", 32000, Duration::from_secs(3)).await?;
    if ![
        "--safe-mode",
        "--tools",
        "--strict-mcp-config",
        "--no-session-persistence",
        "--system-prompt",
        "--disable-slash-commands",
        "--output-format",
    ]
    .iter()
    .all(|flag| help.contains(flag))
    {
        return Err(Failure::Unsupported);
    }
    let raw = run(
        command().args([
            "--safe-mode",
            "--print",
            "--tools",
            "",
            "--strict-mcp-config",
            "--mcp-config",
            "{\"mcpServers\":{}}",
            "--disable-slash-commands",
            "--no-session-persistence",
            "--system-prompt",
            "Reply OK.",
            "--output-format",
            "json",
            "--model",
            "haiku",
        ]),
        "Reply OK.",
        8000,
        Duration::from_secs(15),
    )
    .await?;
    let value: Value = serde_json::from_str(&raw).map_err(|_| Failure::Error)?;
    if value["type"] != "result" || value["is_error"] != false {
        return Err(if value["is_error"] == true {
            text::classify(value["result"].as_str().unwrap_or("failed"))
        } else {
            Failure::Error
        });
    }
    if value["result"].as_str().is_none_or(|s| s.trim() != "OK") {
        return Err(Failure::Empty);
    }
    Ok(())
}
