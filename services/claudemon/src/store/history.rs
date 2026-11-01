//! Permanent session history (docs/session-roster.md §5).
//!
//! Every session the daemon has seen working keeps its `sessions` row for good.
//! Rows used to be born only from Claude hook events, so stream and managed
//! (Codex, OpenCode, Pi, Copilot) sessions never reached SQLite at all. The
//! history recorder (`daemon::history`) now writes those rows from the
//! session-update stream through [`Db::record_history`].
//!
//! The History list reads SQLite in pages ([`Db::history_page`]), never the
//! in-memory map, which still evicts stopped sessions after seven days.
//! [`Db::forget_history`] (an explicit delete from History) is the only path
//! that removes a session row.
//!
//! Transcript copies: the provider owns its transcript and may delete it
//! (Claude Code's own cleanup). The daemon keeps a copy under
//! `<data>/transcripts/<provider>/<id>.<ext>` and records it in the
//! `transcript_copy` column as a [`CopyRecord`].

use std::path::{Path, PathBuf};

use anyhow::{bail, Result};
use rusqlite::{params, Connection, OptionalExtension};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use time::OffsetDateTime;

use super::Db;
use crate::session::{
    windows::{restore_persisted_model_selection, PersistedModelSelection},
    SessionMode, SessionState,
};

/// Default and maximum History page sizes.
pub const DEFAULT_LIMIT: usize = 50;
pub const MAX_LIMIT: usize = 200;
/// Longest title or first prompt kept on a row. These are search and display
/// hints; the transcript stays the source of truth.
const TEXT_MAX_CHARS: usize = 500;
/// Transcripts larger than this are not copied. The original is left alone
/// and the skip is logged.
pub const MAX_COPY_BYTES: u64 = 1024 * 1024 * 1024;

/// What the recorder persists about one session, taken from its live state.
#[derive(Debug, Clone)]
pub struct HistoryFacts {
    pub id: String,
    pub cwd: String,
    pub provider: String,
    pub transport: &'static str,
    pub title: Option<String>,
    pub transcript_path: Option<String>,
    pub config_root: Option<String>,
    pub requested: Option<PersistedModelSelection>,
    pub tool_calls: u64,
    pub user_prompts: u64,
    pub started_at: i64,
    pub updated_at: i64,
    pub stopped: bool,
}

impl HistoryFacts {
    pub fn from_state(state: &SessionState) -> Self {
        Self {
            id: state.session_id.clone(),
            cwd: state.cwd.clone().unwrap_or_default(),
            provider: state.provider.clone(),
            transport: match state.transport {
                crate::session::Transport::Pty => "pty",
                crate::session::Transport::Stream => "stream",
            },
            title: state
                .thread_name
                .as_deref()
                .map(str::trim)
                .filter(|t| !t.is_empty())
                .map(clip),
            transcript_path: state.transcript_path.clone(),
            config_root: state.config_root.clone(),
            requested: state
                .requested_selection
                .clone()
                .map(PersistedModelSelection::from_selection),
            tool_calls: state.tool_calls,
            user_prompts: state.user_prompts,
            started_at: state.started_at.unix_timestamp(),
            updated_at: state.updated_at.unix_timestamp(),
            stopped: state.mode == SessionMode::Stopped,
        }
    }
}

/// Whether the recorder may CREATE a history row for this state, as opposed
/// to only updating one that exists. A terminal shell is registered exactly
/// like a Claude PTY spawn and never leaves `Unknown` until it stops, so a row
/// is created only for a session that has been seen in an agent mode.
pub fn agent_mode(mode: SessionMode) -> bool {
    !matches!(mode, SessionMode::Unknown | SessionMode::Stopped)
}

/// `transcript_copy` column contents.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CopyRecord {
    /// Where the copy lives (under [`Db::transcript_copies_dir`]).
    pub copy: String,
    /// The provider's own path, where a resume expects the file.
    pub original: String,
    pub size: u64,
    pub mtime_ms: i64,
    pub copied_at: i64,
}

/// One History query.
#[derive(Debug, Clone, Default)]
pub struct HistoryQuery {
    pub before: Option<String>,
    pub limit: Option<usize>,
    pub q: Option<String>,
    pub cwd: Option<String>,
}

/// One History page: rows newest first and, when more remain, the cursor for
/// the next page.
#[derive(Debug, Clone, Serialize)]
pub struct HistoryPage {
    pub sessions: Vec<Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub next: Option<String>,
}

/// One history row as stored.
#[derive(Debug, Clone)]
pub struct HistoryRow {
    pub id: String,
    pub cwd: String,
    pub name: String,
    pub provider: Option<String>,
    pub transport: Option<String>,
    pub title: Option<String>,
    pub model: Option<String>,
    pub requested_model: Option<String>,
    pub requested_identity: Option<String>,
    pub requested_window: Option<i64>,
    pub transcript_path: Option<String>,
    pub config_root: Option<String>,
    pub created_at: i64,
    pub last_event_at: i64,
    pub ended_at: Option<i64>,
    pub tool_calls: u64,
    pub user_prompts: u64,
    pub first_prompt: Option<String>,
    pub copy: Option<CopyRecord>,
}

impl HistoryRow {
    /// Hook-born rows predate the provider column; only Claude fires hooks.
    pub fn provider(&self) -> &str {
        self.provider.as_deref().unwrap_or("claude")
    }

    pub fn transport(&self) -> &str {
        self.transport.as_deref().unwrap_or("pty")
    }

    /// The JSON the API serves: snake_case like every other claudemon row, a
    /// stopped `mode` (history rows carry no liveness; callers merge live state
    /// themselves), RFC 3339 `updated_at`/`started_at` like `/sessions`, and
    /// epoch-second `created_at`/`last_event_at`/`ended_at`.
    pub fn to_json(&self) -> Value {
        let selection = restore_persisted_model_selection(
            self.requested_identity.as_deref(),
            self.requested_window,
            self.requested_model.as_deref(),
        );
        json!({
            "session_id": self.id,
            "cwd": (!self.cwd.is_empty()).then_some(&self.cwd),
            "name": self.name,
            "mode": "stopped",
            "provider": self.provider(),
            "transport": self.transport(),
            "title": self.title,
            "model": self.model,
            "requested_model": self.requested_model,
            "requested_selection": selection,
            "transcript_path": self.transcript_path,
            "config_root": self.config_root,
            "started_at": rfc3339(self.created_at),
            "updated_at": rfc3339(self.last_event_at),
            "created_at": self.created_at,
            "last_event_at": self.last_event_at,
            "ended_at": self.ended_at,
            "tool_calls": self.tool_calls,
            "user_prompts": self.user_prompts,
            "first_prompt": self.first_prompt,
            "transcript_copy": self.copy.as_ref().is_some_and(|c| Path::new(&c.copy).is_file()),
            "history": true,
        })
    }
}

fn rfc3339(unix: i64) -> Option<String> {
    OffsetDateTime::from_unix_timestamp(unix)
        .ok()?
        .format(&time::format_description::well_known::Rfc3339)
        .ok()
}

fn clip(text: &str) -> String {
    text.chars().take(TEXT_MAX_CHARS).collect()
}

/// Truncate a prompt for the `first_prompt` column.
pub(crate) fn clip_prompt(text: &str) -> Option<String> {
    let text = text.trim();
    (!text.is_empty()).then(|| clip(text))
}

/// `"<last_event_at>:<id>"`. Opaque to callers; `:` never occurs in a valid id.
fn cursor(ts: i64, id: &str) -> String {
    format!("{ts}:{id}")
}

fn parse_cursor(raw: &str) -> Result<(i64, String)> {
    let Some((ts, id)) = raw.split_once(':') else {
        bail!("invalid history cursor");
    };
    let ts: i64 = ts
        .parse()
        .map_err(|_| anyhow::anyhow!("invalid history cursor"))?;
    if !crate::daemon::api::valid_session_id(id) {
        bail!("invalid history cursor");
    }
    Ok((ts, id.to_owned()))
}

const ROW_COLUMNS: &str = "s.id, s.cwd, s.name, s.provider, s.transport, s.title, s.model,
    s.requested_model, s.requested_model_identity, s.requested_context_window,
    s.transcript_path, s.config_root, s.created_at, s.last_event_at, s.ended_at,
    s.tool_call_count,
    MAX(COALESCE(s.user_prompt_count, 0),
        (SELECT COUNT(*) FROM events e
          WHERE e.session_id = s.id AND e.event_type = 'UserPromptSubmit')),
    s.first_prompt, s.transcript_copy";

fn row(r: &rusqlite::Row<'_>) -> rusqlite::Result<HistoryRow> {
    let text = |i: usize| -> rusqlite::Result<Option<String>> {
        Ok(match r.get_ref(i)? {
            rusqlite::types::ValueRef::Text(v) => std::str::from_utf8(v)
                .ok()
                .filter(|s| !s.is_empty())
                .map(str::to_owned),
            _ => None,
        })
    };
    let int = |i: usize| -> rusqlite::Result<Option<i64>> {
        Ok(match r.get_ref(i)? {
            rusqlite::types::ValueRef::Integer(v) => Some(v),
            _ => None,
        })
    };
    Ok(HistoryRow {
        id: r.get(0)?,
        cwd: text(1)?.unwrap_or_default(),
        name: text(2)?.unwrap_or_default(),
        provider: text(3)?,
        transport: text(4)?,
        title: text(5)?,
        model: text(6)?,
        requested_model: text(7)?,
        requested_identity: text(8)?,
        requested_window: int(9)?,
        transcript_path: text(10)?,
        // `Some("")` is the default account, a known answer.
        config_root: match r.get_ref(11)? {
            rusqlite::types::ValueRef::Text(v) => std::str::from_utf8(v).ok().map(str::to_owned),
            _ => None,
        },
        created_at: int(12)?.unwrap_or(0),
        last_event_at: int(13)?.unwrap_or(0),
        ended_at: int(14)?,
        tool_calls: int(15)?.unwrap_or(0).max(0) as u64,
        user_prompts: int(16)?.unwrap_or(0).max(0) as u64,
        first_prompt: text(17)?,
        copy: text(18)?.and_then(|raw| serde_json::from_str(&raw).ok()),
    })
}

/// Shared `SET` list for a recorder write. `?1` is the id; the remaining
/// placeholders follow the order in [`Db::record_history`].
const RECORD_SET: &str = "
    last_event_at = MAX(sessions.last_event_at, ?11),
    cwd = CASE WHEN sessions.cwd = '' THEN ?2 ELSE sessions.cwd END,
    provider = ?3,
    transport = ?4,
    title = COALESCE(?5, sessions.title),
    transcript_path = COALESCE(?6, sessions.transcript_path),
    config_root = COALESCE(sessions.config_root, ?7),
    requested_model = CASE WHEN ?9 IS NOT NULL THEN ?8 ELSE sessions.requested_model END,
    requested_model_identity = CASE WHEN ?9 IS NOT NULL THEN ?9
        ELSE sessions.requested_model_identity END,
    requested_context_window = CASE WHEN ?9 IS NOT NULL THEN ?10
        ELSE sessions.requested_context_window END,
    tool_call_count = MAX(COALESCE(sessions.tool_call_count, 0), ?12),
    user_prompt_count = MAX(COALESCE(sessions.user_prompt_count, 0), ?13),
    ended_at = CASE WHEN ?14 THEN COALESCE(sessions.ended_at, ?11) ELSE NULL END";

impl Db {
    /// Directory holding transcript copies: `<db dir>/transcripts`. Asking
    /// for it also lets transcript readers open files inside it
    /// (`transcript::allow_copy_root`).
    pub fn transcript_copies_dir(&self) -> PathBuf {
        let dir = self
            .path
            .parent()
            .map(Path::to_path_buf)
            .unwrap_or_default()
            .join("transcripts");
        crate::session::transcript::allow_copy_root(dir.clone());
        dir
    }

    /// Persist recorder facts in one transaction. `create` says, per fact,
    /// whether a missing row may be inserted (see [`agent_mode`]); otherwise
    /// only an existing row is updated.
    pub fn record_history(&self, facts: &[(HistoryFacts, bool)]) -> Result<()> {
        let mut guard = self.conn.lock().expect("db mutex poisoned");
        let tx = guard.transaction()?;
        for (f, create) in facts {
            record_one(&tx, f, *create)?;
        }
        tx.commit()?;
        Ok(())
    }

    /// One History page, newest first by last activity.
    pub fn history_page(&self, query: &HistoryQuery) -> Result<HistoryPage> {
        let limit = query.limit.unwrap_or(DEFAULT_LIMIT).clamp(1, MAX_LIMIT);
        let before = query.before.as_deref().map(parse_cursor).transpose()?;
        let needle = query
            .q
            .as_deref()
            .map(str::trim)
            .filter(|q| !q.is_empty())
            .map(str::to_lowercase);
        let cwd = query.cwd.as_deref().filter(|c| !c.is_empty());
        let guard = self.conn.lock().expect("db mutex poisoned");
        let sql = format!(
            "SELECT {ROW_COLUMNS} FROM sessions s
              WHERE (?1 IS NULL OR s.last_event_at < ?1
                     OR (s.last_event_at = ?1 AND s.id < ?2))
                AND (?3 IS NULL OR s.cwd = ?3)
                AND (?4 IS NULL OR instr(lower(
                        s.id || ' ' || s.cwd || ' ' || COALESCE(s.name, '') || ' '
                        || COALESCE(s.title, '') || ' ' || COALESCE(s.first_prompt, '')),
                     ?4) > 0)
              ORDER BY s.last_event_at DESC, s.id DESC
              LIMIT ?5"
        );
        let mut stmt = guard.prepare(&sql)?;
        let rows = stmt
            .query_map(
                params![
                    before.as_ref().map(|(ts, _)| *ts),
                    before.as_ref().map(|(_, id)| id.as_str()),
                    cwd,
                    needle,
                    (limit + 1) as i64
                ],
                row,
            )?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        let more = rows.len() > limit;
        let rows = &rows[..rows.len().min(limit)];
        let next = more
            .then(|| rows.last().map(|r| cursor(r.last_event_at, &r.id)))
            .flatten();
        Ok(HistoryPage {
            sessions: rows.iter().map(HistoryRow::to_json).collect(),
            next,
        })
    }

    /// One session's history row, if it has one.
    pub fn history_row(&self, id: &str) -> Result<Option<HistoryRow>> {
        let guard = self.conn.lock().expect("db mutex poisoned");
        let sql = format!("SELECT {ROW_COLUMNS} FROM sessions s WHERE s.id = ?1");
        Ok(guard.query_row(&sql, [id], row).optional()?)
    }

    /// Delete a session from History: its events and its row, in one
    /// transaction. Returns the deleted row so the caller can remove the
    /// transcript copy. The only way a session row is ever deleted.
    pub fn forget_history(&self, id: &str) -> Result<Option<HistoryRow>> {
        let mut guard = self.conn.lock().expect("db mutex poisoned");
        let tx = guard.transaction()?;
        let sql = format!("SELECT {ROW_COLUMNS} FROM sessions s WHERE s.id = ?1");
        let found = tx.query_row(&sql, [id], row).optional()?;
        if found.is_some() {
            tx.execute("DELETE FROM events WHERE session_id = ?1", [id])?;
            tx.execute("DELETE FROM sessions WHERE id = ?1", [id])?;
        }
        tx.commit()?;
        Ok(found)
    }

    /// Record (or replace) a session's transcript copy.
    pub fn set_transcript_copy(&self, id: &str, record: &CopyRecord) -> Result<()> {
        let guard = self.conn.lock().expect("db mutex poisoned");
        guard.execute(
            "UPDATE sessions SET transcript_copy = ?2 WHERE id = ?1",
            params![id, serde_json::to_string(record)?],
        )?;
        Ok(())
    }

    /// Remember a transcript path learned outside hooks (stream sessions
    /// discover theirs), without replacing one a hook already recorded.
    pub fn note_transcript_path(&self, id: &str, path: &str) -> Result<()> {
        let guard = self.conn.lock().expect("db mutex poisoned");
        guard.execute(
            "UPDATE sessions SET transcript_path = ?2 WHERE id = ?1 AND transcript_path IS NULL",
            params![id, path],
        )?;
        Ok(())
    }

    /// Size-bounding for long-retained history. Session rows are never
    /// deleted. For sessions idle past the archive window and outside the
    /// newest `keep`, every event except `UserPromptSubmit` is deleted. The
    /// event log's only reader is the prompt count (hydration's "was this ever
    /// used" signal and History's `user_prompts`), and prompts are also what a
    /// History search matches. Tool events are the bulk of the table (their
    /// payloads carry tool inputs) and nothing reads them back.
    pub fn trim_archived_events(&self, keep: usize) -> Result<usize> {
        let cutoff = OffsetDateTime::now_utc().unix_timestamp()
            - crate::session::state::ARCHIVE_AFTER_SECONDS;
        self.trim_archived_events_before(keep, cutoff)
    }

    pub(crate) fn trim_archived_events_before(&self, keep: usize, cutoff: i64) -> Result<usize> {
        let guard = self.conn.lock().expect("db mutex poisoned");
        let trimmed = guard.execute(
            "DELETE FROM events
              WHERE event_type <> 'UserPromptSubmit'
                AND session_id IN (
                  SELECT id FROM sessions
                   WHERE last_event_at < ?1 AND id NOT IN
                     (SELECT id FROM sessions ORDER BY last_event_at DESC, id DESC LIMIT ?2))",
            params![cutoff, keep as i64],
        )?;
        Ok(trimmed)
    }
}

fn record_one(tx: &Connection, f: &HistoryFacts, create: bool) -> Result<()> {
    let requested_model = f.requested.as_ref().map(|s| s.legacy_model.as_str());
    let identity = f.requested.as_ref().map(|s| s.selection.model.as_str());
    let window = f
        .requested
        .as_ref()
        .and_then(|s| s.selection.context_window)
        .map(|w| w as i64);
    let values = params![
        f.id,
        f.cwd,
        f.provider,
        f.transport,
        f.title,
        f.transcript_path,
        f.config_root.as_deref().map(str::trim),
        requested_model,
        identity,
        window,
        f.updated_at,
        f.tool_calls as i64,
        f.user_prompts as i64,
        f.stopped,
        f.started_at,
        super::derive_name_from_cwd(&f.cwd),
    ];
    if create {
        tx.execute(
            &format!(
                "INSERT INTO sessions (
                    id, name, project, cwd, worktree_path, state, created_at, last_event_at,
                    tool_call_count, provider, transport, title, transcript_path, config_root,
                    requested_model, requested_model_identity, requested_context_window,
                    user_prompt_count, ended_at
                 ) VALUES (?1, ?16, ?16, ?2, ?2, 'working', ?15, ?11, ?12, ?3, ?4, ?5, ?6, ?7,
                           ?8, ?9, ?10, ?13, CASE WHEN ?14 THEN ?11 ELSE NULL END)
                 ON CONFLICT(id) DO UPDATE SET {RECORD_SET}"
            ),
            values,
        )?;
    } else {
        tx.execute(
            &format!(
                "UPDATE sessions SET {} WHERE id = ?1 AND (?15 IS NOT NULL OR ?16 IS NOT NULL)",
                RECORD_SET.replace("sessions.", "")
            ),
            values,
        )?;
    }
    Ok(())
}

// --- transcript copies ------------------------------------------------------

/// Path-safe single segment (`provider`, file extension).
fn segment(raw: &str, fallback: &str) -> String {
    let clean: String = raw
        .chars()
        .filter(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_'))
        .take(32)
        .collect();
    if clean.is_empty() {
        fallback.to_owned()
    } else {
        clean
    }
}

fn mtime_ms(meta: &std::fs::Metadata) -> i64 {
    meta.modified()
        .ok()
        .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0)
}

/// Copy `original` into the copy store if it changed since the recorded copy.
/// Blocking; run it on the blocking pool. Returns the new record when a copy
/// was written, `None` when nothing changed (or the session has no row).
pub fn copy_transcript(
    db: &Db,
    id: &str,
    provider: &str,
    original: &Path,
) -> Result<Option<CopyRecord>> {
    if !crate::daemon::api::valid_session_id(id) {
        bail!("invalid session id");
    }
    let Some(existing) = db.history_row(id)? else {
        return Ok(None);
    };
    let meta = std::fs::metadata(original)?;
    if !meta.is_file() {
        bail!("transcript is not a file");
    }
    if meta.len() > MAX_COPY_BYTES {
        tracing::warn!(session = %id, size = meta.len(), "transcript too large to copy");
        return Ok(None);
    }
    let mtime = mtime_ms(&meta);
    let original_text = original.to_string_lossy().into_owned();
    if let Some(prior) = &existing.copy {
        if prior.size == meta.len()
            && prior.mtime_ms == mtime
            && prior.original == original_text
            && Path::new(&prior.copy).is_file()
        {
            return Ok(None);
        }
    }
    let ext = original
        .extension()
        .and_then(|e| e.to_str())
        .map(|e| segment(e, "jsonl"))
        .unwrap_or_else(|| "jsonl".into());
    let dir = db
        .transcript_copies_dir()
        .join(segment(provider, "unknown"));
    std::fs::create_dir_all(&dir)?;
    let dest = dir.join(format!("{id}.{ext}"));
    let tmp = dir.join(format!(".{id}.{ext}.tmp"));
    let copied = std::fs::copy(original, &tmp);
    if let Err(err) = copied {
        let _ = std::fs::remove_file(&tmp);
        return Err(err.into());
    }
    std::fs::rename(&tmp, &dest)?;
    let record = CopyRecord {
        copy: dest.to_string_lossy().into_owned(),
        original: original_text,
        size: meta.len(),
        mtime_ms: mtime,
        copied_at: OffsetDateTime::now_utc().unix_timestamp(),
    };
    db.set_transcript_copy(id, &record)?;
    Ok(Some(record))
}

/// Whether `copy` is a file inside this database's copy store. A row's
/// recorded copy path is only trusted when it is.
fn inside_copies(db: &Db, copy: &Path) -> bool {
    let (Ok(root), Ok(copy)) = (
        std::fs::canonicalize(db.transcript_copies_dir()),
        std::fs::canonicalize(copy),
    ) else {
        return false;
    };
    copy.starts_with(root) && copy.is_file()
}

/// The readable transcript for a session: its original when present, else its
/// copy. `None` when neither exists.
pub fn readable_transcript(db: &Db, id: &str) -> Option<(PathBuf, HistoryRow)> {
    let row = db.history_row(id).ok()??;
    if let Some(record) = &row.copy {
        let original = PathBuf::from(&record.original);
        if original.is_file() {
            return Some((original, row));
        }
        let copy = PathBuf::from(&record.copy);
        if inside_copies(db, &copy) {
            return Some((copy, row));
        }
    }
    let original = PathBuf::from(row.transcript_path.as_deref()?);
    original.is_file().then_some((original, row))
}

/// Before a resume: when the provider's transcript is gone and a copy exists,
/// put the copy back at the provider's path so the provider can resume it.
/// Never overwrites an existing file. Blocking. Returns the restored path.
pub fn restore_before_resume(db: &Db, id: &str) -> Result<Option<PathBuf>> {
    if !crate::daemon::api::valid_session_id(id) {
        return Ok(None);
    }
    let Some(row) = db.history_row(id)? else {
        return Ok(None);
    };
    let Some(record) = row.copy else {
        return Ok(None);
    };
    let original = PathBuf::from(&record.original);
    if original.exists() {
        return Ok(None);
    }
    let copy = PathBuf::from(&record.copy);
    if !inside_copies(db, &copy) {
        return Ok(None);
    }
    if !restore_target_allowed(row.provider.as_deref().unwrap_or("claude"), &original) {
        bail!("refusing to restore a transcript outside the provider's own directory");
    }
    if let Some(parent) = original.parent() {
        std::fs::create_dir_all(parent)?;
    }
    // `create_new` is the no-overwrite guarantee: a file that appeared since
    // the check above wins.
    let mut target = match std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&original)
    {
        Ok(file) => file,
        Err(err) if err.kind() == std::io::ErrorKind::AlreadyExists => return Ok(None),
        Err(err) => return Err(err.into()),
    };
    let mut source = std::fs::File::open(&copy)?;
    if let Err(err) = std::io::copy(&mut source, &mut target) {
        drop(target);
        let _ = std::fs::remove_file(&original);
        return Err(err.into());
    }
    target.sync_all()?;
    tracing::info!(session = %id, path = %original.display(), "restored transcript from history copy");
    Ok(Some(original))
}

/// A restore writes where the provider reads: Claude's projects roots, or the
/// Codex home's `sessions` tree. Anything else recorded on a row is refused.
fn restore_target_allowed(provider: &str, path: &Path) -> bool {
    use std::path::Component;
    if !path.is_absolute() || path.components().any(|c| matches!(c, Component::ParentDir)) {
        return false;
    }
    match provider {
        "claude" => crate::session::transcript::path_is_allowed(path),
        "codex" => crate::providers::codex_rollout::codex_home()
            .is_some_and(|home| path.starts_with(home.join("sessions"))),
        _ => false,
    }
}

/// Remove a session's transcript copy, if it lives in the copy store.
pub fn remove_copy(db: &Db, record: &CopyRecord) -> bool {
    let copy = PathBuf::from(&record.copy);
    inside_copies(db, &copy) && std::fs::remove_file(copy).is_ok()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::session::conversation::tests::{transcript_fixture, two_turn_rows, Scratch};
    use crate::session::Transport;

    fn db() -> Db {
        Db::open(crate::testtmp::db_path("history-test")).unwrap()
    }

    fn state(id: &str, provider: &str, mode: SessionMode, updated_at: i64) -> SessionState {
        let mut s = SessionState::new(id.into(), Some("/work".into()));
        s.provider = provider.into();
        s.transport = Transport::Stream;
        s.mode = mode;
        s.updated_at = OffsetDateTime::from_unix_timestamp(updated_at).unwrap();
        s.started_at = OffsetDateTime::from_unix_timestamp(updated_at - 5).unwrap();
        s
    }

    fn record(db: &Db, s: &SessionState, create: bool) {
        db.record_history(&[(HistoryFacts::from_state(s), create)])
            .unwrap();
    }

    fn hook(event: &str, id: &str, at: Option<i64>) -> crate::session::HookEvent {
        crate::session::HookEvent {
            event: event.into(),
            session_id: id.into(),
            cwd: Some("/w".into()),
            timestamp: at.and_then(|at| OffsetDateTime::from_unix_timestamp(at).ok()),
            payload: serde_json::Map::new(),
        }
    }

    #[test]
    fn managed_sessions_get_rows_and_shells_only_update_existing_ones() {
        let db = db();
        record(
            &db,
            &state("codex-1", "codex", SessionMode::Input, 100),
            true,
        );
        let row = db.history_row("codex-1").unwrap().unwrap();
        assert_eq!((row.provider(), row.transport()), ("codex", "stream"));
        assert_eq!(row.cwd, "/work");
        assert_eq!(row.ended_at, None);

        // Never seen in an agent mode (a shell): no row is invented.
        record(
            &db,
            &state("shell", "claude", SessionMode::Stopped, 100),
            false,
        );
        assert!(db.history_row("shell").unwrap().is_none());

        // A hook-born row (no provider column yet) is enriched in place.
        db.record_event(&hook("SessionStart", "hooked", Some(50)))
            .unwrap();
        assert_eq!(db.history_row("hooked").unwrap().unwrap().provider, None);
        record(
            &db,
            &state("hooked", "claude", SessionMode::Input, 60),
            false,
        );
        let row = db.history_row("hooked").unwrap().unwrap();
        assert_eq!(row.provider.as_deref(), Some("claude"));
        assert_eq!(row.cwd, "/w", "the first nonempty cwd sticks");
        assert_eq!(row.last_event_at, 60);
        // An older recorder write never moves last activity backwards.
        db.record_event(&hook("Stop", "hooked", Some(90))).unwrap();
        record(
            &db,
            &state("hooked", "claude", SessionMode::Input, 70),
            false,
        );
        assert_eq!(db.history_row("hooked").unwrap().unwrap().last_event_at, 90);
    }

    #[test]
    fn ended_at_marks_a_stop_and_clears_on_resume() {
        let db = db();
        record(
            &db,
            &state("s", "claude", SessionMode::Responding, 100),
            true,
        );
        record(&db, &state("s", "claude", SessionMode::Stopped, 200), true);
        assert_eq!(db.history_row("s").unwrap().unwrap().ended_at, Some(200));
        // A repeated stopped write keeps the first stop time.
        record(&db, &state("s", "claude", SessionMode::Stopped, 250), true);
        assert_eq!(db.history_row("s").unwrap().unwrap().ended_at, Some(200));
        record(&db, &state("s", "claude", SessionMode::Input, 300), true);
        assert_eq!(db.history_row("s").unwrap().unwrap().ended_at, None);
    }

    #[test]
    fn history_pages_newest_first_with_a_stable_cursor_through_ties() {
        let db = db();
        // Seven sessions; three share one timestamp to put a page edge in a tie.
        for (id, at) in [
            ("a", 10),
            ("b", 20),
            ("c", 30),
            ("t1", 40),
            ("t2", 40),
            ("t3", 40),
            ("z", 50),
        ] {
            record(&db, &state(id, "codex", SessionMode::Input, at), true);
        }
        let mut seen = Vec::new();
        let mut before = None;
        loop {
            let page = db
                .history_page(&HistoryQuery {
                    before: before.clone(),
                    limit: Some(2),
                    ..Default::default()
                })
                .unwrap();
            seen.extend(
                page.sessions
                    .iter()
                    .map(|r| r["session_id"].as_str().unwrap().to_owned()),
            );
            match page.next {
                Some(next) => before = Some(next),
                None => break,
            }
        }
        assert_eq!(seen, ["z", "t3", "t2", "t1", "c", "b", "a"]);

        let all = db.history_page(&HistoryQuery::default()).unwrap();
        assert_eq!(all.sessions.len(), 7);
        assert!(all.next.is_none());
        let row = &all.sessions[0];
        assert_eq!(row["mode"], "stopped");
        assert_eq!(row["provider"], "codex");
        assert_eq!(row["updated_at"], "1970-01-01T00:00:50Z");
        assert_eq!(row["transcript_copy"], false);

        for bad in ["garbage", "10:../x", "x:abc"] {
            assert!(
                db.history_page(&HistoryQuery {
                    before: Some(bad.into()),
                    ..Default::default()
                })
                .is_err(),
                "{bad}"
            );
        }
        // Limits clamp to [1, MAX_LIMIT].
        let one = db
            .history_page(&HistoryQuery {
                limit: Some(0),
                ..Default::default()
            })
            .unwrap();
        assert_eq!(one.sessions.len(), 1);
    }

    #[test]
    fn history_search_matches_prompts_titles_and_filters_by_cwd() {
        let db = db();
        let mut titled = state("titled", "codex", SessionMode::Input, 10);
        titled.thread_name = Some("Refactor Billing".into());
        record(&db, &titled, true);
        let mut elsewhere = state("elsewhere", "codex", SessionMode::Input, 20);
        elsewhere.cwd = Some("/other".into());
        record(&db, &elsewhere, true);
        let mut prompt = hook("UserPromptSubmit", "prompted", Some(30));
        prompt.cwd = Some("/work".into());
        prompt
            .payload
            .insert("prompt".into(), json!("Fix the LOGIN flow"));
        db.record_event(&prompt).unwrap();

        let ids = |q: HistoryQuery| -> Vec<String> {
            db.history_page(&q)
                .unwrap()
                .sessions
                .iter()
                .map(|r| r["session_id"].as_str().unwrap().to_owned())
                .collect()
        };
        let q = |text: &str| HistoryQuery {
            q: Some(text.into()),
            ..Default::default()
        };
        assert_eq!(ids(q("billing")), ["titled"]);
        assert_eq!(ids(q("login")), ["prompted"]);
        assert_eq!(ids(q("/other")), ["elsewhere"]);
        assert!(ids(q("nothing-matches")).is_empty());
        assert_eq!(
            ids(HistoryQuery {
                cwd: Some("/work".into()),
                ..Default::default()
            }),
            ["prompted", "titled"]
        );
        let row = db.history_row("prompted").unwrap().unwrap();
        assert_eq!(row.user_prompts, 1);
        assert_eq!(row.first_prompt.as_deref(), Some("Fix the LOGIN flow"));
    }

    #[test]
    fn copies_are_written_on_change_only_and_restored_without_overwriting() {
        let db = db();
        let (_dir, original) = transcript_fixture(&two_turn_rows());
        let original = PathBuf::from(original);
        let mut s = state("resumable", "claude", SessionMode::Stopped, 100);
        s.transcript_path = Some(original.to_string_lossy().into_owned());
        record(&db, &s, true);

        let first = copy_transcript(&db, "resumable", "claude", &original)
            .unwrap()
            .expect("first copy is written");
        assert!(Path::new(&first.copy).ends_with("claude/resumable.jsonl"));
        assert_eq!(
            std::fs::read(&first.copy).unwrap(),
            std::fs::read(&original).unwrap()
        );
        assert!(
            copy_transcript(&db, "resumable", "claude", &original)
                .unwrap()
                .is_none(),
            "an unchanged transcript is not copied again"
        );
        let grown = format!(
            "{}{}\n",
            std::fs::read_to_string(&original).unwrap(),
            two_turn_rows()[0]
        );
        std::fs::write(&original, &grown).unwrap();
        let second = copy_transcript(&db, "resumable", "claude", &original)
            .unwrap()
            .expect("a changed transcript is copied again");
        assert_eq!(std::fs::read_to_string(&second.copy).unwrap(), grown);
        assert_eq!(
            db.history_row("resumable").unwrap().unwrap().copy,
            Some(second.clone())
        );

        // A session with no history row gets no copy.
        assert!(copy_transcript(&db, "unknown", "claude", &original)
            .unwrap()
            .is_none());

        // Present original: nothing is restored or touched.
        assert!(restore_before_resume(&db, "resumable").unwrap().is_none());
        assert_eq!(std::fs::read_to_string(&original).unwrap(), grown);
        // The provider deleted it: reads fall back to the copy, and a resume
        // puts the copy back where the provider looks.
        std::fs::remove_file(&original).unwrap();
        let (readable, _) = readable_transcript(&db, "resumable").unwrap();
        assert_eq!(readable, PathBuf::from(&second.copy));
        let restored = restore_before_resume(&db, "resumable")
            .unwrap()
            .expect("restored");
        assert_eq!(restored, original);
        assert_eq!(std::fs::read_to_string(&original).unwrap(), grown);
        assert_eq!(readable_transcript(&db, "resumable").unwrap().0, original);
    }

    #[test]
    fn restore_refuses_targets_outside_the_providers_directory() {
        let db = db();
        let scratch = Scratch::new();
        let original = scratch.0.join("notes.jsonl");
        std::fs::write(&original, "{}\n").unwrap();
        record(&db, &state("odd", "claude", SessionMode::Stopped, 1), true);
        copy_transcript(&db, "odd", "claude", &original)
            .unwrap()
            .unwrap();
        std::fs::remove_file(&original).unwrap();
        assert!(restore_before_resume(&db, "odd").is_err());
        assert!(!original.exists());
        // Providers without a file transcript never restore.
        assert!(!restore_target_allowed("opencode", &original));
    }

    #[test]
    fn forgetting_deletes_the_row_its_events_and_its_copy() {
        let db = db();
        let (_dir, original) = transcript_fixture(&two_turn_rows());
        record(
            &db,
            &state("gone", "claude", SessionMode::Stopped, 10),
            true,
        );
        let mut event = hook("UserPromptSubmit", "gone", None);
        event.payload.insert("prompt".into(), json!("hello"));
        db.record_event(&event).unwrap();
        let copied = copy_transcript(&db, "gone", "claude", Path::new(&original))
            .unwrap()
            .unwrap();
        let row = db.forget_history("gone").unwrap().expect("deleted row");
        assert!(remove_copy(&db, row.copy.as_ref().unwrap()));
        assert!(!Path::new(&copied.copy).exists());
        assert!(
            Path::new(&original).exists(),
            "the provider's file is untouched"
        );
        assert!(db.history_row("gone").unwrap().is_none());
        let events: i64 = db
            .conn
            .lock()
            .unwrap()
            .query_row(
                "SELECT COUNT(*) FROM events WHERE session_id = 'gone'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(events, 0);
        assert!(db.forget_history("gone").unwrap().is_none());
    }

    #[test]
    fn hydration_restores_the_provider_and_transport_of_managed_rows() {
        let db = db();
        record(
            &db,
            &state("codex-1", "codex", SessionMode::Stopped, 100),
            true,
        );
        let restored = db.load_recent_sessions(10).unwrap();
        let store = crate::session::SessionStore::new();
        store.hydrate(restored);
        let state = store.get("codex-1").unwrap();
        assert_eq!(state.provider, "codex");
        assert_eq!(state.transport, Transport::Stream);
        assert_eq!(state.mode, SessionMode::Stopped);
    }
}
