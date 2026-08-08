//! Free async helpers (spawned task bodies) and path-completion utilities.
//!
//! These are free functions rather than methods so they can be shared between
//! `impl App` methods and the tokio tasks those methods spawn.

use std::path::PathBuf;
use std::time::Duration;

use tokio::sync::mpsc::UnboundedSender;

use crate::claudemon::Claudemon;
use crate::profiles;
use crate::types::{search_lines, turns_from_conversation};

use super::{AppMsg, SpawnForm};

// ── free async helpers (shared by methods and spawned tasks) ────────────────

pub(super) async fn fetch_agents(cm: &Claudemon, tx: &UnboundedSender<AppMsg>) {
    // claudemon now returns usage { context_tokens, context_limit, cost_usd,
    // model } directly in the GET /sessions payload, so there is no need to
    // fetch each session's transcript just to compute cost. The Agent struct
    // deserializes it automatically.
    let Ok(list) = cm.list().await else { return };
    let _ = tx.send(AppMsg::Agents(list));
}

/// Fetch a session's conversation snapshot, for the fold to adopt.
///
/// Only needed when a chat is opened or when a delta couldn't be sequenced —
/// the steady state is the delta feed, not this. The fold does the folding, so
/// the raw payload travels and the transport-specific coalescing is decided
/// there (the fold knows its session's transport).
pub(super) async fn fetch_transcript(
    cm: &Claudemon,
    tx: &UnboundedSender<AppMsg>,
    session_id: String,
) {
    if let Ok(v) = cm.conversation(&session_id).await {
        let _ = tx.send(AppMsg::Transcript {
            session_id,
            snapshot: Box::new(v),
        });
    }
}

/// Fetch a session's conversation and emit its searchable lines for the content
/// index. Always sends a (possibly empty) result so the modal's `pending`
/// counter still decrements when a fetch fails.
pub(super) async fn fetch_search_index(
    cm: &Claudemon,
    tx: &UnboundedSender<AppMsg>,
    session_id: String,
    name: String,
    transport: String,
) {
    let lines = match cm.conversation(&session_id).await {
        Ok(v) => search_lines(&turns_from_conversation(&v, &transport)),
        Err(_) => Vec::new(),
    };
    let _ = tx.send(AppMsg::SearchEntries {
        session_id,
        name,
        lines,
    });
}

pub(super) async fn fetch_git_status(cm: &Claudemon, tx: &UnboundedSender<AppMsg>, cwd: String) {
    match cm.git_status(&cwd).await {
        Ok((branch, files)) => {
            let _ = tx.send(AppMsg::GitStatus { cwd, branch, files });
        }
        Err(e) => {
            // Surface in the review pane (e.g. "not inside a git work tree")
            // instead of a fleeting toast, so an empty list isn't mistaken for
            // a clean repo.
            let _ = tx.send(AppMsg::GitError {
                cwd,
                message: e.to_string(),
            });
        }
    }
}

pub(super) async fn fetch_git_summary(cm: &Claudemon, tx: &UnboundedSender<AppMsg>, cwd: String) {
    if let Ok((branch, files)) = cm.git_status(&cwd).await {
        let _ = tx.send(AppMsg::GitSummary {
            cwd,
            branch,
            changed: files.len(),
        });
    }
}

pub(super) async fn fetch_git_diff(
    cm: &Claudemon,
    tx: &UnboundedSender<AppMsg>,
    cwd: String,
    path: String,
    staged: bool,
    untracked: bool,
) {
    if let Ok(diff) = cm.git_diff(&cwd, &path, staged, untracked).await {
        let _ = tx.send(AppMsg::GitDiff {
            cwd,
            path,
            staged,
            diff,
        });
    }
}

/// Wrap text in bracketed-paste markers so a multi-line prompt is inserted into
/// Claude's input as one paste (newlines stay newlines instead of submitting).
///
/// The body is neutralized first. A bracketed paste is only inert if its own
/// content cannot forge the end marker: an embedded ESC[201~ (which a library /
/// skill / agent body can carry verbatim — the text is raw file content after
/// frontmatter) would close paste mode early, and the bytes after it — a
/// trailing CR submits — land as live keystrokes against the focused PTY. That
/// turns an "insert for review" into arbitrary command execution against a shell
/// tab, and defeats the no-submit review gate against a Claude PTY. We replace
/// every ESC (\x1b) with its visible glyph U+241B before wrapping — the same
/// neutralization xterm's bracketTextForPaste applies (the desktop twin) — so
/// the frame we emit is the only paste boundary and no injected control sequence
/// (paste marker or otherwise) survives.
pub(super) fn bracketed_paste(text: &str) -> Vec<u8> {
    let sanitized = text.replace('\x1b', "\u{241b}");
    let mut v = Vec::with_capacity(sanitized.len() + 12);
    v.extend_from_slice(b"\x1b[200~");
    v.extend_from_slice(sanitized.as_bytes());
    v.extend_from_slice(b"\x1b[201~");
    v
}

/// Seed a prompt into a freshly-spawned agent: wait until it reaches its input
/// prompt (claudemon reports mode `input`), then paste — without submitting, so
/// the user reviews and presses enter.
pub(super) async fn seed_prompt(
    cm: &Claudemon,
    tx: &UnboundedSender<AppMsg>,
    sid: &str,
    prompt: &str,
) {
    // The agent has to reach its prompt before a paste means anything. If it
    // never does, the paste goes nowhere — say so rather than claiming success.
    let mut ready = false;
    for _ in 0..40 {
        if cm.session_mode(sid).await.as_deref() == Some("input") {
            ready = true;
            break;
        }
        tokio::time::sleep(Duration::from_millis(400)).await;
    }
    let toast = if !ready {
        "Agent never reached its prompt — the text was not seeded".to_string()
    } else if let Err(err) = cm.input_bytes(sid, &bracketed_paste(prompt)).await {
        format!("Could not seed the prompt: {err}")
    } else {
        "Prompt seeded — open the agent and press enter".to_string()
    };
    let _ = tx.send(AppMsg::Toast(toast));
    fetch_agents(cm, tx).await;
}

// ── path completion ─────────────────────────────────────────────────────────

/// Shell-style directory completion for the spawn modal's cwd field. Completes
/// the trailing component to the longest common prefix of matching directories;
/// fills a single match fully (with a trailing `/`), or records the candidates
/// for display when ambiguous. Only the newly-resolved characters are appended,
/// so the user's literal text (including a leading `~`) is preserved.
pub(super) fn complete_path(form: &mut SpawnForm) {
    let input = form.cwd.clone();
    let (dir_part, partial) = match input.rfind('/') {
        Some(i) => (input[..=i].to_string(), input[i + 1..].to_string()),
        None => (String::new(), input.clone()),
    };

    let real_dir: PathBuf = if dir_part.is_empty() {
        std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."))
    } else {
        PathBuf::from(profiles::expand_tilde(&dir_part))
    };

    // Hidden entries only surface when the user has started typing a dot,
    // matching how shells behave.
    let want_hidden = partial.starts_with('.');
    let mut names: Vec<String> = Vec::new();
    if let Ok(entries) = std::fs::read_dir(&real_dir) {
        for entry in entries.flatten() {
            let name = entry.file_name().to_string_lossy().to_string();
            if !name.starts_with(&partial) || (!want_hidden && name.starts_with('.')) {
                continue;
            }
            if entry.path().is_dir() {
                names.push(name);
            }
        }
    }
    names.sort();

    if names.is_empty() {
        return;
    }
    let prefix = longest_common_prefix(&names);
    if prefix.len() > partial.len() {
        form.cwd.push_str(&prefix[partial.len()..]);
    }
    if names.len() == 1 {
        form.cwd.push('/');
        form.completions.clear();
    } else {
        // Multiple matches: leave them on screen so the user can keep typing.
        form.completions = names;
    }
}

/// The longest common (character-wise) prefix shared by every string.
pub(super) fn longest_common_prefix(names: &[String]) -> String {
    let mut iter = names.iter();
    let Some(first) = iter.next() else {
        return String::new();
    };
    let mut prefix = first.clone();
    for s in iter {
        let common: String = prefix
            .chars()
            .zip(s.chars())
            .take_while(|(a, b)| a == b)
            .map(|(a, _)| a)
            .collect();
        prefix = common;
        if prefix.is_empty() {
            break;
        }
    }
    prefix
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    fn form(cwd: &str) -> SpawnForm {
        SpawnForm {
            cwd: cwd.into(),
            profile_idx: 0,
            provider_idx: 0,
            completions: Vec::new(),
            initial_prompt: None,
        }
    }

    #[test]
    fn bracketed_paste_neutralizes_embedded_end_marker() {
        // A library/skill body that forges its own ESC[201~ end marker (followed
        // by a CR + command) must not break out of paste mode: the only paste
        // boundary is the frame we add, so no embedded marker may survive in the
        // body and no live ESC may reach the PTY.
        let body = "hello\x1b[201~\rrm -rf ~\r";
        let out = bracketed_paste(body);
        // Exactly one opening and one closing marker — ours, at the ends.
        assert!(out.starts_with(b"\x1b[200~"), "must open with a paste marker");
        assert!(out.ends_with(b"\x1b[201~"), "must close with a paste marker");
        let inner = &out[6..out.len() - 6];
        // No raw ESC survives inside the body, so no forged paste marker (which
        // must begin with ESC) can exist there either.
        assert!(
            !inner.contains(&0x1b),
            "raw ESC survived in body, a paste marker could be forged: {:?}",
            String::from_utf8_lossy(inner)
        );
        // The neutralized ESC is present as the visible glyph, so the CR after it
        // can no longer submit a live command.
        assert!(
            String::from_utf8_lossy(inner).contains('\u{241b}'),
            "ESC should be neutralized to U+241B, got {:?}",
            String::from_utf8_lossy(inner)
        );
    }

    #[test]
    fn lcp() {
        assert_eq!(
            longest_common_prefix(&["project-a".into(), "project-b".into()]),
            "project-"
        );
        assert_eq!(longest_common_prefix(&["abc".into()]), "abc");
        assert_eq!(longest_common_prefix(&["a".into(), "b".into()]), "");
        assert_eq!(longest_common_prefix(&[]), "");
    }

    #[test]
    fn completes_unique_directory_with_trailing_slash() {
        let base = std::env::temp_dir().join("wkstui_unique");
        let _ = fs::remove_dir_all(&base);
        fs::create_dir_all(base.join("alpha")).unwrap();

        let mut f = form(&format!("{}/al", base.display()));
        complete_path(&mut f);

        assert_eq!(f.cwd, format!("{}/alpha/", base.display()));
        assert!(f.completions.is_empty());
        let _ = fs::remove_dir_all(&base);
    }

    #[test]
    fn completes_common_prefix_and_records_candidates() {
        let base = std::env::temp_dir().join("wkstui_multi");
        let _ = fs::remove_dir_all(&base);
        fs::create_dir_all(base.join("proj-a")).unwrap();
        fs::create_dir_all(base.join("proj-b")).unwrap();
        fs::create_dir_all(base.join("other")).unwrap();

        let mut f = form(&format!("{}/pr", base.display()));
        complete_path(&mut f);

        assert_eq!(f.cwd, format!("{}/proj-", base.display()));
        assert_eq!(
            f.completions,
            vec!["proj-a".to_string(), "proj-b".to_string()]
        );
        let _ = fs::remove_dir_all(&base);
    }

    #[test]
    fn hides_dotfiles_unless_dot_typed() {
        let base = std::env::temp_dir().join("wkstui_hidden");
        let _ = fs::remove_dir_all(&base);
        fs::create_dir_all(base.join(".secret")).unwrap();
        fs::create_dir_all(base.join("visible")).unwrap();

        // No leading dot in the partial → hidden dir excluded, "visible" completes.
        let mut f = form(&format!("{}/", base.display()));
        complete_path(&mut f);
        assert_eq!(f.cwd, format!("{}/visible/", base.display()));

        // Leading dot → the hidden dir is the only match.
        let mut f = form(&format!("{}/.", base.display()));
        complete_path(&mut f);
        assert_eq!(f.cwd, format!("{}/.secret/", base.display()));
        let _ = fs::remove_dir_all(&base);
    }
}
