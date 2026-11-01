//! The Scratch page's model: a project's `scratch.list` reply, read into rows
//! the page can order and filter, and the `scratch.mark` patches its buttons
//! send. Scratch holds personal files kept outside git — harnesses, repros,
//! SQL meant to be run by hand — shared by every worktree of a project
//! (`docs/features/scratch.md`). The page reads and marks; it never runs a
//! file, and it never writes a file's contents.
use serde_json::{Value, json};

/// The text the page shows inline is capped; the rest stays on disk.
pub const PREVIEW_BYTES: usize = 256 * 1024;

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum Status {
    /// Waiting on the user: SQL to run on prod, a step to do by hand.
    Pending,
    Draft,
    /// Written straight into the directory by an agent; no purpose recorded.
    Unrecorded,
    Done,
    Archived,
}

impl Status {
    pub fn parse(item: &Value) -> Self {
        match item["status"].as_str() {
            Some("pending") => Self::Pending,
            Some("done") => Self::Done,
            Some("archived") => Self::Archived,
            Some(_) => Self::Draft,
            None if item["indexed"] == true => Self::Draft,
            None => Self::Unrecorded,
        }
    }
    /// The wire value `scratch.mark` takes; `None` for Unrecorded.
    pub fn wire(self) -> Option<&'static str> {
        match self {
            Self::Pending => Some("pending"),
            Self::Draft => Some("draft"),
            Self::Done => Some("done"),
            Self::Archived => Some("archived"),
            Self::Unrecorded => None,
        }
    }
    pub fn label(self) -> &'static str {
        match self {
            Self::Pending => "pending",
            Self::Draft => "draft",
            Self::Unrecorded => "no purpose yet",
            Self::Done => "done",
            Self::Archived => "archived",
        }
    }
    /// Still in play: the Open filter's rows.
    pub fn open(self) -> bool {
        !matches!(self, Self::Done | Self::Archived)
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct Item {
    /// Relative to the project's scratch `files/` directory.
    pub path: String,
    pub size: u64,
    pub modified_at: Option<i64>,
    pub purpose: Option<String>,
    pub note: Option<String>,
    pub session_id: Option<String>,
    pub keep: bool,
    pub sensitive: bool,
    pub status: Status,
    pub status_at: Option<i64>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Listing {
    /// The project the scratch belongs to (a worktree resolves to its main
    /// checkout).
    pub root: String,
    pub key: String,
    /// The directory the files live in, on the hub's machine.
    pub files: String,
    pub exists: bool,
    /// Ordered for the page: pending first, then in play, then settled;
    /// newest first within each.
    pub items: Vec<Item>,
    pub truncated: bool,
    pub index_error: Option<String>,
}

impl Listing {
    pub fn pending(&self) -> usize {
        self.items
            .iter()
            .filter(|i| i.status == Status::Pending)
            .count()
    }
    pub fn absolute(&self, item: &Item) -> String {
        join(&self.files, &item.path)
    }
}

fn join(dir: &str, rel: &str) -> String {
    let sep = if dir.contains('\\') && !dir.contains('/') {
        '\\'
    } else {
        '/'
    };
    let rel = if sep == '\\' {
        rel.replace('/', "\\")
    } else {
        rel.to_owned()
    };
    format!("{}{sep}{rel}", dir.trim_end_matches(['/', '\\']))
}

fn text(value: &Value) -> Option<String> {
    value
        .as_str()
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_owned)
}

fn millis(value: &Value) -> Option<i64> {
    let text = value.as_str()?;
    chrono::DateTime::parse_from_rfc3339(text)
        .ok()
        .map(|t| t.timestamp_millis())
}

/// A `scratch.list` (or `scratch.mark`, which answers with one) reply.
/// `None` for anything else.
pub fn parse_list(value: &Value) -> Option<Listing> {
    let project = value.get("project")?;
    let rows = value.get("items")?.as_array()?;
    let mut items: Vec<Item> = rows
        .iter()
        .filter_map(|row| {
            Some(Item {
                path: text(&row["path"])?,
                size: row["size"].as_u64().unwrap_or(0),
                modified_at: millis(&row["modifiedAt"]),
                purpose: text(&row["purpose"]),
                note: text(&row["note"]),
                session_id: text(&row["sessionId"]),
                keep: row["keep"] == true,
                sensitive: row["sensitive"] == true,
                status: Status::parse(row),
                status_at: millis(&row["statusAt"]),
            })
        })
        .collect();
    items.sort_by(|a, b| {
        a.status
            .cmp(&b.status)
            .then(b.modified_at.cmp(&a.modified_at))
            .then(a.path.cmp(&b.path))
    });
    Some(Listing {
        root: text(&project["root"]).unwrap_or_default(),
        key: text(&project["key"]).unwrap_or_default(),
        files: text(&project["files"]).unwrap_or_default(),
        exists: project["exists"] == true,
        items,
        truncated: value["truncated"] == true,
        index_error: text(&value["indexError"]),
    })
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Filter {
    /// Everything not done or archived.
    #[default]
    Open,
    Pending,
    All,
}

impl Filter {
    pub fn admits(self, item: &Item) -> bool {
        match self {
            Self::Open => item.status.open(),
            Self::Pending => item.status == Status::Pending,
            Self::All => true,
        }
    }
}

/// What a `scratch.read` reply holds for the page.
#[derive(Clone, Debug, PartialEq)]
pub enum Content {
    Text {
        text: String,
        clipped: bool,
    },
    /// The hub kept a sensitive file's text from this connection.
    Withheld(String),
}

pub fn parse_read(value: &Value) -> Option<Content> {
    if let Some(reason) = text(&value["withheld"]) {
        return Some(Content::Withheld(reason));
    }
    let content = value["content"].as_str()?;
    if content.len() <= PREVIEW_BYTES {
        return Some(Content::Text {
            text: content.to_owned(),
            clipped: false,
        });
    }
    let mut end = PREVIEW_BYTES;
    while !content.is_char_boundary(end) {
        end -= 1;
    }
    Some(Content::Text {
        text: content[..end].to_owned(),
        clipped: true,
    })
}

/// The `scratch.mark` patch for a status button.
pub fn status_patch(status: Status) -> Option<Value> {
    status.wire().map(|s| json!({"status": s}))
}

/// The `scratch.mark` patch for the purpose/note editor: only what changed.
/// A purpose cannot be cleared (the hub refuses it), so an emptied purpose
/// box leaves the purpose alone. `None` when nothing changed.
pub fn text_patch(item: &Item, purpose: &str, note: &str) -> Option<Value> {
    let mut patch = serde_json::Map::new();
    let purpose = purpose.trim();
    if !purpose.is_empty() && item.purpose.as_deref() != Some(purpose) {
        patch.insert("purpose".into(), json!(purpose));
    }
    let note = note.trim();
    if item.note.as_deref().unwrap_or("") != note {
        patch.insert("note".into(), json!(note));
    }
    (!patch.is_empty()).then_some(Value::Object(patch))
}

pub fn size_label(bytes: u64) -> String {
    match bytes {
        0..1024 => format!("{bytes} B"),
        1024..1_048_576 => format!("{:.1} KB", bytes as f64 / 1024.0),
        _ => format!("{:.1} MB", bytes as f64 / 1_048_576.0),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn listed() -> Value {
        json!({
            "project": {"root":"/work/shop","key":"shop-0011223344556677","dir":"/h/.workspacer/scratch/shop-0011223344556677","files":"/h/.workspacer/scratch/shop-0011223344556677/files","exists":true},
            "items": [
                {"path":"old.sql","size":10,"modifiedAt":"2026-10-01T00:00:00Z","indexed":true,"purpose":"old backfill","status":"done","statusAt":"2026-10-02T00:00:00Z","note":"ran it","keep":false,"sensitive":false},
                {"path":"repro.sh","size":2048,"modifiedAt":"2026-11-01T10:00:00Z","indexed":false,"purpose":null,"status":null,"keep":false,"sensitive":false},
                {"path":"prod/backfill.sql","size":300,"modifiedAt":"2026-10-30T00:00:00Z","indexed":true,"purpose":"backfill orders","status":"pending","keep":true,"sensitive":true,"sessionId":"s1"},
                {"path":"harness/load.ts","size":5,"modifiedAt":"2026-10-31T00:00:00Z","indexed":true,"purpose":"load harness","status":"draft","keep":true,"sensitive":false},
                {"path":"gone.txt","size":1,"indexed":true,"purpose":"x","status":"archived"},
                {"path":"","size":1}
            ],
            "truncated": false
        })
    }

    #[test]
    fn pending_first_then_open_then_settled_newest_first() {
        let list = parse_list(&listed()).unwrap();
        let order: Vec<_> = list.items.iter().map(|i| i.path.as_str()).collect();
        assert_eq!(
            order,
            [
                "prod/backfill.sql",
                "harness/load.ts",
                "repro.sh",
                "old.sql",
                "gone.txt"
            ]
        );
        assert_eq!(list.pending(), 1);
        assert_eq!(list.items[2].status, Status::Unrecorded);
        assert_eq!(list.items[0].session_id.as_deref(), Some("s1"));
        assert!(list.items[0].sensitive && list.items[0].keep);
        assert_eq!(
            list.absolute(&list.items[0]),
            "/h/.workspacer/scratch/shop-0011223344556677/files/prod/backfill.sql"
        );
        assert!(parse_list(&json!({"jobs":[]})).is_none());
    }

    #[test]
    fn filters() {
        let list = parse_list(&listed()).unwrap();
        let count = |f: Filter| list.items.iter().filter(|i| f.admits(i)).count();
        assert_eq!(count(Filter::Open), 3);
        assert_eq!(count(Filter::Pending), 1);
        assert_eq!(count(Filter::All), 5);
    }

    #[test]
    fn reads_clip_on_a_char_boundary_and_report_withheld_content() {
        let big = "é".repeat(PREVIEW_BYTES);
        let Some(Content::Text { text, clipped }) = parse_read(&json!({"content": big})) else {
            panic!("text");
        };
        assert!(clipped && text.len() <= PREVIEW_BYTES && !text.is_empty());
        assert_eq!(
            parse_read(&json!({"content":"SELECT 1;"})),
            Some(Content::Text {
                text: "SELECT 1;".into(),
                clipped: false
            })
        );
        assert!(matches!(
            parse_read(&json!({"content":null,"withheld":"sensitive"})),
            Some(Content::Withheld(_))
        ));
    }

    #[test]
    fn patches_send_only_what_changed_and_never_clear_a_purpose() {
        let list = parse_list(&listed()).unwrap();
        let item = &list.items[0];
        assert_eq!(status_patch(Status::Done), Some(json!({"status":"done"})));
        assert_eq!(status_patch(Status::Unrecorded), None);
        assert_eq!(text_patch(item, "backfill orders", ""), None);
        assert_eq!(
            text_patch(item, "  ", "ran on prod 11/1"),
            Some(json!({"note":"ran on prod 11/1"}))
        );
        assert_eq!(
            text_patch(item, "backfill orders v2", ""),
            Some(json!({"purpose":"backfill orders v2"}))
        );
        let done = &list.items[3];
        assert_eq!(
            text_patch(done, "old backfill", ""),
            Some(json!({"note":""}))
        );
    }

    #[test]
    fn sizes() {
        assert_eq!(size_label(10), "10 B");
        assert_eq!(size_label(2048), "2.0 KB");
        assert_eq!(size_label(3 * 1_048_576), "3.0 MB");
    }
}
