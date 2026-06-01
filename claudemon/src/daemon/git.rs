//! Read-only git inspection for the review pane.
//!
//! Two endpoints, both keyed off a `cwd` query param (the renderer passes the
//! active agent's working directory, which it already has in hand):
//!
//!   - `GET /git/status?cwd=<path>`            → branch + changed files
//!   - `GET /git/diff?cwd=<path>&path=&staged=` → raw unified diff text
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

/// True only when `cwd` exists and sits inside a git work tree.
async fn is_work_tree(cwd: &str) -> bool {
    match run_git(cwd, &["rev-parse", "--is-inside-work-tree"]).await {
        Ok((true, stdout, _)) => stdout.trim() == "true",
        _ => false,
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
    if !is_work_tree(&q.cwd).await {
        return (StatusCode::BAD_REQUEST, "cwd is not inside a git work tree").into_response();
    }

    let files = match run_git(&q.cwd, &["status", "--porcelain"]).await {
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
    let branch = match run_git(&q.cwd, &["rev-parse", "--abbrev-ref", "HEAD"]).await {
        Ok((true, stdout, _)) => {
            let name = stdout.trim().to_string();
            (!name.is_empty() && name != "HEAD").then_some(name)
        }
        _ => None,
    };

    Json(StatusResponse { branch, files }).into_response()
}

pub async fn get_diff(Query(q): Query<DiffQuery>) -> impl IntoResponse {
    if !is_work_tree(&q.cwd).await {
        return (StatusCode::BAD_REQUEST, "cwd is not inside a git work tree").into_response();
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

    match run_git(&q.cwd, &args).await {
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
}
