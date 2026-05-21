//! Inbox-item queries and user-action writes used by the v2 API surface.
//!
//! Reads are denormalized: each [`ItemRow`] also carries the session name
//! and current session state so the L1 inbox view can render rows without
//! a second query per item. Writes (archive, snooze, flag) are small targeted
//! UPDATEs that return the updated row so callers can broadcast it.

use anyhow::{anyhow, Result};
use rusqlite::{params, OptionalExtension, Row, Transaction};
use serde::{Deserialize, Serialize};
use tokio::sync::broadcast;

use super::Db;

const ITEM_CHANGES_CAPACITY: usize = 256;

/// Fanout for inbox-item changes. Subscribed by the SSE endpoint and any
/// other client (Workspacer's IPC bridge) that wants live updates.
#[derive(Clone)]
pub struct ItemBroadcaster {
    tx: broadcast::Sender<ItemChange>,
}

impl ItemBroadcaster {
    pub fn new() -> Self {
        let (tx, _) = broadcast::channel(ITEM_CHANGES_CAPACITY);
        Self { tx }
    }

    pub fn subscribe(&self) -> broadcast::Receiver<ItemChange> {
        self.tx.subscribe()
    }

    /// Best-effort send. If no subscribers exist, the message is dropped
    /// without surfacing an error.
    pub fn send(&self, change: ItemChange) {
        let _ = self.tx.send(change);
    }
}

impl Default for ItemBroadcaster {
    fn default() -> Self {
        Self::new()
    }
}

/// Mirrors the spec §12 IPC event names: item_created / item_changed /
/// item_resolved. Touch-on-existing-item is surfaced as `item_changed`
/// so subscribers only need one upsert path.
#[derive(Debug, Clone, Serialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ItemChange {
    ItemCreated { item: ItemRow },
    ItemChanged { item: ItemRow },
    ItemResolved { id: String, session_id: String },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ItemRow {
    pub id: String,
    pub session_id: String,
    pub state: String,
    pub priority: i32,
    pub kind: String,
    pub summary: Option<String>,
    pub context_paragraph: Option<String>,
    pub next_action: Option<String>,
    pub triggering_event_id: Option<i64>,
    pub created_at: i64,
    pub updated_at: i64,
    pub resolved_at: Option<i64>,
    pub snoozed_until: Option<i64>,
    pub snoozed_on_event: Option<String>,
    pub flagged: bool,
    pub session_name: String,
    pub session_project: String,
    pub session_state: String,
}

#[derive(Debug, Clone, Copy, Default, Deserialize)]
#[serde(default)]
pub struct ListFilter {
    /// Include items in `snoozed` state. Off by default — they're hidden
    /// from the main inbox view per spec §7.
    pub include_snoozed: bool,
    /// Include items in `resolved` state. Off by default.
    pub include_resolved: bool,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(tag = "action", rename_all = "snake_case")]
pub enum ItemAction {
    /// Resolve the item explicitly. State goes to `resolved`, resolved_at = now.
    Archive,
    /// Snooze until a specific unix timestamp.
    SnoozeUntil { until: i64 },
    /// Snooze until a named event fires.
    SnoozeOnEvent { on: String },
    /// Wake a snoozed item back to unread.
    Unsnooze,
    /// Pin to top (and conceptually pause the agent — pause logic is a
    /// separate concern; this just flips the flag).
    Flag,
    Unflag,
}

const SELECT_ITEM_COLUMNS: &str = r#"
SELECT
    i.id, i.session_id, i.state, i.priority, i.kind, i.summary,
    i.context_paragraph, i.next_action, i.triggering_event_id,
    i.created_at, i.updated_at, i.resolved_at, i.snoozed_until,
    i.snoozed_on_event, i.flagged,
    s.name, s.project, s.state
FROM items i
JOIN sessions s ON s.id = i.session_id
"#;

fn row_to_item(row: &Row<'_>) -> rusqlite::Result<ItemRow> {
    Ok(ItemRow {
        id: row.get(0)?,
        session_id: row.get(1)?,
        state: row.get(2)?,
        priority: row.get(3)?,
        kind: row.get(4)?,
        summary: row.get(5)?,
        context_paragraph: row.get(6)?,
        next_action: row.get(7)?,
        triggering_event_id: row.get(8)?,
        created_at: row.get(9)?,
        updated_at: row.get(10)?,
        resolved_at: row.get(11)?,
        snoozed_until: row.get(12)?,
        snoozed_on_event: row.get(13)?,
        flagged: row.get::<_, i32>(14)? != 0,
        session_name: row.get(15)?,
        session_project: row.get(16)?,
        session_state: row.get(17)?,
    })
}

impl Db {
    pub fn list_items(&self, filter: ListFilter) -> Result<Vec<ItemRow>> {
        let mut sql = String::from(SELECT_ITEM_COLUMNS);
        sql.push_str("WHERE 1 = 1");
        if !filter.include_snoozed {
            sql.push_str(" AND i.state != 'snoozed'");
        }
        if !filter.include_resolved {
            sql.push_str(" AND i.state != 'resolved'");
        }
        sql.push_str(" ORDER BY i.flagged DESC, i.priority DESC, i.updated_at DESC");

        let guard = self.conn.lock().expect("db mutex poisoned");
        let mut stmt = guard.prepare(&sql)?;
        let rows = stmt.query_map([], row_to_item)?;
        Ok(rows.collect::<rusqlite::Result<Vec<_>>>()?)
    }

    pub fn get_item(&self, id: &str) -> Result<Option<ItemRow>> {
        let guard = self.conn.lock().expect("db mutex poisoned");
        let mut stmt = guard.prepare(&format!("{SELECT_ITEM_COLUMNS} WHERE i.id = ?1"))?;
        Ok(stmt.query_row(params![id], row_to_item).optional()?)
    }

    /// Apply a user action to an item. Returns the updated row. Errors
    /// `NotFound` if the item doesn't exist; the API layer maps that to 404.
    pub fn apply_item_action(
        &self,
        id: &str,
        action: &ItemAction,
        now_unix: i64,
    ) -> Result<ItemRow> {
        let mut guard = self.conn.lock().expect("db mutex poisoned");
        let tx = guard.transaction()?;
        apply_action_tx(&tx, id, action, now_unix)?;
        let updated = {
            let mut stmt = tx.prepare(&format!("{SELECT_ITEM_COLUMNS} WHERE i.id = ?1"))?;
            stmt.query_row(params![id], row_to_item)?
        };
        tx.commit()?;
        Ok(updated)
    }
}

fn apply_action_tx(
    tx: &Transaction<'_>,
    id: &str,
    action: &ItemAction,
    now_unix: i64,
) -> Result<()> {
    let affected = match action {
        ItemAction::Archive => tx.execute(
            "UPDATE items SET state = 'resolved', resolved_at = ?1, updated_at = ?1
             WHERE id = ?2",
            params![now_unix, id],
        )?,
        ItemAction::SnoozeUntil { until } => tx.execute(
            "UPDATE items SET state = 'snoozed',
                snoozed_until = ?1, snoozed_on_event = NULL, updated_at = ?2
             WHERE id = ?3",
            params![until, now_unix, id],
        )?,
        ItemAction::SnoozeOnEvent { on } => tx.execute(
            "UPDATE items SET state = 'snoozed',
                snoozed_on_event = ?1, snoozed_until = NULL, updated_at = ?2
             WHERE id = ?3",
            params![on, now_unix, id],
        )?,
        ItemAction::Unsnooze => tx.execute(
            "UPDATE items SET state = 'unread',
                snoozed_until = NULL, snoozed_on_event = NULL, updated_at = ?1
             WHERE id = ?2",
            params![now_unix, id],
        )?,
        ItemAction::Flag => tx.execute(
            "UPDATE items SET flagged = 1, updated_at = ?1 WHERE id = ?2",
            params![now_unix, id],
        )?,
        ItemAction::Unflag => tx.execute(
            "UPDATE items SET flagged = 0, updated_at = ?1 WHERE id = ?2",
            params![now_unix, id],
        )?,
    };
    if affected == 0 {
        return Err(anyhow!("no such item: {id}"));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::session::HookEvent;
    use std::path::PathBuf;

    fn ev(event: &str, session_id: &str) -> HookEvent {
        HookEvent {
            event: event.to_string(),
            session_id: session_id.to_string(),
            cwd: Some("/tmp/proj".to_string()),
            timestamp: None,
            payload: serde_json::Map::new(),
        }
    }

    fn tempfile_path() -> PathBuf {
        let mut p = std::env::temp_dir();
        p.push(format!("claudemon-items-test-{}.db", uuid::Uuid::new_v4()));
        p
    }

    fn seed(db: &Db) {
        db.record_and_classify(&ev("SessionStart", "s1"), 1000).unwrap();
        let mut req = ev("PermissionRequest", "s1");
        req.payload.insert("tool_name".into(), serde_json::json!("Bash"));
        db.record_and_classify(&req, 1001).unwrap();
    }

    #[test]
    fn list_items_default_hides_snoozed_and_resolved() {
        let db = Db::open(tempfile_path()).unwrap();
        seed(&db);
        let items = db.list_items(ListFilter::default()).unwrap();
        assert_eq!(items.len(), 1);
        assert_eq!(items[0].session_name, "proj");
        assert_eq!(items[0].state, "unread");
    }

    #[test]
    fn archive_marks_resolved_and_hides_from_default_list() {
        let db = Db::open(tempfile_path()).unwrap();
        seed(&db);
        let items = db.list_items(ListFilter::default()).unwrap();
        let id = items[0].id.clone();
        let updated = db.apply_item_action(&id, &ItemAction::Archive, 1002).unwrap();
        assert_eq!(updated.state, "resolved");
        assert_eq!(updated.resolved_at, Some(1002));
        assert!(db.list_items(ListFilter::default()).unwrap().is_empty());
        let with_resolved = db
            .list_items(ListFilter {
                include_resolved: true,
                ..Default::default()
            })
            .unwrap();
        assert_eq!(with_resolved.len(), 1);
    }

    #[test]
    fn snooze_until_sets_timestamp() {
        let db = Db::open(tempfile_path()).unwrap();
        seed(&db);
        let id = db.list_items(ListFilter::default()).unwrap()[0].id.clone();
        let updated = db
            .apply_item_action(&id, &ItemAction::SnoozeUntil { until: 9999 }, 1002)
            .unwrap();
        assert_eq!(updated.state, "snoozed");
        assert_eq!(updated.snoozed_until, Some(9999));
        assert!(updated.snoozed_on_event.is_none());
    }

    #[test]
    fn snooze_on_event_sets_event_name() {
        let db = Db::open(tempfile_path()).unwrap();
        seed(&db);
        let id = db.list_items(ListFilter::default()).unwrap()[0].id.clone();
        let updated = db
            .apply_item_action(
                &id,
                &ItemAction::SnoozeOnEvent { on: "next_event".into() },
                1002,
            )
            .unwrap();
        assert_eq!(updated.state, "snoozed");
        assert_eq!(updated.snoozed_on_event.as_deref(), Some("next_event"));
        assert!(updated.snoozed_until.is_none());
    }

    #[test]
    fn flag_pins_to_top() {
        let db = Db::open(tempfile_path()).unwrap();
        // Two items, different priorities; ensure flagged one comes first.
        db.record_and_classify(&ev("SessionStart", "s1"), 1000).unwrap();
        let mut req = ev("PermissionRequest", "s1");
        req.payload.insert("tool_name".into(), serde_json::json!("Bash"));
        db.record_and_classify(&req, 1001).unwrap();
        db.record_and_classify(&ev("SessionStart", "s2"), 1002).unwrap();
        let mut fail = ev("PostToolUseFailure", "s2");
        fail.payload.insert("tool_name".into(), serde_json::json!("Edit"));
        db.record_and_classify(&fail, 1003).unwrap();

        let items_before = db.list_items(ListFilter::default()).unwrap();
        // Top should be priority 95 (needs_input)
        assert_eq!(items_before[0].priority, 95);
        let lower_id = items_before
            .iter()
            .find(|i| i.priority == 80)
            .unwrap()
            .id
            .clone();
        db.apply_item_action(&lower_id, &ItemAction::Flag, 1004).unwrap();
        let items_after = db.list_items(ListFilter::default()).unwrap();
        assert_eq!(items_after[0].id, lower_id, "flagged item should pin to top");
        assert!(items_after[0].flagged);
    }

    #[test]
    fn unsnooze_returns_to_unread() {
        let db = Db::open(tempfile_path()).unwrap();
        seed(&db);
        let id = db.list_items(ListFilter::default()).unwrap()[0].id.clone();
        db.apply_item_action(&id, &ItemAction::SnoozeUntil { until: 9999 }, 1002)
            .unwrap();
        let updated = db
            .apply_item_action(&id, &ItemAction::Unsnooze, 1003)
            .unwrap();
        assert_eq!(updated.state, "unread");
        assert!(updated.snoozed_until.is_none());
    }

    #[test]
    fn apply_action_to_unknown_item_errors() {
        let db = Db::open(tempfile_path()).unwrap();
        let err = db
            .apply_item_action("does-not-exist", &ItemAction::Archive, 1000)
            .unwrap_err();
        assert!(err.to_string().contains("no such item"));
    }

    #[test]
    fn get_item_returns_none_for_unknown_id() {
        let db = Db::open(tempfile_path()).unwrap();
        assert!(db.get_item("nope").unwrap().is_none());
    }
}
