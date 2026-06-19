//! Git inspection and staging actions for the review pane.
//!
//! Read endpoints (keyed off a `cwd` query param — the renderer passes the
//! active agent's working directory, which it already has in hand):
//!
//!   - `GET /git/status?cwd=<path>`            → branch + changed files
//!   - `GET /git/diff?cwd=<path>&path=&staged=` → raw unified diff text
//!
//! Write endpoints (cwd + args in a JSON body):
//!
//!   - `POST /git/stage`   { cwd, path? }    → `git add` (path, or -A for all)
//!   - `POST /git/unstage` { cwd, path? }    → `git reset HEAD` (path, or all)
//!   - `POST /git/commit`  { cwd, message }  → `git commit -m <message>`
//!   - `POST /git/push`    { cwd }           → `git push`
//!
//! Everything shells out to the `git` binary via `tokio::process::Command`.
//! Before running anything we verify `cwd` is inside a git work tree with
//! `rev-parse --is-inside-work-tree`, so an arbitrary path can't turn these
//! into a generic "run git anywhere" surface.

use axum::{extract::Query, http::StatusCode, response::IntoResponse, Json};
use serde::{Deserialize, Serialize};
use serde_json::json;
use tokio::process::Command;

#[derive(Debug, Deserialize)]
pub struct StatusQuery {
    cwd: String,
}

#[derive(Debug, Deserialize)]
pub struct DiffQuery {
    cwd: String,
    /// Limit the diff to a single path. Omit for the whole work tree.
    path: Option<String>,
    /// When true, diff the staged (index vs HEAD) changes instead of the
    /// unstaged (work tree vs index) changes.
    #[serde(default)]
    staged: bool,
    /// When true, render an untracked file as an all-added diff
    /// (`git diff --no-index /dev/null <path>`). Requires `path`.
    #[serde(default)]
    untracked: bool,
}

#[derive(Debug, Deserialize)]
pub struct NumstatQuery {
    cwd: String,
    /// When true, count the staged (index vs HEAD) changes instead.
    #[serde(default)]
    staged: bool,
}

/// Body for `/git/stage` and `/git/unstage`. `path` limits the action to a
/// single file; omit it to act on the whole work tree.
#[derive(Debug, Deserialize)]
pub struct StageRequest {
    cwd: String,
    path: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct CommitRequest {
    cwd: String,
    message: String,
}

#[derive(Debug, Deserialize)]
pub struct PushRequest {
    cwd: String,
}

/// One changed file as reported by `git status --porcelain`. `staged` and
/// `unstaged` are the porcelain XY codes (e.g. "M", "A", "D", "?", " ").
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct FileStatus {
    pub path: String,
    /// Set only for renames/copies: the original path.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub orig_path: Option<String>,
    pub staged: String,
    pub unstaged: String,
}

#[derive(Debug, Serialize)]
struct StatusResponse {
    branch: Option<String>,
    files: Vec<FileStatus>,
}

/// Run `git` in `cwd` with `args`, returning (success, stdout, stderr).
async fn run_git(cwd: &str, args: &[&str]) -> std::io::Result<(bool, String, String)> {
    let output = Command::new("git").args(args).current_dir(cwd).output().await?;
    Ok((
        output.status.success(),
        String::from_utf8_lossy(&output.stdout).into_owned(),
        String::from_utf8_lossy(&output.stderr).into_owned(),
    ))
}

/// Resolve `cwd` to its git work-tree root, or `None` when `cwd` isn't inside
/// one. Every git command below runs from this root rather than `cwd` itself,
/// because `git status`/`diff --numstat` emit *repo-root-relative* paths while
/// `git diff`/`add` interpret a pathspec relative to the *current directory*.
/// Run from a subdirectory those two conventions disagree, so a root-relative
/// path silently matches nothing and the diff comes back empty. Anchoring at
/// the root keeps both ends speaking the same path language.
async fn work_root(cwd: &str) -> Option<String> {
    match run_git(cwd, &["rev-parse", "--show-toplevel"]).await {
        Ok((true, stdout, _)) => {
            let root = stdout.trim();
            (!root.is_empty()).then(|| root.to_string())
        }
        _ => None,
    }
}

/// Parse `git status --porcelain` (v1) output into structured rows.
///
/// Each line is `XY <path>`, where X is the staged (index) status and Y the
/// unstaged (work tree) status. Renames/copies look like `R  old -> new`.
fn parse_porcelain(stdout: &str) -> Vec<FileStatus> {
    let mut files = Vec::new();
    for line in stdout.lines() {
        // Need at least "XY <path>" — two status chars, a space, then a path.
        if line.len() < 4 {
            continue;
        }
        let staged = line[0..1].to_string();
        let unstaged = line[1..2].to_string();
        let rest = line[3..].trim_end_matches('\r');

        let (orig_path, path) = match rest.split_once(" -> ") {
            Some((orig, new)) => (Some(orig.to_string()), new.to_string()),
            None => (None, rest.to_string()),
        };

        files.push(FileStatus {
            path,
            orig_path,
            staged,
            unstaged,
        });
    }
    files
}

pub async fn get_status(Query(q): Query<StatusQuery>) -> impl IntoResponse {
    let Some(root) = work_root(&q.cwd).await else {
        return (StatusCode::BAD_REQUEST, "cwd is not inside a git work tree").into_response();
    };

    let files = match run_git(&root, &["status", "--porcelain"]).await {
        Ok((true, stdout, _)) => parse_porcelain(&stdout),
        Ok((false, _, stderr)) => {
            tracing::warn!(stderr, "git status failed");
            return (StatusCode::INTERNAL_SERVER_ERROR, "git status failed").into_response();
        }
        Err(err) => {
            tracing::warn!(?err, "spawning git status failed");
            return (StatusCode::INTERNAL_SERVER_ERROR, "could not run git").into_response();
        }
    };

    // Branch name is best-effort: a detached HEAD or fresh repo may not have
    // one, in which case we just report null rather than failing the request.
    let branch = match run_git(&root, &["rev-parse", "--abbrev-ref", "HEAD"]).await {
        Ok((true, stdout, _)) => {
            let name = stdout.trim().to_string();
            (!name.is_empty() && name != "HEAD").then_some(name)
        }
        _ => None,
    };

    Json(StatusResponse { branch, files }).into_response()
}

pub async fn get_diff(Query(q): Query<DiffQuery>) -> impl IntoResponse {
    let Some(root) = work_root(&q.cwd).await else {
        return (StatusCode::BAD_REQUEST, "cwd is not inside a git work tree").into_response();
    };

    if q.untracked {
        let Some(path) = q.path.as_deref() else {
            return (StatusCode::BAD_REQUEST, "untracked diff requires a path").into_response();
        };
        // `--no-index` exits 1 when the files differ — the expected case here —
        // so success is "produced output", not "exit 0". git special-cases the
        // literal "/dev/null" on every platform, including Windows.
        return match run_git(&root, &["diff", "--no-index", "--", "/dev/null", path]).await {
            Ok((ok, stdout, stderr)) => {
                if ok || !stdout.is_empty() {
                    Json(json!({ "diff": stdout })).into_response()
                } else {
                    tracing::warn!(stderr, "git diff --no-index failed");
                    (StatusCode::INTERNAL_SERVER_ERROR, "git diff failed").into_response()
                }
            }
            Err(err) => {
                tracing::warn!(?err, "spawning git diff failed");
                (StatusCode::INTERNAL_SERVER_ERROR, "could not run git").into_response()
            }
        };
    }

    let mut args: Vec<&str> = vec!["diff"];
    if q.staged {
        args.push("--staged");
    }
    // `--` separates pathspecs from revisions so a file named like a flag
    // can't be misread as one.
    if let Some(path) = q.path.as_deref() {
        args.push("--");
        args.push(path);
    }

    match run_git(&root, &args).await {
        Ok((true, stdout, _)) => Json(json!({ "diff": stdout })).into_response(),
        Ok((false, _, stderr)) => {
            tracing::warn!(stderr, "git diff failed");
            (StatusCode::INTERNAL_SERVER_ERROR, "git diff failed").into_response()
        }
        Err(err) => {
            tracing::warn!(?err, "spawning git diff failed");
            (StatusCode::INTERNAL_SERVER_ERROR, "could not run git").into_response()
        }
    }
}

/// One row of `git diff --numstat`: lines added/deleted per file. `None`
/// counts mean a binary file (numstat prints `-` for those).
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct NumstatEntry {
    pub path: String,
    pub added: Option<u64>,
    pub deleted: Option<u64>,
}

/// Resolve a numstat path to the *new* name. Renames appear either as
/// `old => new` or in brace form `prefix/{old => new}/suffix`.
fn parse_numstat_path(raw: &str) -> String {
    if let (Some(open), Some(close)) = (raw.find('{'), raw.find('}')) {
        if open < close {
            if let Some((_, new)) = raw[open + 1..close].split_once(" => ") {
                let joined = format!("{}{}{}", &raw[..open], new, &raw[close + 1..]);
                // An empty side ("{ => sub}") leaves a doubled separator behind.
                return joined.replace("//", "/");
            }
        }
    }
    match raw.split_once(" => ") {
        Some((_, new)) => new.to_string(),
        None => raw.to_string(),
    }
}

fn parse_numstat(stdout: &str) -> Vec<NumstatEntry> {
    stdout
        .lines()
        .filter_map(|line| {
            let mut parts = line.splitn(3, '\t');
            let added = parts.next()?;
            let deleted = parts.next()?;
            let raw_path = parts.next()?;
            Some(NumstatEntry {
                path: parse_numstat_path(raw_path.trim_end_matches('\r')),
                added: added.parse().ok(),
                deleted: deleted.parse().ok(),
            })
        })
        .collect()
}

pub async fn get_numstat(Query(q): Query<NumstatQuery>) -> impl IntoResponse {
    let Some(root) = work_root(&q.cwd).await else {
        return (StatusCode::BAD_REQUEST, "cwd is not inside a git work tree").into_response();
    };

    let mut args: Vec<&str> = vec!["diff", "--numstat"];
    if q.staged {
        args.push("--staged");
    }

    match run_git(&root, &args).await {
        Ok((true, stdout, _)) => Json(json!({ "files": parse_numstat(&stdout) })).into_response(),
        Ok((false, _, stderr)) => {
            tracing::warn!(stderr, "git diff --numstat failed");
            (StatusCode::INTERNAL_SERVER_ERROR, "git diff failed").into_response()
        }
        Err(err) => {
            tracing::warn!(?err, "spawning git diff failed");
            (StatusCode::INTERNAL_SERVER_ERROR, "could not run git").into_response()
        }
    }
}

/// Run a mutating git command in `cwd`, mapping the result to a JSON response.
/// On a non-zero git exit we return 422 with git's stderr so the renderer can
/// surface the real reason (nothing staged, no upstream, merge conflict, …).
async fn git_action(cwd: &str, args: &[&str], what: &str) -> axum::response::Response {
    let Some(root) = work_root(cwd).await else {
        return (StatusCode::BAD_REQUEST, "cwd is not inside a git work tree").into_response();
    };

    match run_git(&root, args).await {
        Ok((true, stdout, _)) => Json(json!({ "ok": true, "output": stdout })).into_response(),
        Ok((false, _, stderr)) => {
            tracing::warn!(action = what, stderr = %stderr, "git action failed");
            (
                StatusCode::UNPROCESSABLE_ENTITY,
                Json(json!({ "ok": false, "error": stderr })),
            )
                .into_response()
        }
        Err(err) => {
            tracing::warn!(action = what, ?err, "spawning git failed");
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(json!({ "ok": false, "error": "could not run git" })),
            )
                .into_response()
        }
    }
}

pub async fn post_stage(Json(req): Json<StageRequest>) -> impl IntoResponse {
    let mut args: Vec<&str> = vec!["add"];
    match req.path.as_deref() {
        // `--` keeps a path that looks like a flag from being parsed as one.
        Some(path) => {
            args.push("--");
            args.push(path);
        }
        None => args.push("-A"),
    }
    git_action(&req.cwd, &args, "stage").await
}

pub async fn post_unstage(Json(req): Json<StageRequest>) -> impl IntoResponse {
    // `reset -q HEAD -- <path>` drops the path from the index without touching
    // the work tree. With no path it unstages everything.
    let mut args: Vec<&str> = vec!["reset", "-q", "HEAD"];
    if let Some(path) = req.path.as_deref() {
        args.push("--");
        args.push(path);
    }
    git_action(&req.cwd, &args, "unstage").await
}

pub async fn post_commit(Json(req): Json<CommitRequest>) -> impl IntoResponse {
    if req.message.trim().is_empty() {
        return (
            StatusCode::BAD_REQUEST,
            Json(json!({ "ok": false, "error": "empty commit message" })),
        )
            .into_response();
    }
    git_action(&req.cwd, &["commit", "-m", req.message.as_str()], "commit").await
}

pub async fn post_push(Json(req): Json<PushRequest>) -> impl IntoResponse {
    git_action(&req.cwd, &["push"], "push").await
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_modified_and_untracked() {
        let out = " M src/main.rs\n?? new.txt\nM  staged.rs\n";
        let files = parse_porcelain(out);
        assert_eq!(
            files,
            vec![
                FileStatus {
                    path: "src/main.rs".into(),
                    orig_path: None,
                    staged: " ".into(),
                    unstaged: "M".into(),
                },
                FileStatus {
                    path: "new.txt".into(),
                    orig_path: None,
                    staged: "?".into(),
                    unstaged: "?".into(),
                },
                FileStatus {
                    path: "staged.rs".into(),
                    orig_path: None,
                    staged: "M".into(),
                    unstaged: " ".into(),
                },
            ]
        );
    }

    #[test]
    fn parses_rename() {
        let files = parse_porcelain("R  old/name.rs -> new/name.rs\n");
        assert_eq!(files.len(), 1);
        assert_eq!(files[0].path, "new/name.rs");
        assert_eq!(files[0].orig_path.as_deref(), Some("old/name.rs"));
        assert_eq!(files[0].staged, "R");
    }

    #[test]
    fn skips_blank_and_short_lines() {
        assert!(parse_porcelain("\n\nx\n").is_empty());
    }

    #[test]
    fn parses_numstat_counts_and_binary() {
        let out = "12\t3\tsrc/main.rs\n-\t-\tlogo.png\n";
        assert_eq!(
            parse_numstat(out),
            vec![
                NumstatEntry {
                    path: "src/main.rs".into(),
                    added: Some(12),
                    deleted: Some(3),
                },
                NumstatEntry {
                    path: "logo.png".into(),
                    added: None,
                    deleted: None,
                },
            ]
        );
    }

    #[test]
    fn resolves_numstat_rename_paths() {
        assert_eq!(parse_numstat_path("old.rs => new.rs"), "new.rs");
        assert_eq!(parse_numstat_path("src/{a.rs => b.rs}"), "src/b.rs");
        assert_eq!(parse_numstat_path("src/{ => sub}/mod.rs"), "src/sub/mod.rs");
        assert_eq!(parse_numstat_path("src/{old => }/mod.rs"), "src/mod.rs");
        assert_eq!(parse_numstat_path("plain/path.rs"), "plain/path.rs");
    }
}
