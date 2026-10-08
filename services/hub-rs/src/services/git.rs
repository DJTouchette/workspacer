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
const DIFF_FAMILY: &[&str] = &[
    "diff",
    "show",
    "log",
    "format-patch",
    "diff-index",
    "diff-tree",
    "diff-files",
    "range-diff",
    "whatchanged",
];
fn command(cwd: &Path, args: &[String]) -> tokio::process::Command {
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
        if DIFF_FAMILY.contains(&arg.as_str()) {
            command.arg("--no-ext-diff");
        }
    }
    command
}
async fn run(cwd: &Path, args: &[String]) -> Result<Output> {
    let mut command = command(cwd, args);
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
        bail!("cwd is not inside a git work tree");
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
        if entry.len() < 4 || !(1..=3).all(|offset| entry.is_char_boundary(offset)) {
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

fn branch_header(header: &str) -> (Value, u64, u64) {
    let mut upstream = Value::Null;
    let mut ahead = 0;
    let mut behind = 0;
    if header.starts_with("## ")
        && let Some((_, rest)) = header.split_once("...")
    {
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
    (upstream, ahead, behind)
}
fn log_rows(text: &str) -> Vec<Value> {
    text.split('\n').filter_map(|line| {
        let fields:Vec<_>=line.split('\0').collect();
        if fields.len()<3 || fields[0].is_empty() {return None;}
        Some(json!({"hash":fields[0],"subject":fields[1],"authoredAt":fields[2].parse::<i64>().ok()?}))
    }).collect()
}

pub async fn call(method: &str, params: Value) -> Result<Value> {
    // Decode consumed fields as the legacy typed request did. In particular,
    // an invalid path must never turn a single-file stage into stage-all.
    let strings: &[&str] = match method {
        "git.diff" | "git.stage" | "git.unstage" => &["cwd", "path"],
        "git.commitDiff" => &["cwd", "hash", "path"],
        "git.commitNumstat" => &["cwd", "hash"],
        "git.commit" => &["cwd", "message"],
        _ => &["cwd"],
    };
    let booleans: &[&str] = if method == "git.diff" {
        &["staged", "untracked"]
    } else if method == "git.numstat" {
        &["staged"]
    } else {
        &[]
    };
    for key in params.as_object().into_iter().flat_map(|map| map.keys()) {
        let folded: String = key
            .chars()
            .map(|c| match c {
                'ſ' => 's',
                'K' => 'k',
                _ => c.to_ascii_lowercase(),
            })
            .collect();
        if strings
            .iter()
            .chain(booleans)
            .copied()
            .chain((method == "git.log").then_some("limit"))
            .any(|canonical| key != canonical && folded == canonical.to_ascii_lowercase())
        {
            bail!("non-canonical {method} field {key:?}");
        }
    }
    for key in strings {
        if params
            .get(*key)
            .is_some_and(|v| !v.is_null() && !v.is_string())
        {
            bail!("{key} must be text");
        }
    }
    for key in booleans {
        if params
            .get(*key)
            .is_some_and(|v| !v.is_null() && !v.is_boolean())
        {
            bail!("{key} must be a boolean");
        }
    }
    if method == "git.log"
        && params
            .get("limit")
            .is_some_and(|v| !v.is_null() && v.as_i64().is_none())
    {
        bail!("limit must be an integer");
    }
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
            let (upstream, ahead, behind) = branch_header(header);
            let branch = run(&root, &args(&["rev-parse", "--abbrev-ref", "HEAD"]))
                .await
                .ok()
                .filter(|r| r.ok)
                .map(|r| r.stdout.trim().to_owned())
                .filter(|s| !s.is_empty() && s != "HEAD");
            Ok(
                json!({"branch":branch,"files":porcelain(body),"upstream":upstream,"ahead":ahead,"behind":behind,"root":root}),
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
                log_rows(&result.stdout)
            };
            Ok(json!({"commits":commits}))
        }
        "git.diff" | "git.numstat" => {
            let untracked = method == "git.diff" && params["untracked"].as_bool().unwrap_or(false);
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
                if method == "git.diff" && !path.is_empty() {
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
                // A branch with no upstream yet (an agent's new worktree
                // branch) is published and tracked on its first push, as
                // `push -u <remote> <branch>`, instead of failing.
                _ => args(&["-c", "push.autoSetupRemote=true", "push"]),
            };
            let result = run(&root, &argv).await?;
            if !result.ok {
                bail!(
                    "{}",
                    format_action_error(
                        &format!("{}\n{}", result.stderr, result.stdout),
                        &format!("git {} failed", argv[if argv[0] == "-c" { 2 } else { 0 }])
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

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn branch_and_log_parsers_retain_legacy_vectors() {
        for (header, upstream, ahead, behind) in [
            ("## master", Value::Null, 0, 0),
            ("## master...origin/master", json!("origin/master"), 0, 0),
            (
                "## m...origin/m [ahead 1, behind 2]",
                json!("origin/m"),
                1,
                2,
            ),
            ("## m...origin/m [ahead 3]", json!("origin/m"), 3, 0),
            ("## m...origin/m [gone]", Value::Null, 0, 0),
            ("## HEAD (no branch)", Value::Null, 0, 0),
            ("## No commits yet on main", Value::Null, 0, 0),
            ("M a.ts", Value::Null, 0, 0),
        ] {
            assert_eq!(branch_header(header), (upstream, ahead, behind), "{header}");
        }
        assert_eq!(
            log_rows("abc123\0first subject\01700000000\ndef456\0second\01700000001\n"),
            vec![
                json!({"hash":"abc123","subject":"first subject","authoredAt":1700000000}),
                json!({"hash":"def456","subject":"second","authoredAt":1700000001})
            ]
        );
        assert!(log_rows("abc\0s\0notanumber\n\0empty hash\01700000000\n").is_empty());
    }
    #[test]
    fn real_command_prefix_and_diff_family_match_desktop_twin() {
        let source = include_str!("../../../../apps/desktop/src/main/lib/gitExec.ts");
        let strings = |start: &str, end: &str| -> std::collections::BTreeSet<String> {
            source
                .split_once(start)
                .unwrap()
                .1
                .split_once(end)
                .unwrap()
                .0
                .lines()
                .filter_map(|line| {
                    line.trim()
                        .trim_end_matches(',')
                        .strip_prefix('\'')
                        .and_then(|s| s.strip_suffix('\''))
                        .map(str::to_owned)
                })
                .collect()
        };
        assert_eq!(
            strings("export const GIT_NO_EXEC_KEYS: readonly string[] = [", "]"),
            GIT_NO_EXEC.iter().map(|s| s.to_string()).collect()
        );
        assert_eq!(
            strings("const DIFF_FAMILY = new Set([", "])"),
            DIFF_FAMILY.iter().map(|s| s.to_string()).collect()
        );
        for subcommand in DIFF_FAMILY.iter().copied().chain(["status", "rev-parse"]) {
            let incoming = args(&["-c", "core.quotepath=false", subcommand, "fixture-argument"]);
            let cmd = command(Path::new("fixture-cwd"), &incoming);
            let argv: Vec<_> = cmd
                .as_std()
                .get_args()
                .map(|s| s.to_str().unwrap())
                .collect();
            let prefix: Vec<_> = GIT_NO_EXEC.iter().flat_map(|pair| ["-c", *pair]).collect();
            assert_eq!(&argv[..prefix.len()], prefix);
            assert_eq!(
                &argv[prefix.len()..prefix.len() + 3],
                ["-c", "core.quotepath=false", subcommand]
            );
            let tail = &argv[prefix.len() + 3..];
            assert_eq!(
                tail,
                if DIFF_FAMILY.contains(&subcommand) {
                    &["--no-ext-diff", "fixture-argument"][..]
                } else {
                    &["fixture-argument"][..]
                }
            );
            assert_eq!(
                cmd.as_std().get_current_dir(),
                Some(Path::new("fixture-cwd"))
            );
        }
        // The old Go syntax sweep retires; this service has one constructor,
        // exercised above, and every operation delegates to its owned runner.
        let own = include_str!("git.rs").split("#[cfg(test)]").next().unwrap();
        assert_eq!(own.matches("Command::new(\"git\")").count(), 1);
        assert!(own.contains("let mut command = command(cwd, args);"));
    }
    #[tokio::test]
    async fn actual_status_suppresses_a_configured_monitor_without_running_it() {
        let root = tempfile::tempdir().unwrap();
        let control = tempfile::tempdir().unwrap();
        let empty = control.path().join("config");
        std::fs::write(&empty, "").unwrap();
        let template = control.path().join("template");
        std::fs::create_dir(&template).unwrap();
        let make = || {
            let mut command = std::process::Command::new("git");
            command
                .current_dir(root.path())
                .env("GIT_CONFIG_NOSYSTEM", "1")
                .env("GIT_CONFIG_GLOBAL", &empty)
                .env("GIT_CONFIG_SYSTEM", &empty)
                .env_remove("GIT_CONFIG")
                .env_remove("GIT_CONFIG_PARAMETERS")
                .env_remove("GIT_CONFIG_COUNT")
                .env_remove("GIT_DIR")
                .env_remove("GIT_WORK_TREE")
                .env_remove("GIT_INDEX_FILE")
                .env_remove("GIT_OBJECT_DIRECTORY")
                .env_remove("GIT_ALTERNATE_OBJECT_DIRECTORIES");
            command
        };
        for argv in [
            vec!["init", "--quiet", "--template", template.to_str().unwrap()],
            vec!["config", "core.attributesFile", empty.to_str().unwrap()],
        ] {
            let output = make().args(argv).output().unwrap();
            assert!(output.status.success());
        }
        std::fs::write(root.path().join("tracked"), "fixture\n").unwrap();
        assert!(make().args(["add", "tracked"]).status().unwrap().success());
        let sentinel = root.path().join("monitor-must-not-exist");
        assert!(
            make()
                .args(["config", "core.fsmonitor", sentinel.to_str().unwrap()])
                .status()
                .unwrap()
                .success()
        );
        // The control attempts a nonexistent executable, never repository code.
        let attempted = make().args(["status", "--porcelain"]).output().unwrap();
        assert!(
            String::from_utf8_lossy(&attempted.stderr).contains("monitor-must-not-exist"),
            "monitor fixture was not exercised: {}",
            String::from_utf8_lossy(&attempted.stderr)
        );
        let guarded = run(root.path(), &args(&["status", "--porcelain"]))
            .await
            .unwrap();
        assert!(guarded.ok);
        assert!(guarded.stdout.contains("tracked"));
        assert!(
            !guarded.stderr.contains("monitor-must-not-exist"),
            "{}",
            guarded.stderr
        );
        assert!(
            super::super::files::call("fs.listEntries", json!({"path":root.path()}), root.path())
                .is_ok()
        );
    }
}
