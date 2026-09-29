use serde_json::{Value, json};
use std::{
    ffi::OsStr,
    path::{Path, PathBuf},
};
pub fn on_path(binary: &str) -> Option<PathBuf> {
    let path = Path::new(binary);
    if path.components().count() > 1 {
        return path.is_file().then(|| path.to_path_buf());
    }
    let names = if cfg!(windows) {
        let mut names = std::env::var("PATHEXT")
            .unwrap_or(".COM;.EXE;.BAT;.CMD".into())
            .split(';')
            .filter(|s| !s.is_empty())
            .map(|ext| format!("{binary}{}", ext.to_lowercase()))
            .collect::<Vec<_>>();
        names.push(binary.into());
        names
    } else {
        vec![binary.into()]
    };
    std::env::split_paths(&std::env::var_os("PATH").unwrap_or_default())
        .filter(|dir| !dir.as_os_str().is_empty())
        .flat_map(|dir| names.iter().map(move |name| dir.join(name)))
        .find(|path| path.is_file())
}
pub fn status() -> Value {
    let git = if cfg!(target_os = "macos") {
        "Install with `xcode-select --install` (or `brew install git`)"
    } else if cfg!(windows) {
        "Install with `winget install Git.Git` — https://git-scm.com/downloads"
    } else {
        "Install git with your package manager (e.g. `pacman -S git`, `apt install git`)"
    };
    Value::Array([
        ("git","Git",vec!["Review changes pane","Per-turn changed-files cards","Branch display in the status bar","Worktree isolation"],git),
        ("claude","Claude Code",vec!["Claude agents"],"Install with `npm install -g @anthropic-ai/claude-code`"),
        ("codex","Codex CLI",vec!["Codex agents"],"Install with `npm install -g @openai/codex`"),
        ("copilot","GitHub Copilot CLI",vec!["GitHub Copilot agents"],"Install with `npm install -g @github/copilot`"),
        ("opencode","OpenCode",vec!["OpenCode agents"],"Install from https://opencode.ai"),
        ("pi","Pi",vec!["Pi agents"],"Install with `npm install -g @earendil-works/pi-coding-agent`"),
        ("tailscale","Tailscale",vec!["Remote share (HTTPS via tailscale serve)"],"Install from https://tailscale.com/download"),
    ].into_iter().map(|(id,label,features,install)|{let path=on_path(id);let mut value=json!({"id":id,"label":label,"bin":id,"features":features,"install":install,"available":path.is_some()});if let Some(path)=path{value["path"]=path.to_string_lossy().as_ref().into();}value}).collect())
}
/// npm's Windows command shim is never a shell boundary for prompt/model text.
/// Resolve only the provider's declared package entrypoint in that installation.
pub fn launcher(provider: &str, binary: &Path) -> anyhow::Result<Vec<String>> {
    let extension = binary.extension().and_then(OsStr::to_str).unwrap_or("");
    if !extension.eq_ignore_ascii_case("cmd") && !extension.eq_ignore_ascii_case("bat") {
        return Ok(vec![binary.to_string_lossy().into_owned()]);
    }
    let package = match provider {
        "claude" => "@anthropic-ai/claude-code",
        "codex" => "@openai/codex",
        "copilot" => "@github/copilot",
        "opencode" => "opencode-ai",
        "pi" => "@earendil-works/pi-coding-agent",
        _ => anyhow::bail!("unknown provider launcher"),
    };
    let directory = binary
        .parent()
        .ok_or_else(|| anyhow::anyhow!("launcher directory missing"))?;
    let root = directory
        .join("node_modules")
        .join(package)
        .canonicalize()?;
    let metadata = root.join("package.json");
    anyhow::ensure!(
        metadata.metadata()?.len() <= 128 * 1024,
        "launcher metadata too large"
    );
    let value: Value = serde_json::from_slice(&std::fs::read(metadata)?)?;
    anyhow::ensure!(value["name"] == package, "launcher package mismatch");
    let entry = value["bin"]
        .as_str()
        .or_else(|| value["bin"][provider].as_str())
        .ok_or_else(|| anyhow::anyhow!("provider entrypoint missing"))?;
    let script = root.join(entry).canonicalize()?;
    anyhow::ensure!(
        script.starts_with(&root)
            && script.is_file()
            && matches!(
                script.extension().and_then(OsStr::to_str),
                Some("js" | "mjs" | "cjs")
            ),
        "unsupported provider entrypoint"
    );
    let sibling = directory.join("node.exe");
    let node = if sibling.is_file() {
        sibling
    } else {
        on_path("node").ok_or_else(|| anyhow::anyhow!("Node runtime missing"))?
    };
    anyhow::ensure!(
        !matches!(
            node.extension().and_then(OsStr::to_str),
            Some("cmd" | "bat")
        ),
        "native Node runtime required"
    );
    Ok(vec![
        node.to_string_lossy().into_owned(),
        script.to_string_lossy().into_owned(),
    ])
}
