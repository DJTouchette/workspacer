use super::{files::GIT_NO_EXEC, paths};
use crate::Options;
use anyhow::{Result, anyhow, bail};
use serde_json::{Value, json};
use std::{
    path::{Path, PathBuf},
    time::Duration,
};

pub(crate) fn install(mut options: Options) -> Options {
    for method in [
        "git.status",
        "git.log",
        "git.diff",
        "git.numstat",
        "git.commitDiff",
        "git.commitNumstat",
        "git.stage",
        "git.unstage",
        "git.commit",
        "git.push",
    ] {
        options = options.handler(method, move |_, params| async move {
            call(method, params).await
        });
    }
    options
}
struct Output {
    ok: bool,
    stdout: String,
    stderr: String,
}
async fn run(cwd: &Path, args: &[String]) -> Result<Output> {
    let mut command = tokio::process::Command::new("git");
    command.current_dir(cwd);
    for pair in GIT_NO_EXEC {
        command.args(["-c", pair]);
    }
    let mut command_seen = false;
    let mut config_value = false;
    for arg in args {
        command.arg(arg);
        if command_seen {
            continue;
        }
        if config_value {
            config_value = false;
            continue;
        }
        if arg == "-c" {
            config_value = true;
            continue;
        }
        command_seen = true;
        if [
            "diff",
            "show",
            "log",
            "format-patch",
            "diff-index",
            "diff-tree",
            "diff-files",
            "range-diff",
            "whatchanged",
        ]
        .contains(&arg.as_str())
        {
            command.arg("--no-ext-diff");
        }
    }
    let output = super::owned_process::capture_limits(
        &mut command,
        256 * 1024 * 1024,
        1024 * 1024,
        Duration::from_secs(60),
    )
    .await
    .map_err(|error| {
        if let Some(limit) = error.downcast_ref::<super::owned_process::OutputLimit>() {
            anyhow!("git output exceeded the size cap ({} bytes)", limit.0)
        } else if error
            .downcast_ref::<std::io::Error>()
            .is_some_and(|e| e.kind() == std::io::ErrorKind::NotFound)
        {
            anyhow!("could not run git (is it installed and on PATH?): {error}")
        } else {
            error
        }
    })?;
    Ok(Output {
        ok: output.status.success(),
        stdout: String::from_utf8_lossy(&output.stdout).into(),
        stderr: String::from_utf8_lossy(&output.stderr).into(),
    })
}

fn args(values: &[&str]) -> Vec<String> {
    values.iter().map(|s| (*s).into()).collect()
}
async fn root(cwd: &Path) -> Result<PathBuf> {
    let result = run(cwd, &args(&["rev-parse", "--show-toplevel"])).await?;
    if !result.ok || result.stdout.trim().is_empty() {
        bail!("not a git repository");
    }
    paths::canonicalize(Path::new(result.stdout.trim()))
}
fn operand(root: &Path, path: &str) -> Result<String> {
    let path = Path::new(path);
    let anchored = if path.is_absolute() {
        path.to_path_buf()
    } else {
        root.join(path)
    };
    let canonical = paths::canonicalize(&anchored)?;
    if !paths::contained(&canonical, root) {
        bail!("path is outside the selected repository");
    }
    let relative = canonical.strip_prefix(root).unwrap_or(&canonical);
    Ok(if relative.as_os_str().is_empty() {
        canonical.to_string_lossy().into()
    } else {
        relative.to_string_lossy().into()
    })
}
fn read_error(output: &Output, fallback: &str) -> anyhow::Error {
    anyhow!(
        if output.stderr.trim().is_empty() {
            fallback
        } else {
            output.stderr.trim()
        }
        .to_owned()
    )
}

pub fn porcelain(text: &str) -> Vec<Value> {
    let tokens: Vec<_> = text.split('\0').collect();
    let mut out = Vec::new();
    let mut i = 0;
    while i < tokens.len() {
        let entry = tokens[i];
        i += 1;
        if entry.len() < 4 || !entry.is_char_boundary(3) {
            continue;
        }
        let staged = &entry[..1];
        let unstaged = &entry[1..2];
        let mut row = json!({"path":&entry[3..],"staged":staged,"unstaged":unstaged});
        if matches!(staged, "R" | "C") || matches!(unstaged, "R" | "C") {
            if let Some(original) = tokens.get(i).filter(|s| !s.is_empty()) {
                row["orig_path"] = json!(original);
            }
            i += 1;
        }
        out.push(row);
    }
    out
}
pub fn numstat(text: &str) -> Vec<Value> {
    text.lines()
        .filter_map(|line| {
            let mut fields = line.splitn(3, '\t');
            let added = fields.next()?.trim().parse::<i64>().ok();
            let deleted = fields.next()?.trim().parse::<i64>().ok();
            let raw = fields.next()?.trim_end_matches('\r');
            let mut path = raw.to_owned();
            if let (Some(start), Some(end)) = (raw.find('{'), raw.find('}'))
                && start < end
                && let Some((_, new)) = raw[start + 1..end].split_once(" => ")
            {
                path =
                    format!("{}{}{}", &raw[..start], new, &raw[end + 1..]).replacen("//", "/", 1);
            }
            if path == raw
                && let Some((_, new)) = raw.split_once(" => ")
            {
                path = new.into();
            }
            Some(json!({"path":path,"added":added,"deleted":deleted}))
        })
        .collect()
}

pub async fn call(method: &str, params: Value) -> Result<Value> {
    let requested = params["cwd"].as_str().unwrap_or("");
    let cwd = paths::canonicalize(Path::new(requested))?;
    let root = root(&cwd).await?;
    let path = params["path"].as_str().unwrap_or("");
    let staged = params["staged"].as_bool().unwrap_or(false);
    match method {
        "git.status" => {
            let result = run(
                &root,
                &args(&[
                    "status",
                    "--porcelain",
                    "-z",
                    "--branch",
                    "--untracked-files=all",
                ]),
            )
            .await?;
            if !result.ok {
                return Err(read_error(&result, "git status failed"));
            }
            let (header, body) = if result.stdout.starts_with("## ") {
                result
                    .stdout
                    .split_once('\0')
                    .unwrap_or((&result.stdout, ""))
            } else {
                ("", result.stdout.as_str())
            };
            let mut upstream = Value::Null;
            let mut ahead = 0;
            let mut behind = 0;
            if let Some((_, rest)) = header.split_once("...") {
                let (name, tracking) = rest.split_once(" [").unwrap_or((rest, ""));
                if tracking != "gone]" {
                    if !name.is_empty() {
                        upstream = json!(name);
                    }
                    for (keyword, number) in [("ahead ", &mut ahead), ("behind ", &mut behind)] {
                        if let Some((_, suffix)) = tracking.split_once(keyword) {
                            *number = suffix
                                .chars()
                                .take_while(char::is_ascii_digit)
                                .collect::<String>()
                                .parse::<u64>()
                                .unwrap_or(0);
                        }
                    }
                }
            }
            let branch = run(&root, &args(&["rev-parse", "--abbrev-ref", "HEAD"]))
                .await
                .ok()
                .filter(|r| r.ok)
                .map(|r| r.stdout.trim().to_owned())
                .filter(|s| !s.is_empty() && s != "HEAD");
            Ok(
                json!({"branch":branch,"files":porcelain(body),"upstream":upstream,"ahead":ahead,"behind":behind}),
            )
        }
        "git.log" => {
            let limit = params["limit"]
                .as_i64()
                .filter(|n| *n > 0)
                .unwrap_or(5)
                .min(50)
                .to_string();
            let result = run(
                &root,
                &args(&["log", "-n", &limit, "--pretty=format:%h%x00%s%x00%at"]),
            )
            .await?;
            let commits: Vec<_> = if !result.ok {
                vec![]
            } else {
                result.stdout.lines().filter_map(|line|{let fields:Vec<_>=line.split('\0').collect();if fields.len()<3||fields[0].is_empty(){return None;}Some(json!({"hash":fields[0],"subject":fields[1],"authoredAt":fields[2].parse::<i64>().ok()?}))}).collect()
            };
            Ok(json!({"commits":commits}))
        }
        "git.diff" | "git.numstat" => {
            let untracked = params["untracked"].as_bool().unwrap_or(false);
            let mut argv = args(&["-c", "core.quotepath=false", "diff"]);
            if method == "git.numstat" {
                argv.push("--numstat".into());
            }
            if untracked {
                if path.is_empty() {
                    bail!("untracked diff requires a path");
                }
                if path.ends_with(['/', '\\']) {
                    bail!("untracked diff path is a directory");
                }
                argv.extend(args(&["--no-index", "--", "/dev/null"]));
                argv.push(operand(&root, path)?);
            } else {
                if staged {
                    argv.push("--staged".into());
                }
                if !path.is_empty() {
                    argv.push("--".into());
                    argv.push(operand(&root, path)?);
                }
            }
            let result = run(&root, &argv).await?;
            if !result.ok && !(untracked && !result.stdout.is_empty()) {
                return Err(read_error(&result, "git diff failed"));
            }
            if method == "git.numstat" {
                Ok(json!({"files":numstat(&result.stdout)}))
            } else {
                Ok(json!({"diff":result.stdout}))
            }
        }
        "git.commitDiff" | "git.commitNumstat" => {
            let hash = params["hash"].as_str().unwrap_or("").trim();
            if !(4..=40).contains(&hash.len()) || !hash.bytes().all(|b| b.is_ascii_hexdigit()) {
                bail!("not a commit hash");
            }
            let mut argv = args(&[
                "show",
                "--format=",
                if method == "git.commitDiff" {
                    "--patch"
                } else {
                    "--numstat"
                },
                hash,
            ]);
            if method == "git.commitDiff" && !path.is_empty() {
                argv.push("--".into());
                argv.push(operand(&root, path)?);
            }
            let result = run(&root, &argv).await?;
            if !result.ok {
                return Err(read_error(&result, "git show failed"));
            }
            if method == "git.commitDiff" {
                Ok(json!({"diff":result.stdout}))
            } else {
                Ok(json!({"files":numstat(&result.stdout)}))
            }
        }
        "git.stage" | "git.unstage" | "git.commit" | "git.push" => {
            let argv = match method {
                "git.stage" | "git.unstage" => {
                    let target = if path.is_empty() {
                        let relative = cwd
                            .strip_prefix(&root)
                            .map_err(|_| anyhow!("cwd is outside repository"))?;
                        if relative.as_os_str().is_empty() {
                            ".".into()
                        } else {
                            relative.to_string_lossy().into_owned()
                        }
                    } else {
                        operand(&root, path)?
                    };
                    let mut argv = if method == "git.stage" {
                        args(&["add", "-A", "--"])
                    } else {
                        args(&["reset", "-q", "HEAD", "--"])
                    };
                    argv.push(target);
                    argv
                }
                "git.commit" => {
                    let message = params["message"].as_str().unwrap_or("");
                    if message.trim().is_empty() {
                        bail!("empty commit message");
                    }
                    args(&["commit", "-m", message])
                }
                _ => args(&["push"]),
            };
            let result = run(&root, &argv).await?;
            if !result.ok {
                bail!(
                    "{}",
                    format_action_error(
                        &format!("{}\n{}", result.stderr, result.stdout),
                        &format!("git {} failed", argv[0])
                    )
                );
            }
            Ok(json!({"ok":true,"output":result.stdout}))
        }
        _ => bail!("unknown git operation"),
    }
}
pub fn format_action_error(raw: &str, fallback: &str) -> String {
    let message = raw.trim();
    let message = if message.is_empty() {
        fallback
    } else {
        message
    };
    let lower = message.to_lowercase();
    for (needles, summary) in [
        (
            &[
                "you have unmerged paths",
                "fix conflicts",
                "conflict (",
                "merge conflict",
            ][..],
            "Merge conflicts need resolution before this git action can continue. Resolve the conflicted files, stage them, then retry.",
        ),
        (
            &["no changes added to commit", "nothing to commit"][..],
            "Nothing is staged to commit. Stage files in Review, then commit again.",
        ),
        (
            &["no upstream branch"][..],
            "No upstream branch is configured. Set an upstream with git push --set-upstream, then retry.",
        ),
        (
            &[
                "non-fast-forward",
                "fetch first",
                "updates were rejected",
                "rejected",
            ][..],
            "Push was rejected because the remote has changes this branch does not have. Pull or rebase, resolve anything needed, then retry.",
        ),
    ] {
        if needles.iter().any(|s| lower.contains(s)) {
            return format!("{summary}\n\n{message}");
        }
    }
    message.into()
}
