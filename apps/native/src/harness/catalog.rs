//! `native-harness serve --catalog`: believable data for the feature pages
//! (Projects, project groups, Scratch, Jobs, Hosts, History) so they can be looked
//! at without a real hub. Every path is an obvious fixture path under
//! `/home/dev`; nothing here touches the disk or starts anything.
//!
//! Replies mirror the Rust hub's JSON (`services/hub-rs`: `project_groups.rs`,
//! `scratch.rs`, `jobs.rs`, `live_controls.rs` for `sessions.history`), and
//! writes (`projectGroups.save/remove`, `scratch.mark`, `jobs.upsert/remove/
//! run`, `sessions.forget`) land in memory for the connection's lifetime.
use serde_json::{Map, Value, json};
use std::collections::{BTreeMap, BTreeSet};

use crate::project_groups::within;
use crate::projects::{self, WorktreeRepo, same_dir};

pub const WORKSPACER: &str = "/home/dev/src/workspacer";
pub const NATIVE: &str = "/home/dev/src/workspacer/apps/native";
pub const HUB: &str = "/home/dev/src/hub-rs";
pub const INFRA: &str = "/home/dev/src/infra";
pub const NOTES: &str = "/home/dev/notes";
const SCRATCH_BASE: &str = "/home/dev/.workspacer/scratch";
const TREES: &str = "/home/dev/.workspacer/worktrees";

/// The repositories scratch resolves to (a subfolder or worktree files under
/// its repository's main checkout, as the hub's `project_root` does).
const SCRATCH_ROOTS: [&str; 4] = [WORKSPACER, HUB, INFRA, NOTES];

const MINUTE: i64 = 60_000;
const HOUR: i64 = 60 * MINUTE;
const DAY: i64 = 24 * HOUR;

/// Rows one `sessions.history` page holds here, below the hub's 200, so the
/// page's "Load older" cursor path has something to load.
const HISTORY_PAGE: usize = 20;
/// The hub's own bound on `limit`.
const HISTORY_LIMIT: u64 = 200;

/// What a live fixture session is doing. The first row's approval (or
/// plan) and the question are the harness's to add; see `harness.rs`.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Live {
    /// Waiting on the user's approval (the active session).
    Approval,
    /// Waiting on the user's answer to a question.
    Question,
    /// Mid-turn.
    Working,
    /// Ready for the next message.
    Idle,
    /// Ended.
    Ended,
}

/// One live fixture session: folder, title, provider, model, state, minutes
/// since its last activity, and the request that opened it.
type LiveRow = (
    &'static str,
    &'static str,
    &'static str,
    &'static str,
    Live,
    i64,
    &'static str,
);

/// The live fleet: a developer supervising a handful of agents across the
/// workspacer repository (with its native client), hub-rs, infra and notes.
/// Two need the user (an approval, a question), three are mid-turn (one a
/// hub-launched reviewer under the first), five sit idle and the rest ended
/// over the last five days. Every title is distinct; past the table rows
/// repeat with a " · N" suffix (load runs only: the rig serves exactly
/// [`LIVE_SESSIONS`]).
const LIVE: [LiveRow; 30] = [
    (
        NATIVE,
        "Restore sessions on launch",
        "claude",
        "sonnet",
        Live::Approval,
        3,
        "Sessions I had open don't come back after a restart. Restore them on launch, focused one included.",
    ),
    (
        WORKSPACER,
        "Session creation review",
        "codex",
        "gpt-5.6-sol",
        Live::Working,
        0,
        "Review session creation and model selection for regressions.",
    ),
    (
        HUB,
        "Bus fan-out backpressure",
        "claude",
        "sonnet",
        Live::Idle,
        18,
        "Slow subscribers stall the whole bus fan-out. Add per-subscriber backpressure.",
    ),
    (
        WORKSPACER,
        "Release checklist for 0.9",
        "claude",
        "opus",
        Live::Question,
        1,
        "Walk the 0.9 release checklist and cut the release branch.",
    ),
    (
        HUB,
        "Federation handshake retries",
        "codex",
        "gpt-5.6-sol",
        Live::Working,
        0,
        "Peers give up after one failed handshake. Retry with backoff.",
    ),
    (
        INFRA,
        "Fly deploy dry run",
        "claude",
        "sonnet",
        Live::Working,
        0,
        "Dry-run the Fly deploy for the hub and report anything that would change.",
    ),
    (
        NATIVE,
        "Scroll perf with 1,000 sessions",
        "claude",
        "sonnet",
        Live::Idle,
        42,
        "Profile sidebar scrolling with 1,000 sessions and fix any dropped frames.",
    ),
    (
        NOTES,
        "Weekly notes tidy-up",
        "claude",
        "haiku",
        Live::Idle,
        2 * 60,
        "Tidy this week's notes into the usual sections.",
    ),
    (
        WORKSPACER,
        "Settings search audit",
        "claude",
        "sonnet",
        Live::Idle,
        3 * 60 + 10,
        "Check that every setting is reachable from the settings search.",
    ),
    (
        INFRA,
        "Terraform drift check",
        "claude",
        "sonnet",
        Live::Idle,
        5 * 60,
        "Run a drift check on staging and explain any differences.",
    ),
    (
        NATIVE,
        "Jobs page empty state",
        "claude",
        "sonnet",
        Live::Ended,
        7 * 60,
        "Design the Jobs page for someone with no jobs yet.",
    ),
    (
        HUB,
        "Reconnect storm repro",
        "codex",
        "gpt-5.6-sol",
        Live::Ended,
        9 * 60,
        "Reproduce the reconnect storm when the hub restarts under load.",
    ),
    (
        WORKSPACER,
        "Usage meter rounding",
        "claude",
        "sonnet",
        Live::Ended,
        11 * 60,
        "The usage meter shows 100% while the window still has room.",
    ),
    (
        NATIVE,
        "Keyboard focus after dialogs",
        "claude",
        "sonnet",
        Live::Ended,
        14 * 60,
        "Focus is lost after closing a dialog with Escape.",
    ),
    (
        HUB,
        "Queue priority ordering",
        "codex",
        "gpt-5.6-sol",
        Live::Ended,
        20 * 60,
        "Queued work with the same priority runs out of order.",
    ),
    (
        INFRA,
        "Rotate staging certificates",
        "claude",
        "opus",
        Live::Ended,
        23 * 60,
        "Rotate the staging TLS certificates before they expire.",
    ),
    (
        WORKSPACER,
        "Changelog link checker",
        "claude",
        "haiku",
        Live::Ended,
        26 * 60,
        "Find dead links in CHANGELOG.md.",
    ),
    (
        NOTES,
        "1:1 agenda for Thursday",
        "claude",
        "haiku",
        Live::Ended,
        28 * 60,
        "Draft Thursday's 1:1 agenda from this week's notes.",
    ),
    (
        NATIVE,
        "Title bar double-click on Linux",
        "claude",
        "sonnet",
        Live::Ended,
        31 * 60,
        "Double-clicking the title bar should maximise on Linux.",
    ),
    (
        HUB,
        "Scratch pending counts",
        "codex",
        "gpt-5.6-sol",
        Live::Ended,
        34 * 60,
        "scratch.pending counts files that were already reviewed.",
    ),
    (
        WORKSPACER,
        "Auto-title prompt tuning",
        "claude",
        "sonnet",
        Live::Ended,
        40 * 60,
        "Auto titles are too long. Tune the prompt.",
    ),
    (
        INFRA,
        "Grafana alert thresholds",
        "claude",
        "sonnet",
        Live::Ended,
        46 * 60,
        "The hub latency alert fires on every deploy. Fix the thresholds.",
    ),
    (
        NATIVE,
        "Terminal paste bracketing",
        "claude",
        "sonnet",
        Live::Ended,
        52 * 60,
        "Multi-line paste into the terminal runs each line.",
    ),
    (
        HUB,
        "Session store vacuum",
        "codex",
        "gpt-5.6-sol",
        Live::Ended,
        60 * 60,
        "The session database keeps growing. Vacuum it safely.",
    ),
    (
        WORKSPACER,
        "Model picker search",
        "claude",
        "sonnet",
        Live::Ended,
        70 * 60,
        "Let the model picker filter as you type.",
    ),
    (
        NOTES,
        "Conference talk outline",
        "claude",
        "opus",
        Live::Ended,
        80 * 60,
        "Outline a 20-minute talk on supervising coding agents.",
    ),
    (
        NATIVE,
        "Font fallback for emoji",
        "claude",
        "sonnet",
        Live::Ended,
        90 * 60,
        "Emoji in session titles render as boxes.",
    ),
    (
        INFRA,
        "Backup restore drill",
        "claude",
        "sonnet",
        Live::Ended,
        100 * 60,
        "Restore last night's backup into a scratch database and verify it.",
    ),
    (
        HUB,
        "Peer token expiry warning",
        "codex",
        "gpt-5.6-sol",
        Live::Ended,
        110 * 60,
        "Warn before a peer's token expires.",
    ),
    (
        WORKSPACER,
        "Onboarding checklist copy",
        "claude",
        "sonnet",
        Live::Ended,
        118 * 60,
        "Rewrite the first-run checklist for someone new.",
    ),
];

/// How many live sessions the catalog's story holds (the rig serves this many).
pub const LIVE_SESSIONS: usize = LIVE.len();

fn live_row(index: usize) -> &'static LiveRow {
    &LIVE[index % LIVE.len()]
}

pub fn live_cwd(index: usize) -> &'static str {
    live_row(index).0
}

pub fn live_label(index: usize) -> String {
    let label = live_row(index).1;
    match index / LIVE.len() {
        0 => label.to_owned(),
        round => format!("{label} · {}", round + 1),
    }
}

/// `(provider, model)` of a live session.
pub fn live_agent(index: usize) -> (&'static str, &'static str) {
    let row = live_row(index);
    (row.2, row.3)
}

pub fn live_state(index: usize) -> Live {
    live_row(index).4
}

/// When the session last did anything (epoch ms), counted back from `now`.
pub fn live_activity(index: usize, now: i64) -> i64 {
    now - live_row(index).5 * MINUTE - (index / LIVE.len()) as i64 * DAY
}

/// The request that opened the session.
pub fn live_prompt(index: usize) -> &'static str {
    live_row(index).6
}

fn wall_now() -> i64 {
    chrono::Utc::now().timestamp_millis()
}

fn rfc3339(ms: i64) -> String {
    chrono::DateTime::from_timestamp_millis(ms)
        .map(|t| t.to_rfc3339_opts(chrono::SecondsFormat::Secs, true))
        .unwrap_or_default()
}

fn text<'a>(params: &'a Value, key: &str) -> &'a str {
    params[key].as_str().map(str::trim).unwrap_or("")
}

/// `config.projects`: the registry the Projects page reads, four pinned.
pub fn projects(now: i64) -> Value {
    json!({
        WORKSPACER: {"label": "workspacer", "favourite": true, "lastOpened": now - 40 * MINUTE,
            "color": "#5b8def", "git": {"provider": "github"}},
        NATIVE: {"label": "Native client", "icon": "🦀", "favourite": true,
            "lastOpened": now - 2 * HOUR},
        HUB: {"label": "hub-rs", "favourite": true, "lastOpened": now - 26 * HOUR,
            "color": "#e0794f"},
        NOTES: {"label": "Notes", "icon": "📝", "favourite": true, "lastOpened": now - 3 * DAY},
        INFRA: {"label": "Infra", "lastOpened": now - 6 * DAY, "color": "#9b6bd6"},
    })
}

/// One fixture scratch file: its repository, the hub's list row and its text.
struct ScratchFile {
    root: &'static str,
    item: Value,
    content: &'static str,
}

/// A seeded scratch file: `(root, path, hours ago, purpose, status, keep,
/// sensitive, note, session, content)`; no purpose = written straight into
/// the directory by an agent (`indexed:false`).
type Seed = (
    &'static str,
    &'static str,
    i64,
    Option<&'static str>,
    Option<&'static str>,
    bool,
    bool,
    Option<&'static str>,
    Option<&'static str>,
    &'static str,
);

const SCRATCH: [Seed; 12] = [
    (
        WORKSPACER,
        "prod/backfill-session-titles.sql",
        3,
        Some("Backfill auto titles for sessions created before 0.9"),
        Some("pending"),
        true,
        true,
        None,
        Some("hist-04"),
        "-- Run by hand against the hub's session store (prod). Safe to re-run.\nBEGIN;\nCREATE TABLE IF NOT EXISTS sessions_titles_backup AS\n  SELECT session_id, title FROM sessions;\nUPDATE sessions\n   SET title = substr(first_prompt, 1, 60)\n WHERE (title IS NULL OR title = '')\n   AND first_prompt IS NOT NULL\n   AND started_at < '2026-10-01';\nCOMMIT;\n",
    ),
    (
        WORKSPACER,
        "repro/sidebar-flicker.sh",
        6,
        Some("Repro for the sidebar flicker on reconnect"),
        Some("draft"),
        true,
        false,
        Some("Restart the hub between runs or the cache hides it."),
        None,
        "#!/usr/bin/env bash\n# Drop and restore the bus connection every 2s while the sidebar is open.\nset -euo pipefail\nfor i in $(seq 1 20); do\n  sudo iptables -A OUTPUT -p tcp --dport 7895 -j DROP\n  sleep 2\n  sudo iptables -D OUTPUT -p tcp --dport 7895 -j DROP\n  sleep 2\n  echo \"cycle $i\"\ndone\n",
    ),
    (
        WORKSPACER,
        "notes/native-ui-pass.md",
        1,
        None,
        None,
        false,
        false,
        None,
        None,
        "# Native UI pass\n\n- [x] Project cards: running/total counts\n- [ ] History: date headers wrap at narrow widths\n- [ ] Jobs: failed run detail is cut at one line\n- [ ] Scratch: pending badge contrast on Light\n",
    ),
    (
        WORKSPACER,
        "bench/switch-results.json",
        50,
        Some("Parent/child switch timings for the PR"),
        Some("done"),
        false,
        false,
        Some("Numbers went into the PR description."),
        None,
        "{\n  \"cycles\": 20,\n  \"parent_to_new_child\": {\"shown_p50_ms\": 38.2, \"shown_p95_ms\": 51.0},\n  \"child_to_parent\": {\"shown_p50_ms\": 34.9, \"shown_p95_ms\": 47.3}\n}\n",
    ),
    (
        WORKSPACER,
        "old/migrate-config.py",
        300,
        Some("One-off config.yaml migration"),
        Some("archived"),
        false,
        false,
        None,
        None,
        "import yaml, pathlib\n\npath = pathlib.Path.home() / '.workspacer' / 'config.yaml'\nconfig = yaml.safe_load(path.read_text())\nconfig.setdefault('projects', {})\nfor fav in config.get('directories', {}).get('favourites', []):\n    config['projects'].setdefault(fav, {})['favourite'] = True\npath.write_text(yaml.safe_dump(config))\n",
    ),
    (
        HUB,
        "load/bus-fanout.ts",
        20,
        Some("Load test the bus fan-out before the release"),
        Some("pending"),
        true,
        false,
        Some("Staging only. Prod has the rate limiter off."),
        Some("hist-02"),
        "import WebSocket from 'ws';\n\nconst url = process.env.BUS ?? 'ws://127.0.0.1:7895/bus';\nconst clients = Array.from({ length: 200 }, () => new WebSocket(url));\nfor (const ws of clients) {\n  ws.on('open', () => ws.send(JSON.stringify({ op: 'subscribe', topics: ['agent.snapshot'] })));\n}\n",
    ),
    (
        HUB,
        "sql/jobs-history-cleanup.sql",
        75,
        Some("Trim job history older than 90 days"),
        Some("draft"),
        false,
        false,
        None,
        None,
        "DELETE FROM job_runs WHERE started_at < strftime('%s','now','-90 days') * 1000;\nVACUUM;\n",
    ),
    (
        HUB,
        "curl/federation-handshake.sh",
        120,
        Some("Manual federation handshake check"),
        Some("done"),
        false,
        false,
        Some("Handshake fixed in #482."),
        None,
        "curl -fsS -H \"Authorization: Bearer $PEER_TOKEN\" https://peer.internal:7895/federation/hello | jq .\n",
    ),
    (
        INFRA,
        "deploy/rotate-fly-token.sh",
        30,
        Some("Rotate the Fly.io deploy token by hand"),
        Some("pending"),
        true,
        true,
        None,
        Some("hist-08"),
        "#!/usr/bin/env bash\nset -euo pipefail\nfly tokens create deploy --app wks-hub --expiry 720h > /tmp/fly-token\ngh secret set FLY_API_TOKEN < /tmp/fly-token\nshred -u /tmp/fly-token\n",
    ),
    (
        INFRA,
        "terraform/plan-2026-10-30.txt",
        72,
        None,
        None,
        false,
        false,
        None,
        None,
        "Terraform will perform the following actions:\n\n  # fly_machine.hub will be updated in-place\n  ~ resource \"fly_machine\" \"hub\" {\n      ~ image = \"registry.fly.io/wks-hub:0.8.4\" -> \"registry.fly.io/wks-hub:0.9.0\"\n    }\n\nPlan: 0 to add, 1 to change, 0 to destroy.\n",
    ),
    (
        NOTES,
        "drafts/release-0.9.md",
        52,
        Some("Release notes for 0.9"),
        Some("draft"),
        true,
        false,
        None,
        Some("hist-10"),
        "# 0.9\n\n## Highlights\n\n- Projects can group several folders.\n- Scratch: pending files show on every project card.\n- Jobs: proposals wait for your approval.\n",
    ),
    (
        NOTES,
        "todo/review-replies.md",
        9,
        Some("Review replies to post by hand"),
        Some("pending"),
        false,
        false,
        None,
        None,
        "- #512: agree on the cursor format; will rename `before` to `cursor`.\n- #515: the empty state copy is intentional, see the design note.\n",
    ),
];

/// Ended sessions in History: `(id, provider, model, title, folder, hours
/// ago, prompts, tool calls, hidden, first prompt)`.
type Past = (
    &'static str,
    &'static str,
    &'static str,
    &'static str,
    &'static str,
    f64,
    u64,
    u64,
    bool,
    &'static str,
);

const PAST: [Past; 25] = [
    (
        "hist-01",
        "claude",
        "sonnet",
        "Sidebar polish pass",
        "/home/dev/.workspacer/worktrees/workspacer/sidebar-polish",
        0.6,
        6,
        41,
        false,
        "Tighten the sidebar row spacing and fix the hover state.",
    ),
    (
        "hist-02",
        "codex",
        "gpt-5.6-sol",
        "Bus reconnect jitter",
        HUB,
        1.5,
        4,
        27,
        false,
        "Add jitter to the bus reconnect backoff.",
    ),
    (
        "hist-03",
        "claude",
        "opus",
        "Flaky watcher test",
        "/home/dev/src/workspacer/.claude/worktrees/flaky-watcher",
        3.,
        9,
        88,
        false,
        "watcher::tests::debounce fails about one run in ten. Find out why.",
    ),
    (
        "hist-04",
        "claude",
        "sonnet",
        "Project cards: running counts",
        NATIVE,
        5.,
        5,
        36,
        false,
        "Project cards should say how many sessions are running.",
    ),
    (
        "hist-05",
        "claude",
        "haiku",
        "Standup notes",
        NOTES,
        8.,
        2,
        4,
        false,
        "Summarise yesterday's merged PRs for standup.",
    ),
    (
        "hist-06",
        "codex",
        "gpt-5.6-sol",
        "Jobs history pagination",
        "/home/dev/src/hub-rs/src/services",
        22.,
        7,
        52,
        false,
        "jobs.history returns everything at once; page it.",
    ),
    (
        "hist-07",
        "claude",
        "sonnet",
        "Theme contrast audit",
        NATIVE,
        26.,
        3,
        19,
        false,
        "Check every theme for text below 4.5:1 contrast.",
    ),
    (
        "hist-08",
        "claude",
        "opus",
        "Fly secrets rotation runbook",
        INFRA,
        30.,
        4,
        12,
        false,
        "Write a runbook for rotating the Fly deploy token.",
    ),
    (
        "hist-09",
        "codex",
        "gpt-5.6-sol",
        "Scratch store index repair",
        HUB,
        48.,
        6,
        44,
        false,
        "A damaged scratch index.json hides every file. Make it recover.",
    ),
    (
        "hist-10",
        "claude",
        "sonnet",
        "Release notes draft",
        NOTES,
        52.,
        3,
        9,
        false,
        "Draft the 0.9 release notes from the changelog.",
    ),
    (
        "hist-11",
        "claude",
        "sonnet",
        "Terminal resize glitch",
        NATIVE,
        70.,
        8,
        63,
        false,
        "Resizing the terminal pane leaves a stale last row.",
    ),
    (
        "hist-12",
        "claude",
        "opus",
        "Federation token refresh",
        HUB,
        75.,
        5,
        31,
        false,
        "Peer tokens expire mid-session; refresh them before they do.",
    ),
    (
        "hist-13",
        "claude",
        "sonnet",
        "Quick repro: ws close codes",
        "/tmp/repro-ws-close",
        96.,
        2,
        7,
        true,
        "Which close code does tungstenite send on a dropped TCP connection?",
    ),
    (
        "hist-14",
        "codex",
        "gpt-5.6-sol",
        "Migrate config loader",
        WORKSPACER,
        100.,
        6,
        49,
        false,
        "Move the config loader to the shared crate.",
    ),
    (
        "hist-15",
        "claude",
        "sonnet",
        "Profile startup",
        NATIVE,
        120.,
        4,
        22,
        false,
        "Cold start takes 900ms. Profile it.",
    ),
    (
        "hist-16",
        "claude",
        "haiku",
        "Reading list triage",
        NOTES,
        145.,
        1,
        3,
        true,
        "Sort my reading list by topic.",
    ),
    (
        "hist-17",
        "claude",
        "sonnet",
        "Terraform plan review",
        INFRA,
        170.,
        3,
        11,
        false,
        "Review this terraform plan before I apply it.",
    ),
    (
        "hist-18",
        "codex",
        "gpt-5.6-sol",
        "Usage pace calculation",
        WORKSPACER,
        190.,
        5,
        38,
        false,
        "The 7-day pace reads 'overspending' right after a reset.",
    ),
    (
        "hist-19",
        "claude",
        "sonnet",
        "Handoff brief wording",
        "/home/dev/.workspacer/worktrees/workspacer/handoff-copy",
        215.,
        3,
        14,
        false,
        "Make the handoff brief read less like a log dump.",
    ),
    (
        "hist-20",
        "claude",
        "opus",
        "Hub plugin loader",
        HUB,
        240.,
        11,
        97,
        false,
        "Load hub plugins from ~/.workspacer/plugins with a manifest.",
    ),
    (
        "hist-21",
        "claude",
        "sonnet",
        "Changelog for 0.8",
        NOTES,
        260.,
        2,
        6,
        false,
        "Write the 0.8 changelog.",
    ),
    (
        "hist-22",
        "codex",
        "gpt-5.6-sol",
        "Diff viewer word wrap",
        NATIVE,
        290.,
        4,
        26,
        false,
        "Long lines in the diff viewer overflow instead of wrapping.",
    ),
    (
        "hist-23",
        "claude",
        "sonnet",
        "Bootstrap new laptop",
        INFRA,
        310.,
        3,
        18,
        true,
        "Write a bootstrap script for a fresh dev laptop.",
    ),
    (
        "hist-24",
        "claude",
        "sonnet",
        "Session archive semantics",
        WORKSPACER,
        340.,
        6,
        29,
        false,
        "What should Hide do to a running session?",
    ),
    (
        "hist-25",
        "claude",
        "haiku",
        "Try the new picker",
        "/tmp/picker-spike",
        400.,
        1,
        2,
        false,
        "Spike: a fuzzy folder picker.",
    ),
];

const JOB_TESTS: &str = "a41c7e09d2b84f6e9c03b5d1";
const JOB_HEALTH: &str = "5e8d20f1c7a94b3e8d6f0a2c";
const JOB_AUDIT: &str = "c09b4e7a1f2d48c6b5e3a790";
const JOB_PRUNE: &str = "e7f3a5c1b9d04e28a6c4f1b0";

/// One connection's catalog: what the feature pages read and write.
pub struct Catalog {
    /// When the connection opened; seeded times count back from it.
    now: i64,
    groups: Vec<Value>,
    revision: u64,
    scratch: Vec<ScratchFile>,
    jobs: Vec<Value>,
    /// Newest first, as the hub keeps them.
    runs: Vec<Value>,
    next_run: BTreeMap<String, i64>,
    /// Ended sessions, newest first.
    history: Vec<Value>,
    /// Ended live rows deleted from History.
    forgotten: BTreeSet<String>,
    created: u64,
    /// The host work queue (`docs/host-work-queue.md`): this hub's items,
    /// each peer's, the `queue.*` limits, and the next item number.
    queue: Vec<Value>,
    peer_queues: BTreeMap<String, Vec<Value>>,
    limits: Value,
    queue_seq: u64,
    /// Saved project links (`projects.setLink`), as `project-links.json`.
    links: Vec<Value>,
}

impl Catalog {
    pub fn new(now: i64) -> Self {
        let group = |id: &str, name: &str, folders: &[&str], color: &str, icon: &str| {
            json!({"id": id, "name": name, "folders": folders, "color": color, "icon": icon,
                "createdAt": rfc3339(now - 9 * DAY), "updatedAt": rfc3339(now - 2 * DAY)})
        };
        let scratch = SCRATCH
            .iter()
            .map(
                |&(root, path, hours, purpose, status, keep, sensitive, note, session, content)| {
                    let modified = now - hours * HOUR;
                    let indexed = purpose.is_some();
                    let stamp = |ms: i64| indexed.then(|| rfc3339(ms));
                    ScratchFile {
                        root,
                        item: json!({
                            "path": path,
                            "size": content.len(),
                            "modifiedAt": rfc3339(modified),
                            "indexed": indexed,
                            "purpose": purpose,
                            "sessionId": session,
                            "keep": keep,
                            "sensitive": sensitive,
                            "status": status,
                            "statusAt": status.and(stamp(modified)),
                            "note": note,
                            "createdAt": stamp(modified - 2 * HOUR),
                            "updatedAt": stamp(modified),
                        }),
                        content,
                    }
                },
            )
            .collect();
        let history = PAST
            .iter()
            .enumerate()
            .map(
                |(
                    ix,
                    &(id, provider, model, title, cwd, hours, prompts, tools, hidden, prompt),
                )| {
                    let updated = now - (hours * HOUR as f64) as i64;
                    let started = updated - (20 + (ix as i64 * 17) % 90) * MINUTE;
                    json!({
                        "sessionId": id,
                        "provider": provider,
                        "cwd": cwd,
                        "mode": "stopped",
                        "transport": "stream",
                        "archived": false,
                        "updatedAt": updated,
                        "startedAt": started,
                        "endedAt": updated,
                        "name": title,
                        "title": title,
                        "model": model,
                        "hidden": hidden,
                        "onSidebar": false,
                        "transcriptCopy": ix % 7 != 3,
                        "firstPrompt": prompt,
                        "userPrompts": prompts,
                        "toolCalls": tools,
                    })
                },
            )
            .collect();
        let mut catalog = Self {
            now,
            groups: vec![
                group(
                    "p_7c1e5a90d2",
                    "Workspacer",
                    &[WORKSPACER, HUB],
                    "#5b8def",
                    "🧰",
                ),
                group("p_3b9f0e64a1", "Ops", &[INFRA], "#9b6bd6", "🚀"),
            ],
            revision: 3,
            scratch,
            jobs: seed_jobs(now),
            runs: seed_runs(now),
            next_run: BTreeMap::new(),
            history,
            forgotten: BTreeSet::new(),
            created: 0,
            queue: seed_queue(now),
            peer_queues: BTreeMap::from([(PEER_GPU.to_owned(), seed_peer_queue(now))]),
            limits: json!({"maxAgents": 3, "maxShell": 2, "maxCpuPressure": 60,
                "maxMemoryPressure": 25, "minAvailableMemoryMb": 1024, "minFreeDiskMb": 2048}),
            queue_seq: 0,
            links: vec![json!({"localPath": NATIVE, "peer": PEER_MAC,
                "peerPath": MAC_NATIVE, "createdAt": now - 3 * DAY})],
        };
        for job in catalog.jobs.clone() {
            catalog.schedule(&job, now);
        }
        catalog
    }

    // r4-truth (R4.2)
    /// All clear: nothing waits on the user, so the agent-proposed job has
    /// been approved (it is listed, scheduled and enabled like the rest).
    pub fn all_clear(mut self, now: i64) -> Self {
        for job in self.jobs.iter_mut() {
            if !text(job, "proposedBy").is_empty()
                && let Some(map) = job.as_object_mut()
            {
                map.remove("proposedBy");
                map.insert("enabled".into(), Value::Bool(true));
            }
        }
        for job in self.jobs.clone() {
            self.schedule(&job, now);
        }
        self
    }

    /// A first run: the projects are known, but nothing has happened yet:
    /// no scratch waiting for review, no jobs or runs, no history, nothing
    /// queued (so no badge or count reads as pending work).
    pub fn first_run(mut self) -> Self {
        self.scratch.clear();
        self.jobs.clear();
        self.runs.clear();
        self.next_run.clear();
        self.history.clear();
        self.queue.clear();
        self.peer_queues.clear();
        self
    }

    /// The reply to `method`, or `None` when the catalog does not serve it.
    /// `live` is the fixture's `sessions.snapshots` rows.
    pub fn call(&mut self, method: &str, params: &Value, live: &[Value]) -> Option<Value> {
        let result = match method {
            "projectGroups.list" => Ok(self.list_groups()),
            "projectGroups.save" => self.save_group(params),
            "projectGroups.remove" => self.remove_group(params),
            "scratch.pending" => Ok(self.scratch_pending()),
            "scratch.list" => self.scratch_list(params),
            "scratch.read" => self.scratch_read(params),
            "scratch.mark" => self.scratch_mark(params),
            "jobs.list" => Ok(self.jobs_list()),
            "jobs.history" => Ok(self.jobs_history(params)),
            "jobs.upsert" => self.jobs_upsert(params),
            "jobs.remove" => self.jobs_remove(params),
            "jobs.run" => self.jobs_run(params),
            "host.resources" => Ok(self.resources(LOCAL_HUB)),
            "queue.list" => Ok(self.queue_list(params)),
            "queue.targets" => Ok(self.queue_targets(params)),
            "queue.config" => Ok(self.limits.clone()),
            "queue.setConfig" => self.queue_set_config(params),
            "queue.submit" => self.queue_submit(params),
            "queue.cancel" => self.queue_cancel(params),
            "projects.links" => Ok(json!({"links": self.links})),
            "projects.linkMatrix" => Ok(self.link_matrix()),
            "projects.setLink" => self.set_link(params),
            "projects.removeLink" => self.remove_link(params),
            "sessions.history" => self.history(params, live),
            "sessions.forget" => self.forget(params, live),
            _ => return None,
        };
        Some(result.unwrap_or_else(|error| json!({"ok": false, "error": error})))
    }

    // ---- host work queue -------------------------------------------------

    /// A sample: this hub is busy (one agent running, three queued) with its
    /// gate open; the GPU peer's memory pressure has closed its gate. The
    /// laptop has no PSI (macOS): those fields are null, never zero.
    fn resources(&self, host: &str) -> Value {
        let running = |items: &[Value], kind: &str| {
            items
                .iter()
                .filter(|i| i["state"] == "running" && i["kind"] == kind)
                .count()
        };
        let queued = |items: &[Value]| items.iter().filter(|i| i["state"] == "queued").count();
        let local = host == LOCAL_HUB;
        let items = if local {
            &self.queue
        } else {
            self.peer_queues.get(host).map_or(&[][..], Vec::as_slice)
        };
        let limit = |key: &str, default: i64| self.limits[key].as_i64().unwrap_or(default);
        let (cpu, memory, available, gate) = match host {
            LOCAL_HUB => (
                json!(4.2),
                json!(1.3),
                12_288,
                json!({"open": true, "reasons": []}),
            ),
            PEER_GPU => (
                json!(18.0),
                json!(34.0),
                5_900,
                json!({"open": false, "reasons": ["memory pressure 34.0 >= 25.0"]}),
            ),
            _ => (
                Value::Null,
                Value::Null,
                7_400,
                json!({"open": true, "reasons": []}),
            ),
        };
        json!({
            "host": host,
            "sampledAt": self.now - 3_000,
            "cpu": {"cores": if local { 8 } else { 32 }, "load1": 1.4,
                "usagePct": if local { 23.5 } else { 61.0 }, "pressureAvg10": cpu},
            "memory": {"totalMb": if local { 31_800 } else { 128_000 }, "availableMb": available,
                "pressureAvg10": memory},
            "io": {"pressureAvg10": if host == PEER_LAPTOP { Value::Null } else { json!(0.4) }},
            "disk": {"path": "/home/dev/.config/workspacer-hub", "freeMb": 80_000, "totalMb": 500_000},
            "queue": {
                "agentSlots": if local { limit("maxAgents", 4) } else { 2 },
                "agentsRunning": running(items, "spawn"),
                "shellSlots": if local { limit("maxShell", 2) } else { 1 },
                "shellRunning": running(items, "shell"),
                "queued": queued(items),
                "gate": gate,
            },
        })
    }

    fn queue_list(&self, params: &Value) -> Value {
        let peers: Vec<Value> = if params["includePeers"] == true {
            let mut peers: Vec<Value> = self
                .peer_queues
                .iter()
                .map(|(peer, items)| json!({"peer": peer, "ok": true, "items": items}))
                .collect();
            peers.push(json!({"peer": PEER_LAPTOP, "ok": false,
                "error": "hub:laptop/queue.list timed out after 10s", "items": []}));
            peers
        } else {
            Vec::new()
        };
        json!({"host": LOCAL_HUB, "items": self.queue, "peers": peers})
    }

    fn queue_targets(&self, params: &Value) -> Value {
        let provider = text(params, "provider");
        let cwd = text(params, "cwd");
        let providers = |codex: bool| {
            json!([{"provider": "claude", "found": true, "authenticated": true},
                   {"provider": "codex", "found": codex, "authenticated": codex}])
        };
        let gpu_ok = provider != "codex";
        let mut targets = vec![
            json!({"host": "local", "local": true, "connected": true, "dispatch": true,
             "resources": self.resources(LOCAL_HUB), "providers": providers(true),
             "cwds": local_dirs(), "eligible": true, "score": 3.0, "reasons": []}),
            json!({"host": PEER_GPU, "local": false, "connected": true, "dispatch": true,
             "resources": self.resources(PEER_GPU), "providers": providers(false),
             "cwds": peer_dirs(PEER_GPU), "eligible": gpu_ok, "score": 1.0,
             "reasons": if gpu_ok { json!([]) } else { json!(["codex is not installed"]) }}),
            json!({"host": PEER_MAC, "local": false, "connected": true, "dispatch": true,
             "resources": self.resources(PEER_MAC), "providers": providers(true),
             "cwds": peer_dirs(PEER_MAC), "eligible": true, "score": 2.0, "reasons": []}),
            json!({"host": PEER_LAPTOP, "local": false, "connected": false, "dispatch": true,
             "resources": null, "providers": [], "cwds": [], "eligible": false, "score": 0,
             "reasons": ["not connected"]}),
        ];
        if !cwd.is_empty() {
            for target in &mut targets {
                if target["local"] == true {
                    target["resolvedCwd"] = json!(cwd);
                    target["resolvedVia"] = json!("exact");
                    target["suggestions"] = json!([]);
                    continue;
                }
                let peer = target["host"].as_str().unwrap_or_default().to_owned();
                let (resolved, suggestions) = self.resolve(cwd, &peer);
                target["resolvedCwd"] = json!(resolved.as_ref().map(|r| &r.0));
                target["resolvedVia"] = json!(resolved.as_ref().map(|r| r.1));
                target["suggestions"] = json!(suggestions);
                if resolved.is_none() && target["eligible"] == true {
                    target["eligible"] = json!(false);
                    target["score"] = json!(0);
                    target["reasons"] = json!([no_match(cwd, &peer, &suggestions)]);
                }
            }
        }
        json!({"targets": targets, "best": "local"})
    }

    /// The contract's resolution of local `cwd` for `peer`: a saved link
    /// still offered there, the same path, the one folder with the same
    /// remote; else suggestions by name and ambiguous remote.
    fn resolve(&self, cwd: &str, peer: &str) -> (Option<(String, &'static str)>, Vec<Value>) {
        let choices = peer_dirs(peer);
        let choices = choices.as_array().map(Vec::as_slice).unwrap_or_default();
        let offered = |path: &str| choices.iter().any(|c| c["path"] == path);
        if let Some(link) = self
            .links
            .iter()
            .find(|l| l["localPath"] == cwd && l["peer"] == peer)
            && let Some(path) = link["peerPath"].as_str()
            && offered(path)
        {
            return (Some((path.to_owned(), "link")), Vec::new());
        }
        if offered(cwd) {
            return (Some((cwd.to_owned(), "exact")), Vec::new());
        }
        let local = local_dirs();
        let local = local
            .as_array()
            .and_then(|dirs| dirs.iter().find(|d| d["path"] == cwd).cloned())
            .unwrap_or(Value::Null);
        let remote = local["remote"].as_str().filter(|r| !r.is_empty());
        let same_remote: Vec<&Value> = choices
            .iter()
            .filter(|c| remote.is_some() && c["remote"].as_str() == remote)
            .collect();
        if let [only] = same_remote.as_slice() {
            return (Some((text(only, "path").to_owned(), "remote")), Vec::new());
        }
        let name = crate::project_links::basename(cwd);
        let mut suggestions: Vec<Value> = choices
            .iter()
            .filter(|c| c["name"] == name.as_str())
            .map(|c| json!({"path": c["path"], "why": "name"}))
            .collect();
        suggestions.extend(
            same_remote
                .iter()
                .map(|c| json!({"path": c["path"], "why": "remote"})),
        );
        (None, suggestions)
    }

    /// Every local project against every peer.
    fn link_matrix(&self) -> Value {
        let projects: Vec<Value> = local_dirs()
            .as_array()
            .into_iter()
            .flatten()
            .map(|dir| {
                let path = text(dir, "path");
                let peers: Vec<Value> = [PEER_GPU, PEER_MAC, PEER_LAPTOP]
                    .into_iter()
                    .map(|peer| {
                        let (resolved, suggestions) = self.resolve(path, peer);
                        json!({"peer": peer, "connected": peer != PEER_LAPTOP,
                            "resolved": resolved.map(|(path, via)| json!({"path": path, "via": via})),
                            "suggestions": suggestions, "choices": peer_dirs(peer)})
                    })
                    .collect();
                json!({"path": path, "name": dir["name"], "remote": dir["remote"], "peers": peers})
            })
            .collect();
        json!({"projects": projects})
    }

    fn set_link(&mut self, params: &Value) -> Result<Value, String> {
        let (local, peer, peer_path) = (
            text(params, "localPath"),
            text(params, "peer"),
            text(params, "peerPath"),
        );
        if local.is_empty() || peer.is_empty() {
            return Err("projects.setLink needs localPath and peer".into());
        }
        if !peer_path.starts_with('/') {
            return Err("peerPath must be absolute".into());
        }
        if ![PEER_GPU, PEER_MAC, PEER_LAPTOP].contains(&peer) {
            return Err(format!("unknown peer {peer}"));
        }
        self.links
            .retain(|l| !(l["localPath"] == local && l["peer"] == peer));
        self.links.push(
            json!({"localPath": local, "peer": peer, "peerPath": peer_path,
            "createdAt": wall_now()}),
        );
        Ok(json!({"links": self.links}))
    }

    fn remove_link(&mut self, params: &Value) -> Result<Value, String> {
        let (local, peer) = (text(params, "localPath"), text(params, "peer"));
        self.links
            .retain(|l| !(l["localPath"] == local && l["peer"] == peer));
        Ok(json!({"links": self.links}))
    }

    fn queue_set_config(&mut self, params: &Value) -> Result<Value, String> {
        let Some(patch) = params.as_object() else {
            return Err("queue.setConfig takes an object".into());
        };
        for (key, value) in patch {
            if self.limits.get(key).is_none() {
                return Err(format!("unknown queue setting {key}"));
            }
            if !value.as_f64().is_some_and(|v| v >= 0.) {
                return Err(format!("{key} must be a non-negative number"));
            }
            self.limits[key] = value.clone();
        }
        Ok(self.limits.clone())
    }

    /// Spawns only (shell items are owner host code, out of scope here):
    /// queued here, or on the GPU peer; `any` lands here (best fit).
    fn queue_submit(&mut self, params: &Value) -> Result<Value, String> {
        if params["kind"] != "spawn" || !params["spawn"].is_object() {
            return Err("the fixture queues spawn items only".into());
        }
        let host = match text(params, "host") {
            "" | "local" | "any" => LOCAL_HUB,
            PEER_GPU => PEER_GPU,
            PEER_MAC => PEER_MAC,
            PEER_LAPTOP => return Err("peer laptop is not connected".into()),
            other => return Err(format!("unknown peer {other}")),
        };
        // A peer gets the translated folder; the local one is kept.
        let mut spawn = params["spawn"].clone();
        let mut origin_cwd = Value::Null;
        if host != LOCAL_HUB {
            let cwd = text(&spawn, "cwd").to_owned();
            match self.resolve(&cwd, host) {
                (Some((path, _)), _) => {
                    if path != cwd {
                        origin_cwd = json!(cwd);
                    }
                    spawn["cwd"] = json!(path);
                }
                (None, suggestions) => return Err(no_match(&cwd, host, &suggestions)),
            }
        }
        self.queue_seq += 1;
        let item = json!({
            "id": format!("q_fixture{:03}", self.queue_seq),
            "kind": "spawn",
            "label": text(params, "label"),
            "priority": params["priority"].as_i64().unwrap_or(0),
            "state": "queued",
            "source": {"kind": if host == LOCAL_HUB { "prompt" } else { "remote" },
                "origin": if host == LOCAL_HUB { "" } else { LOCAL_HUB },
                "originCwd": origin_cwd},
            "spawn": spawn,
            "enqueuedAt": wall_now(), "startedAt": 0, "finishedAt": 0,
            "exitCode": null, "detail": "", "waitReason": "no free agent slot",
        });
        if host == LOCAL_HUB {
            self.queue.push(item.clone());
        } else {
            self.peer_queues
                .entry(host.to_owned())
                .or_default()
                .push(item.clone());
        }
        Ok(json!({"host": if host == LOCAL_HUB { "local" } else { host }, "item": item}))
    }

    fn queue_cancel(&mut self, params: &Value) -> Result<Value, String> {
        let id = text(params, "id");
        let items = match text(params, "host") {
            "" | "local" => &mut self.queue,
            peer => self
                .peer_queues
                .get_mut(peer)
                .ok_or_else(|| format!("unknown peer {peer}"))?,
        };
        let item = items
            .iter_mut()
            .find(|i| i["id"] == id)
            .ok_or_else(|| format!("no queue item {id}"))?;
        if item["state"] != "queued" {
            return Err(format!(
                "item {id} is {}; only queued items can be cancelled",
                item["state"].as_str().unwrap_or("?")
            ));
        }
        item["state"] = json!("cancelled");
        item["finishedAt"] = json!(wall_now());
        item["waitReason"] = json!("");
        Ok(json!({"ok": true, "item": item}))
    }

    // ---- project groups ---------------------------------------------------

    /// By name, case-insensitively, then by id (the hub's order).
    fn list_groups(&self) -> Value {
        let mut groups = self.groups.clone();
        groups.sort_by(|a, b| {
            let name = |g: &Value| g["name"].as_str().unwrap_or_default().to_lowercase();
            name(a)
                .cmp(&name(b))
                .then_with(|| a["id"].as_str().cmp(&b["id"].as_str()))
        });
        json!({"revision": self.revision, "groups": groups})
    }

    fn check_revision(&self, params: &Value) -> Result<(), String> {
        match params["revision"].as_u64() {
            Some(expected) if expected != self.revision => Err(format!(
                "conflict: project groups changed (now revision {}, this edit was made against {expected}); reload and try again",
                self.revision
            )),
            _ => Ok(()),
        }
    }

    fn save_group(&mut self, params: &Value) -> Result<Value, String> {
        let name = text(params, "name");
        if name.is_empty() {
            return Err("name is required".into());
        }
        let folders: Vec<String> = params["folders"]
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(Value::as_str)
            .map(projects::project_key)
            .filter(|f| !f.is_empty())
            .collect();
        if folders.is_empty() {
            return Err("folders is required: at least one absolute folder".into());
        }
        if let Some(folder) = folders
            .iter()
            .find(|f| !crate::launch::absolute_directory(f))
        {
            return Err(format!("{folder} is not an absolute folder"));
        }
        self.check_revision(params)?;
        let id = Some(text(params, "id")).filter(|id| !id.is_empty());
        let existing = id.map(|id| self.groups.iter().position(|g| g["id"] == id));
        if let (Some(id), Some(None)) = (id, existing) {
            return Err(format!(
                "there is no project {id}; leave id out to create a new one"
            ));
        }
        for folder in &folders {
            if let Some(other) = self.groups.iter().find(|g| {
                g["id"].as_str() != id
                    && g["folders"]
                        .as_array()
                        .into_iter()
                        .flatten()
                        .filter_map(Value::as_str)
                        .any(|f| same_dir(f, folder))
            }) {
                let other = other["name"].as_str().unwrap_or_default();
                return Err(format!(
                    "{folder} is already in the project “{other}”; remove it there first"
                ));
            }
        }
        let stamp = rfc3339(wall_now());
        let optional = |key: &str| Some(text(params, key)).filter(|v| !v.is_empty());
        let mut group = json!({
            "id": id.map(str::to_owned).unwrap_or_else(|| {
                self.created += 1;
                format!("p_f1c0{:06x}", self.created)
            }),
            "name": name,
            "folders": folders,
            "color": optional("color").map(str::to_ascii_lowercase),
            "icon": optional("icon"),
            "createdAt": stamp,
            "updatedAt": stamp,
        });
        match existing.flatten() {
            Some(ix) => {
                group["createdAt"] = self.groups[ix]["createdAt"].clone();
                self.groups[ix] = group.clone();
            }
            None => self.groups.push(group.clone()),
        }
        self.revision += 1;
        group["revision"] = json!(self.revision);
        Ok(group)
    }

    fn remove_group(&mut self, params: &Value) -> Result<Value, String> {
        let id = text(params, "id");
        if id.is_empty() {
            return Err("id is required: the project to remove".into());
        }
        self.check_revision(params)?;
        let before = self.groups.len();
        self.groups.retain(|g| g["id"] != id);
        if self.groups.len() == before {
            return Err(format!("there is no project {id}"));
        }
        self.revision += 1;
        Ok(json!({"removed": true, "revision": self.revision}))
    }

    // ---- scratch ----------------------------------------------------------

    fn scratch_pending(&self) -> Value {
        let mut rows: Vec<Value> = SCRATCH_ROOTS
            .iter()
            .filter_map(|root| {
                let pending = self
                    .scratch
                    .iter()
                    .filter(|f| f.root == *root && f.item["status"] == "pending")
                    .count();
                (pending > 0)
                    .then(|| json!({"key": scratch_key(root), "root": root, "pending": pending}))
            })
            .collect();
        rows.sort_by(|a, b| a["key"].as_str().cmp(&b["key"].as_str()));
        json!({"projects": rows, "truncated": false})
    }

    fn scratch_list(&self, params: &Value) -> Result<Value, String> {
        let cwd = scratch_cwd(params)?;
        let Some(root) = scratch_root(cwd) else {
            // A folder with no scratch yet: the hub answers an empty listing.
            return Ok(json!({
                "project": describe(&projects::project_key(cwd), false),
                "items": [],
                "truncated": false,
            }));
        };
        let mut items: Vec<Value> = self
            .scratch
            .iter()
            .filter(|f| f.root == root)
            .map(|f| f.item.clone())
            .collect();
        items.sort_by(|a, b| a["path"].as_str().cmp(&b["path"].as_str()));
        Ok(json!({"project": describe(root, true), "items": items, "truncated": false}))
    }

    fn scratch_file(&mut self, params: &Value) -> Result<&mut ScratchFile, String> {
        let cwd = scratch_cwd(params)?;
        let path = text(params, "path");
        if path.is_empty() {
            return Err("path is required: a path relative to the project's scratch".into());
        }
        let root = scratch_root(cwd);
        self.scratch
            .iter_mut()
            .find(|f| Some(f.root) == root && f.item["path"] == path)
            .ok_or_else(|| format!("{path} is not a file in this project's scratch"))
    }

    fn scratch_read(&mut self, params: &Value) -> Result<Value, String> {
        let file = self.scratch_file(params)?;
        let mut result = file.item.clone();
        result["content"] = json!(file.content);
        Ok(result)
    }

    fn scratch_mark(&mut self, params: &Value) -> Result<Value, String> {
        let purpose = params["purpose"].as_str().map(str::trim);
        if purpose == Some("") {
            return Err("purpose cannot be cleared; give a one-line purpose".into());
        }
        let status = params["status"].as_str();
        if let Some(status) = status
            && !["draft", "pending", "done", "archived"].contains(&status)
        {
            return Err("status must be one of draft, pending, done, archived".into());
        }
        let note = params["note"].as_str().map(str::trim);
        let (keep, sensitive) = (params["keep"].as_bool(), params["sensitive"].as_bool());
        if purpose.is_none()
            && status.is_none()
            && note.is_none()
            && keep.is_none()
            && sensitive.is_none()
        {
            return Err("nothing to mark: pass purpose, keep, sensitive, status or note".into());
        }
        let stamp = rfc3339(wall_now());
        let item = &mut self.scratch_file(params)?.item;
        if let Some(purpose) = purpose {
            item["purpose"] = json!(purpose);
        }
        if let Some(keep) = keep {
            item["keep"] = json!(keep);
        }
        if let Some(sensitive) = sensitive {
            item["sensitive"] = json!(sensitive);
        }
        if let Some(status) = status {
            if item["status"] != status {
                item["statusAt"] = json!(stamp);
            }
            item["status"] = json!(status);
        }
        if let Some(note) = note {
            item["note"] = json!(note);
        }
        if item["indexed"] != true {
            item["indexed"] = json!(true);
            item["createdAt"] = json!(stamp);
        }
        item["updatedAt"] = json!(stamp);
        Ok(item.clone())
    }

    // ---- jobs -------------------------------------------------------------

    fn schedule(&mut self, job: &Value, from: i64) {
        let id = job["id"].as_str().unwrap_or_default().to_owned();
        let trigger = &job["trigger"];
        let next = match trigger["kind"].as_str() {
            _ if job["enabled"] != true || !text(job, "proposedBy").is_empty() => None,
            Some("interval") => {
                Some(from + trigger["everyMinutes"].as_i64().unwrap_or(60) * MINUTE)
            }
            Some("daily") => Some(from + 14 * HOUR + 12 * MINUTE),
            _ => None,
        };
        match next {
            Some(at) => self.next_run.insert(id, at),
            None => self.next_run.remove(&id),
        };
    }

    fn jobs_list(&self) -> Value {
        let jobs: Vec<Value> = self
            .jobs
            .iter()
            .map(|job| {
                let mut row = job.clone();
                let id = job["id"].as_str().unwrap_or_default();
                if let Some(at) = self.next_run.get(id) {
                    row["nextRunAt"] = json!(at);
                }
                if let Some(run) = self.runs.iter().find(|r| r["jobId"] == id) {
                    row["lastRun"] = run.clone();
                }
                row
            })
            .collect();
        json!({"jobs": jobs})
    }

    fn jobs_history(&self, params: &Value) -> Value {
        let id = text(params, "id");
        let runs: Vec<&Value> = self
            .runs
            .iter()
            .filter(|r| id.is_empty() || r["jobId"] == id)
            .collect();
        json!({"runs": runs})
    }

    fn jobs_upsert(&mut self, params: &Value) -> Result<Value, String> {
        let mut job = params.clone();
        let Some(spec) = job.as_object_mut() else {
            return Err("a job is an object".into());
        };
        for live in ["nextRunAt", "running", "lastRun"] {
            spec.remove(live);
        }
        if text(&job, "name").is_empty() {
            return Err("job needs a name".into());
        }
        let now = wall_now();
        let id = text(params, "id").to_owned();
        let proposal = !text(params, "proposedBy").is_empty();
        let replaces = text(params, "replaces").to_owned();
        job["updatedAt"] = json!(now);
        // Approving a proposed change: only the spec moves onto the target,
        // which keeps its id, history and on/off state.
        if !proposal && !replaces.is_empty() {
            if !self
                .jobs
                .iter()
                .any(|j| j["id"] == id.as_str() && !text(j, "proposedBy").is_empty())
            {
                return Err(format!("no proposal {id:?}"));
            }
            let target = self
                .jobs
                .iter_mut()
                .find(|j| j["id"] == replaces.as_str() && text(j, "proposedBy").is_empty())
                .ok_or_else(|| format!("no job {replaces:?}"))?;
            for key in ["name", "trigger", "action", "updatedAt"] {
                target[key] = job[key].clone();
            }
            let updated = target.clone();
            self.jobs.retain(|j| j["id"] != id.as_str());
            self.next_run.remove(&id);
            self.schedule(&updated, now);
            return Ok(updated);
        }
        if !proposal && let Some(spec) = job.as_object_mut() {
            spec.remove("replaces");
        }
        if id.is_empty() {
            self.created += 1;
            job["id"] = json!(format!("f1c0{:020x}", self.created));
            job["createdAt"] = json!(now);
            self.jobs.push(job.clone());
        } else {
            let existing = self
                .jobs
                .iter_mut()
                .find(|j| j["id"] == id.as_str())
                .ok_or_else(|| format!("no job {id:?}"))?;
            job["createdAt"] = existing["createdAt"].clone();
            *existing = job.clone();
        }
        self.schedule(&job, now);
        Ok(job)
    }

    fn jobs_remove(&mut self, params: &Value) -> Result<Value, String> {
        let id = text(params, "id");
        if id.is_empty() {
            return Err("jobs.remove requires {id}".into());
        }
        self.jobs.retain(|j| {
            j["id"] != id && !(!text(j, "proposedBy").is_empty() && j["replaces"] == id)
        });
        self.runs.retain(|r| r["jobId"] != id);
        self.next_run.remove(id);
        Ok(json!({"ok": true}))
    }

    /// Records a skipped run: the fixture hub never runs anything.
    fn jobs_run(&mut self, params: &Value) -> Result<Value, String> {
        let id = text(params, "id");
        let job = self
            .jobs
            .iter()
            .find(|j| j["id"] == id)
            .ok_or_else(|| format!("no job {id:?}"))?;
        if !text(job, "proposedBy").is_empty() {
            return Err(format!(
                "job {:?} is an unapproved proposal — the user approves it in Jobs first",
                text(job, "name")
            ));
        }
        let now = wall_now();
        self.runs.insert(
            0,
            json!({"jobId": id, "startedAt": now, "finishedAt": now + 300, "status": "skipped",
                "detail": "fixture hub: nothing runs here"}),
        );
        Ok(json!({"started": true}))
    }

    // ---- history ----------------------------------------------------------

    /// History rows for the live sessions (the hub's history holds every
    /// session it has seen, running ones included) and any launched here.
    fn live_history(&self, live: &[Value]) -> Vec<Value> {
        live.iter()
            .enumerate()
            .filter(|(_, row)| {
                let id = row["sessionId"].as_str().unwrap_or_default();
                !self.forgotten.contains(id)
            })
            .map(|(ix, row)| {
                let ix = ix as i64;
                let label = row["label"].as_str().unwrap_or_default();
                let updated = row["lastActivity"]
                    .as_i64()
                    .unwrap_or(self.now - ix * 4 * MINUTE);
                json!({
                    "sessionId": row["sessionId"],
                    "provider": row["provider"].as_str().unwrap_or("claude"),
                    "cwd": row["cwd"],
                    "mode": row["mode"].as_str().unwrap_or("input"),
                    "transport": "stream",
                    "archived": false,
                    "updatedAt": updated,
                    "startedAt": updated - (25 + (ix * 13) % 70) * MINUTE,
                    "name": label,
                    "title": label,
                    "model": row["model"].as_str().unwrap_or_default(),
                    "hidden": false,
                    "onSidebar": true,
                    "transcriptCopy": false,
                    "userPrompts": 3 + ix % 6,
                    "toolCalls": 12 + (7 * ix) % 60,
                })
            })
            .collect()
    }

    /// One page, newest first: `{sessions, next?}` where `next` is the
    /// `<updatedAt>:<sessionId>` cursor of the page's last row.
    fn history(&self, params: &Value, live: &[Value]) -> Result<Value, String> {
        let limit = match &params["limit"] {
            Value::Null => HISTORY_LIMIT,
            value => value
                .as_u64()
                .filter(|n| (1..=HISTORY_LIMIT).contains(n))
                .ok_or_else(|| format!("sessions.history: limit must be 1-{HISTORY_LIMIT}"))?,
        } as usize;
        let limit = limit.min(HISTORY_PAGE);
        let mut rows = self.live_history(live);
        rows.extend(self.history.iter().cloned());
        rows.sort_by(|a, b| {
            b["updatedAt"]
                .as_i64()
                .cmp(&a["updatedAt"].as_i64())
                .then_with(|| a["sessionId"].as_str().cmp(&b["sessionId"].as_str()))
        });
        let query = text(params, "query").to_lowercase();
        if !query.is_empty() {
            rows.retain(|row| {
                let haystack = [
                    "name",
                    "cwd",
                    "firstPrompt",
                    "provider",
                    "model",
                    "sessionId",
                ]
                .iter()
                .filter_map(|key| row[*key].as_str())
                .collect::<Vec<_>>()
                .join("\n")
                .to_lowercase();
                query.split_whitespace().all(|term| haystack.contains(term))
            });
        }
        let start = match Some(text(params, "cursor")).filter(|c| !c.is_empty()) {
            None => 0,
            Some(cursor) => {
                let (at, id) = cursor
                    .split_once(':')
                    .ok_or("sessions.history: invalid cursor")?;
                let at: i64 = at.parse().map_err(|_| "sessions.history: invalid cursor")?;
                rows.iter()
                    .position(|row| row["sessionId"] == id)
                    .map(|ix| ix + 1)
                    .or_else(|| {
                        rows.iter()
                            .position(|row| row["updatedAt"].as_i64() < Some(at))
                    })
                    .unwrap_or(rows.len())
            }
        };
        let end = (start + limit).min(rows.len());
        let page = &rows[start..end];
        let mut reply = json!({"sessions": page});
        if end < rows.len()
            && let Some(last) = page.last()
        {
            reply["next"] = json!(format!(
                "{}:{}",
                last["updatedAt"].as_i64().unwrap_or(0),
                last["sessionId"].as_str().unwrap_or_default()
            ));
        }
        Ok(reply)
    }

    fn forget(&mut self, params: &Value, live: &[Value]) -> Result<Value, String> {
        let id = text(params, "sessionId");
        if id.is_empty() {
            return Err("sessions.forget requires a sessionId".into());
        }
        if let Some(row) = live.iter().find(|row| row["sessionId"] == id) {
            if row["mode"] != "stopped" {
                return Err(
                    "This session is still running; stop it before deleting it from History".into(),
                );
            }
            if !self.forgotten.insert(id.to_owned()) {
                return Err(format!("History has no session {id}"));
            }
            return Ok(json!({"ok": true, "sessionId": id, "transcriptCopyRemoved": false}));
        }
        let ix = self
            .history
            .iter()
            .position(|row| row["sessionId"] == id)
            .ok_or_else(|| format!("History has no session {id}"))?;
        let row = self.history.remove(ix);
        Ok(
            json!({"ok": true, "sessionId": id, "transcriptCopyRemoved": row["transcriptCopy"] == true}),
        )
    }
}

const LOCAL_HUB: &str = "studio";
const PEER_GPU: &str = "gpu-box";
const PEER_LAPTOP: &str = "laptop";
/// A macOS peer: the same projects under `/Users`, none of them at the same
/// path. `hub` is matched by its git remote, `workspacer` only by name, the
/// infra checkouts share one remote (ambiguous), and the native app is
/// linked by hand.
const PEER_MAC: &str = "mac";
const MAC_NATIVE: &str = "/Users/dev/code/workspacer/apps/native";

/// This hub's project folders as directory objects.
fn local_dirs() -> Value {
    json!([
        {"path": WORKSPACER, "name": "workspacer", "remote": "github.com/dev/workspacer",
         "git": true, "source": "project"},
        {"path": NATIVE, "name": "native", "remote": null, "git": true, "source": "live"},
        {"path": HUB, "name": "hub-rs", "remote": "github.com/dev/hub-rs", "git": true,
         "source": "project"},
        {"path": INFRA, "name": "infra", "remote": "gitlab.com/ops/infra", "git": true,
         "source": "project"},
    ])
}

/// What each peer offers. The GPU box has the workspacer checkout under the
/// same remote (an automatic match) and an `infra` folder with no remote (a
/// name-only suggestion).
fn peer_dirs(peer: &str) -> Value {
    match peer {
        PEER_GPU => json!([
            {"path": "/srv/dev/workspacer", "name": "workspacer",
             "remote": "github.com/dev/workspacer", "git": true, "source": "project"},
            {"path": "/srv/dev/models", "name": "models", "remote": "github.com/dev/models",
             "git": true, "source": "project"},
            {"path": "/srv/dev/infra", "name": "infra", "remote": null, "git": false,
             "source": "project"},
        ]),
        PEER_MAC => json!([
            {"path": "/Users/dev/code/hub", "name": "hub", "remote": "github.com/dev/hub-rs",
             "git": true, "source": "project"},
            {"path": "/Users/dev/code/workspacer", "name": "workspacer", "remote": null,
             "git": true, "source": "project"},
            {"path": MAC_NATIVE, "name": "native", "remote": null, "git": true, "source": "live"},
            {"path": "/Users/dev/code/infra-a", "name": "infra-a",
             "remote": "gitlab.com/ops/infra", "git": true, "source": "project"},
            {"path": "/Users/dev/code/infra-b", "name": "infra-b",
             "remote": "gitlab.com/ops/infra", "git": true, "source": "project"},
        ]),
        _ => json!([]),
    }
}

/// The contract's refusal for a folder a peer has no match for.
fn no_match(cwd: &str, peer: &str, suggestions: &[Value]) -> String {
    match suggestions.first().and_then(|s| s["path"].as_str()) {
        Some(path) => format!(
            "cwd {cwd} has no match on {peer} (suggested: {path} — link it in Hosts → Projects)"
        ),
        None => format!("cwd {cwd} has no match on {peer}"),
    }
}

/// This hub's queue: every agent slot taken by the live fleet's three
/// working sessions, two waiting for a slot (one at a higher priority, from
/// the nightly job), a finished shell item and one whose outcome was lost to
/// a restart.
fn seed_queue(now: i64) -> Vec<Value> {
    let spawn = |cwd: &str, prompt: &str| json!({"provider": "claude", "cwd": cwd, "prompt": prompt, "model": "sonnet"});
    // The live rows that are mid-turn, as the running queue items.
    let running = (0..LIVE.len())
        .filter(|&ix| live_state(ix) == Live::Working)
        .enumerate()
        .map(|(n, ix)| {
            let (provider, model) = live_agent(ix);
            let id = if n == 0 {
                "q_01JRUN0000000000000000001".to_owned()
            } else {
                format!("q_01JRUN00000000000000000{:02}", n + 10)
            };
            let started = now - (12 + 9 * n as i64) * MINUTE;
            json!({"id": id, "kind": "spawn", "label": live_label(ix),
                "priority": 0, "state": "running", "source": {"kind": "prompt"},
                "spawn": {"provider": provider, "cwd": live_cwd(ix), "prompt": live_prompt(ix), "model": model},
                "enqueuedAt": started - MINUTE, "startedAt": started, "finishedAt": 0,
                "sessionId": format!("demo-{ix:04}"), "exitCode": null, "detail": "", "waitReason": ""})
        });
    let mut items: Vec<Value> = running.collect();
    items.extend([
        json!({"id": "q_01JQUE0000000000000000002", "kind": "spawn", "label": "Nightly test sweep",
            "priority": 2, "state": "queued", "source": {"kind": "job", "jobId": JOB_TESTS},
            "spawn": spawn(WORKSPACER, "The nightly test run failed. Triage the failures."),
            "enqueuedAt": now - 6 * MINUTE, "startedAt": 0, "finishedAt": 0,
            "exitCode": null, "detail": "", "waitReason": "no free agent slot"}),
        json!({"id": "q_01JQUE0000000000000000003", "kind": "spawn", "label": "",
            "priority": 0, "state": "queued", "source": {"kind": "agent"},
            "spawn": spawn(NATIVE, "Tidy the Hosts view spacing at narrow widths."),
            "enqueuedAt": now - 2 * MINUTE, "startedAt": 0, "finishedAt": 0,
            "exitCode": null, "detail": "", "waitReason": "no free agent slot"}),
        json!({"id": "q_01JDON0000000000000000004", "kind": "shell", "label": "cargo test",
            "priority": 0, "state": "failed", "source": {"kind": "job", "jobId": JOB_HEALTH},
            "shell": {"command": "cargo test --locked", "cwd": HUB, "timeoutSec": 3600},
            "enqueuedAt": now - 3 * HOUR, "startedAt": now - 3 * HOUR, "finishedAt": now - 3 * HOUR + 4 * MINUTE,
            "exitCode": 101, "detail": "test bus::fanout ... FAILED", "waitReason": ""}),
        json!({"id": "q_01JUNK0000000000000000005", "kind": "spawn", "label": "Release notes draft",
            "priority": 0, "state": "unknown", "source": {"kind": "prompt"},
            "spawn": spawn(WORKSPACER, "Draft the 0.9 release notes."),
            "enqueuedAt": now - DAY, "startedAt": now - DAY, "finishedAt": now - DAY + HOUR,
            "exitCode": null, "detail": "hub restarted while running", "waitReason": ""}),
    ]);
    items
}

/// The GPU peer's queue: a remote item from this hub, waiting on its gate.
fn seed_peer_queue(now: i64) -> Vec<Value> {
    vec![
        json!({"id": "q_01JGPU0000000000000000001", "kind": "spawn", "label": "Fine-tune eval",
        "priority": 1, "state": "queued", "source": {"kind": "remote", "origin": LOCAL_HUB},
        "spawn": {"provider": "claude", "cwd": "/srv/dev/models", "prompt": "Run the eval suite."},
        "enqueuedAt": now - 9 * MINUTE, "startedAt": 0, "finishedAt": 0,
        "exitCode": null, "detail": "", "waitReason": "memory pressure 34.0 >= 25.0"}),
    ]
}

fn seed_jobs(now: i64) -> Vec<Value> {
    vec![
        json!({
            "id": JOB_TESTS, "name": "Nightly test sweep", "enabled": true,
            "host": "local", "priority": 2,
            "trigger": {"kind": "daily", "at": "07:00", "days": [1, 2, 3, 4, 5]},
            "action": {"kind": "spawn", "spawn": {
                "cwd": WORKSPACER, "provider": "claude", "model": "sonnet",
                "permissionMode": "acceptEdits",
                "prompt": "The nightly test run failed. Triage the failures below, fix what is clearly a regression, and list anything that looks flaky.\n\n{{output}}",
                "context": [{"kind": "shell",
                    "shell": {"command": "cargo test --locked --workspace 2>&1 | tail -n 200"},
                    "ignoreExitCode": true, "skipUnlessMatch": "FAILED|panicked"}],
            }},
            "createdAt": now - 40 * DAY, "updatedAt": now - 12 * DAY,
        }),
        json!({
            "id": JOB_HEALTH, "name": "Hub health check", "enabled": true,
            "trigger": {"kind": "interval", "everyMinutes": 30},
            "action": {"kind": "shell", "shell": {
                "command": "curl -fsS http://127.0.0.1:7895/health", "cwd": HUB}},
            "createdAt": now - 21 * DAY, "updatedAt": now - 21 * DAY,
        }),
        json!({
            "id": JOB_AUDIT, "name": "Weekly dependency audit", "enabled": false,
            "trigger": {"kind": "daily", "at": "09:30", "days": [1]},
            "action": {"kind": "spawn", "spawn": {
                "cwd": HUB, "provider": "codex", "model": "gpt-5.6-sol", "effort": "medium",
                "prompt": "Review this cargo audit report. Open a PR bumping anything with a fix available; summarise the rest.\n\n{{output}}",
                "context": [{"kind": "shell", "shell": {"command": "cargo audit --json"},
                    "ignoreExitCode": true, "skipIfEmpty": true}],
            }},
            "createdAt": now - 30 * DAY, "updatedAt": now - 5 * DAY,
        }),
        json!({
            "id": JOB_PRUNE, "name": "Prune merged worktrees", "enabled": false,
            "proposedBy": "Release checklist for 0.9",
            "trigger": {"kind": "daily", "at": "18:00"},
            "action": {"kind": "shell", "shell": {
                "command": "git worktree prune && git branch --merged main | grep -v ' main$' | xargs -r git branch -d",
                "cwd": WORKSPACER}},
            "createdAt": now - 50 * MINUTE, "updatedAt": now - 50 * MINUTE,
        }),
    ]
}

/// Runs across the three jobs, newest first: the nightly sweep's latest run
/// failed, the health check had one refused connection, the paused audit's
/// last run found nothing to fix.
fn seed_runs(now: i64) -> Vec<Value> {
    let run = |job: &str, ago: i64, took: i64, status: &str, detail: &str| {
        json!({"jobId": job, "startedAt": now - ago, "finishedAt": now - ago + took,
            "status": status, "detail": detail})
    };
    let mut runs = vec![
        run(
            JOB_HEALTH,
            8 * MINUTE,
            400,
            "ok",
            "{\"ok\":true,\"uptime\":\"3d 4h\"}",
        ),
        run(
            JOB_HEALTH,
            38 * MINUTE,
            380,
            "ok",
            "{\"ok\":true,\"uptime\":\"3d 3h\"}",
        ),
        run(
            JOB_HEALTH,
            68 * MINUTE,
            40,
            "error",
            "exit 7\ncurl: (7) Failed to connect to 127.0.0.1 port 7895 after 0 ms: Connection refused",
        ),
        run(
            JOB_HEALTH,
            98 * MINUTE,
            410,
            "ok",
            "{\"ok\":true,\"uptime\":\"3d 2h\"}",
        ),
        run(
            JOB_TESTS,
            3 * HOUR,
            20 * MINUTE,
            "error",
            "step 1 timed out after 20m: cargo test --locked --workspace 2>&1 | tail -n 200",
        ),
        run(
            JOB_TESTS,
            DAY + 3 * HOUR,
            9 * MINUTE,
            "ok",
            "spawned 6d0f3c1a-8b2e-4f57-9c1d-2a7e5b90c3f4",
        ),
        run(
            JOB_TESTS,
            4 * DAY + 3 * HOUR,
            8 * MINUTE,
            "skipped",
            "step 1 output did not match /FAILED|panicked/",
        ),
        run(
            JOB_AUDIT,
            5 * DAY + 2 * HOUR,
            3 * MINUTE,
            "skipped",
            "step 1 output was empty",
        ),
        run(
            JOB_TESTS,
            5 * DAY + 3 * HOUR,
            11 * MINUTE,
            "ok",
            "spawned 0b7e2d94-51c3-4a8f-b6e0-9f2c4d1a7e35",
        ),
        run(
            JOB_AUDIT,
            12 * DAY + 2 * HOUR,
            6 * MINUTE,
            "ok",
            "spawned 9a4c1e7f-2d3b-4c58-8e6a-1f0b3d5c7a92",
        ),
    ];
    runs.sort_by_key(|r| std::cmp::Reverse(r["startedAt"].as_i64()));
    runs
}

fn scratch_cwd(params: &Value) -> Result<&str, String> {
    Some(text(params, "cwd"))
        .filter(|cwd| !cwd.is_empty())
        .ok_or_else(|| "cwd is required: the absolute working directory of the project".into())
}

/// The repository `cwd` belongs to: the longest scratch root holding it,
/// with a worktree checkout resolved to its repository first.
fn scratch_root(cwd: &str) -> Option<&'static str> {
    let cwd = match projects::worktree_repo(cwd, TREES) {
        Some(WorktreeRepo::Dir(repo)) => repo,
        Some(WorktreeRepo::Named { name, .. }) => {
            return SCRATCH_ROOTS
                .iter()
                .find(|root| projects::basename(root) == name)
                .copied();
        }
        None => cwd.to_owned(),
    };
    SCRATCH_ROOTS
        .iter()
        .filter(|root| within(&cwd, root))
        .max_by_key(|root| root.len())
        .copied()
}

/// claudemon's scratch key: the folder name, then a 64-bit FNV-1a of the
/// whole root.
fn scratch_key(root: &str) -> String {
    let base: String = projects::basename(root)
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '-') {
                c
            } else {
                '-'
            }
        })
        .take(40)
        .collect();
    let hash = root.bytes().fold(0xcbf2_9ce4_8422_2325u64, |h, b| {
        (h ^ u64::from(b)).wrapping_mul(0x0100_0000_01b3)
    });
    format!("{base}-{hash:016x}")
}

fn describe(root: &str, exists: bool) -> Value {
    let key = scratch_key(root);
    let dir = format!("{SCRATCH_BASE}/{key}");
    let mut project = Map::new();
    project.insert("root".into(), json!(root));
    project.insert("key".into(), json!(key));
    project.insert("files".into(), json!(format!("{dir}/files")));
    project.insert("dir".into(), json!(dir));
    project.insert("exists".into(), json!(exists));
    Value::Object(project)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::Session;

    fn live(count: usize) -> Vec<Value> {
        (0..count)
            .map(|i| {
                let (provider, model) = live_agent(i);
                json!({"sessionId": format!("demo-{i:04}"), "label": live_label(i),
                "cwd": live_cwd(i), "provider": provider, "model": model,
                "lastActivity": live_activity(i, wall_now()),
                "mode": match live_state(i) {
                    Live::Ended => "stopped",
                    Live::Working => "responding",
                    _ => "input",
                }})
            })
            .collect()
    }

    fn sessions(rows: &[Value]) -> Vec<Session> {
        rows.iter()
            .map(|row| {
                let mut session = Session::default();
                session.merge(row);
                session
            })
            .collect()
    }

    #[test]
    fn projects_read_as_pinned_folders_with_running_counts() {
        let now = wall_now();
        let config = json!({"projects": projects(now)});
        let rows = live(12);
        let known = projects::list(Some(&projects::registry(&config)), &sessions(&rows), &[]);
        assert_eq!(known.len(), 5, "{known:#?}");
        assert_eq!(known.iter().filter(|p| p.favourite).count(), 4);
        let get = |path: &str| known.iter().find(|p| p.path == path).unwrap();
        let native = get(NATIVE);
        assert_eq!(
            (native.title(), native.mark().as_str()),
            ("Native client", "🦀")
        );
        assert_eq!((native.sessions, native.live_sessions), (3, 2));
        assert_eq!(get(WORKSPACER).color, Some(0x5b8def));
        assert_eq!(get(WORKSPACER).git.provider, "github");
        assert!(
            known.iter().all(|p| p.sessions > 0),
            "every project has sessions"
        );
    }

    #[test]
    fn groups_parse_and_save_remove_in_memory() {
        let mut catalog = Catalog::new(wall_now());
        let list = catalog.call("projectGroups.list", &json!({}), &[]).unwrap();
        let groups = crate::project_groups::parse(&list).unwrap();
        assert_eq!(groups.revision, 3);
        let shape: Vec<_> = groups
            .groups
            .iter()
            .map(|g| {
                (
                    g.name.as_str(),
                    g.folders.len(),
                    g.color.is_some(),
                    g.icon.is_some(),
                )
            })
            .collect();
        assert_eq!(
            shape,
            [("Ops", 1, true, true), ("Workspacer", 2, true, true)]
        );
        let workspacer = groups
            .groups
            .iter()
            .find(|g| g.name == "Workspacer")
            .unwrap();
        assert!(
            workspacer.holds(NATIVE),
            "a subfolder is in its folder's group"
        );

        // A save the editor would send: new project with a free folder.
        let mut draft = crate::project_groups::Draft {
            name: "Notes".into(),
            color: "#E0A84F".into(),
            icon: "📝".into(),
            ..Default::default()
        };
        draft.add_folder(NOTES);
        let saved = catalog
            .call("projectGroups.save", &draft.params(3).unwrap(), &[])
            .unwrap();
        assert_eq!(
            (saved["revision"].as_u64(), saved["color"].as_str()),
            (Some(4), Some("#e0a84f"))
        );
        // A stale revision and a folder another group holds are refused.
        let stale = catalog.call("projectGroups.save", &draft.params(3).unwrap(), &[]);
        assert!(
            stale.unwrap()["error"]
                .as_str()
                .unwrap()
                .contains("conflict")
        );
        draft.folders = vec![HUB.into()];
        let taken = catalog.call("projectGroups.save", &draft.params(4).unwrap(), &[]);
        assert!(
            taken.unwrap()["error"]
                .as_str()
                .unwrap()
                .contains("Workspacer")
        );
        // Renaming keeps the id; removing drops it.
        let mut rename = crate::project_groups::Draft::of(workspacer);
        rename.name = "Workspacer stack".into();
        let renamed = catalog
            .call("projectGroups.save", &rename.params(4).unwrap(), &[])
            .unwrap();
        assert_eq!(renamed["id"], workspacer.id.as_str());
        let removed = catalog
            .call(
                "projectGroups.remove",
                &json!({"id": saved["id"], "revision": 5}),
                &[],
            )
            .unwrap();
        assert_eq!(removed, json!({"removed": true, "revision": 6}));
        let list = catalog.call("projectGroups.list", &json!({}), &[]).unwrap();
        let groups = crate::project_groups::parse(&list).unwrap();
        assert_eq!(groups.groups.len(), 2);
        assert!(groups.groups.iter().any(|g| g.name == "Workspacer stack"));
    }

    #[test]
    fn scratch_lists_reads_and_marks_per_repository() {
        let mut catalog = Catalog::new(wall_now());
        let pending = crate::project_groups::pending_roots(
            &catalog.call("scratch.pending", &json!({}), &[]).unwrap(),
        );
        assert_eq!(pending.len(), 4);
        assert_eq!(
            crate::project_groups::pending_in(&pending, &[NATIVE.to_owned()]),
            1
        );
        assert_eq!(
            crate::project_groups::pending_in(&pending, &[WORKSPACER.into(), HUB.into()]),
            2
        );

        // A subfolder and a worktree list their repository's scratch.
        for cwd in [
            NATIVE,
            "/home/dev/.workspacer/worktrees/workspacer/sidebar-polish",
        ] {
            let value = catalog
                .call("scratch.list", &json!({"cwd": cwd}), &[])
                .unwrap();
            let list = crate::scratch::parse_list(&value).unwrap();
            assert_eq!(list.root, WORKSPACER);
            assert_eq!(list.items.len(), 5);
            assert_eq!(list.pending(), 1);
            assert!(
                list.items
                    .iter()
                    .any(|i| i.status == crate::scratch::Status::Unrecorded)
            );
            assert!(list.key.starts_with("workspacer-") && list.exists);
        }
        let empty = catalog
            .call("scratch.list", &json!({"cwd": "/tmp/x"}), &[])
            .unwrap();
        assert!(crate::scratch::parse_list(&empty).unwrap().items.is_empty());

        let read = catalog
            .call(
                "scratch.read",
                &json!({"cwd": HUB, "path": "load/bus-fanout.ts"}),
                &[],
            )
            .unwrap();
        assert!(matches!(
            crate::scratch::parse_read(&read),
            Some(crate::scratch::Content::Text { text, .. }) if text.contains("WebSocket")
        ));
        let mark = json!({"cwd": HUB, "path": "load/bus-fanout.ts", "status": "done", "note": "ran on staging"});
        let marked = catalog.call("scratch.mark", &mark, &[]).unwrap();
        assert_eq!(
            (marked["status"].as_str(), marked["note"].as_str()),
            (Some("done"), Some("ran on staging"))
        );
        let pending = crate::project_groups::pending_roots(
            &catalog.call("scratch.pending", &json!({}), &[]).unwrap(),
        );
        assert_eq!(pending.len(), 3);
        let nothing = catalog.call(
            "scratch.mark",
            &json!({"cwd": HUB, "path": "load/bus-fanout.ts"}),
            &[],
        );
        assert_eq!(nothing.unwrap()["ok"], false);
    }

    #[test]
    fn hosts_fixture_serves_the_whole_contract() {
        use crate::hosts::{self, Snapshot, Support};
        let mut catalog = Catalog::new(wall_now());
        let mut call = |method: &str, params: Value| catalog.call(method, &params, &[]).unwrap();
        let snapshot = Snapshot::parse(&json!({
            "resources": call("host.resources", json!({})),
            "list": call("queue.list", json!({"includePeers": true})),
            "targets": call("queue.targets", json!({"kind": "spawn"})),
            "config": call("queue.config", json!({})),
        }));
        assert_eq!(snapshot.support(), Support::Supported);
        let cards = hosts::cards(&snapshot, &hosts::Live::default());
        let names: Vec<_> = cards.iter().map(|c| c.host.as_str()).collect();
        assert_eq!(names, ["local", PEER_GPU, PEER_LAPTOP, PEER_MAC]);
        let local = cards[0].resources.as_ref().unwrap();
        assert_eq!(
            local.agent_line(),
            "3/3 agents",
            "the working sessions fill it"
        );
        assert_eq!(local.queued_line(), "2 queued");
        let gpu = cards[1].resources.as_ref().unwrap();
        assert_eq!(gpu.gate_label(), "Gate closed");
        assert!(!cards[2].error.is_empty(), "the laptop timed out");

        // Submit to the peer, cancel it, and refuse to cancel running work.
        let submitted = call(
            "queue.submit",
            json!({"host": PEER_GPU, "kind": "spawn",
            "spawn": {"provider": "claude", "cwd": "/srv/dev/models", "prompt": "go"}}),
        );
        let (host, item) = hosts::parse_submitted(&submitted).unwrap();
        assert_eq!((host.as_str(), item.state.label()), (PEER_GPU, "queued"));
        let cancelled = call("queue.cancel", json!({"id": item.id, "host": PEER_GPU}));
        assert_eq!(cancelled["item"]["state"], "cancelled");
        let refused = call("queue.cancel", json!({"id": "q_01JRUN0000000000000000001"}));
        assert_eq!(refused["ok"], false);
        let limits = call("queue.setConfig", json!({"maxAgents": 6}));
        assert_eq!(limits["maxAgents"], 6);
        assert_eq!(call("queue.setConfig", json!({"nope": 1}))["ok"], false);
        let resources = hosts::Resources::parse(&call("host.resources", json!({}))).unwrap();
        assert_eq!(resources.agent_line(), "3/6 agents");
        let targets = hosts::parse_targets(&call("queue.targets", json!({"provider": "codex"})));
        assert!(!targets.unwrap().find(PEER_GPU).unwrap().eligible);
    }

    #[test]
    fn project_links_fixture_resolves_like_the_contract() {
        use crate::hosts;
        use crate::project_links::{CellState, CellView, Snapshot, Via};
        let mut catalog = Catalog::new(wall_now());
        let mut call = |method: &str, params: Value| catalog.call(method, &params, &[]).unwrap();

        // Old-hub shape still parses: no cwd asked, no resolution fields.
        let plain = hosts::parse_targets(&call("queue.targets", json!({}))).unwrap();
        let gpu = plain.find(PEER_GPU).unwrap();
        assert_eq!(gpu.resolved_cwd, None);
        assert_eq!(
            gpu.cwds[0].remote.as_deref(),
            Some("github.com/dev/workspacer")
        );

        let targets = |call: &mut dyn FnMut(&str, Value) -> Value, cwd: &str| {
            hosts::parse_targets(&call("queue.targets", json!({"cwd": cwd}))).unwrap()
        };
        let ws = targets(&mut call, WORKSPACER);
        assert_eq!(
            ws.find(PEER_GPU).unwrap().resolved_cwd.as_deref(),
            Some("/srv/dev/workspacer")
        );
        assert_eq!(ws.find(PEER_GPU).unwrap().match_label(), Some("git remote"));
        let mac = ws.find(PEER_MAC).unwrap();
        assert_eq!(
            mac.resolved_cwd, None,
            "a name match is never used unlinked"
        );
        assert!(!mac.eligible);
        assert!(
            mac.reasons[0].contains("link it in Hosts → Projects"),
            "{:?}",
            mac.reasons
        );
        assert_eq!(mac.suggestions[0].why, "name");
        assert_eq!(
            targets(&mut call, NATIVE)
                .find(PEER_MAC)
                .unwrap()
                .match_label(),
            Some("linked")
        );
        let infra = targets(&mut call, INFRA);
        assert_eq!(infra.find(PEER_GPU).unwrap().suggestions[0].why, "name");
        let ambiguous = &infra.find(PEER_MAC).unwrap().suggestions;
        assert_eq!(ambiguous.len(), 2);
        assert!(ambiguous.iter().all(|s| s.why == "remote"));

        let matrix = |call: &mut dyn FnMut(&str, Value) -> Value| {
            Snapshot::parse(&json!({"matrix": call("projects.linkMatrix", json!({})),
                "links": call("projects.links", json!({}))}))
        };
        let snapshot = matrix(&mut call);
        let m = snapshot.matrix.as_ref().unwrap();
        assert_eq!(m.peers(), [PEER_GPU, PEER_MAC, PEER_LAPTOP]);
        let cell = |s: &Snapshot, row: usize, col: usize| {
            let m = s.matrix.as_ref().unwrap();
            CellView::new(&m.projects[row], &m.projects[row].peers[col], s.saved())
        };
        assert!(matches!(
            cell(&snapshot, 0, 0).state,
            CellState::Matched {
                via: Via::Remote,
                ..
            }
        ));
        assert!(matches!(
            cell(&snapshot, 0, 1).state,
            CellState::Suggested(_)
        ));
        assert!(cell(&snapshot, 0, 2).offline);
        assert!(matches!(cell(&snapshot, 1, 1).state, CellState::Linked(_)));
        assert!(matches!(
            cell(&snapshot, 2, 1).state,
            CellState::Matched {
                via: Via::Remote,
                ..
            }
        ));
        assert_eq!(cell(&snapshot, 2, 0).state, CellState::Missing);

        // Accept the suggestion; then a named-peer submit translates the cwd.
        let refused = call(
            "projects.setLink",
            json!({"localPath": WORKSPACER, "peer": PEER_MAC, "peerPath": "relative"}),
        );
        assert_eq!(refused["ok"], false);
        let saved = call(
            "projects.setLink",
            json!({"localPath": WORKSPACER, "peer": PEER_MAC,
            "peerPath": "/Users/dev/code/workspacer"}),
        );
        assert_eq!(crate::project_links::parse_links(&saved).unwrap().len(), 2);
        let snapshot = matrix(&mut call);
        assert!(matches!(cell(&snapshot, 0, 1).state, CellState::Linked(_)));
        let submitted = call(
            "queue.submit",
            json!({"host": PEER_MAC, "kind": "spawn",
            "spawn": {"provider": "claude", "cwd": WORKSPACER, "prompt": "go"}}),
        );
        let (host, item) = hosts::parse_submitted(&submitted).unwrap();
        assert_eq!(host, PEER_MAC);
        assert_eq!(item.cwd, "/Users/dev/code/workspacer");
        assert_eq!(item.origin_cwd, WORKSPACER);
        let unmatched = call(
            "queue.submit",
            json!({"host": PEER_GPU, "kind": "spawn",
            "spawn": {"provider": "claude", "cwd": HUB, "prompt": "go"}}),
        );
        assert!(
            unmatched["error"]
                .as_str()
                .unwrap()
                .contains("has no match on gpu-box")
        );
        let removed = call(
            "projects.removeLink",
            json!({"localPath": WORKSPACER, "peer": PEER_MAC}),
        );
        assert_eq!(
            crate::project_links::parse_links(&removed).unwrap().len(),
            1
        );
    }

    #[test]
    fn jobs_mix_states_and_history_pages_by_job() {
        let mut catalog = Catalog::new(wall_now());
        let jobs = crate::jobs::parse_list(&catalog.call("jobs.list", &json!({}), &[]).unwrap());
        assert_eq!(jobs.len(), 4);
        assert!(jobs[0].proposal(), "proposals list first");
        assert_eq!(jobs.iter().filter(|j| j.enabled).count(), 2);
        let tests = jobs.iter().find(|j| j.id == JOB_TESTS).unwrap();
        assert_eq!(tests.last_run.as_ref().unwrap().label(), "failed");
        assert_eq!(
            tests.action_summary(),
            format!("1 step → agent in {WORKSPACER}")
        );
        assert!(tests.next_run_at.is_some());
        assert!(
            jobs.iter()
                .find(|j| j.id == JOB_AUDIT)
                .unwrap()
                .next_run_at
                .is_none()
        );

        let all = crate::jobs::parse_runs(&catalog.call("jobs.history", &json!({}), &[]).unwrap());
        assert_eq!(all.len(), 10);
        assert!(all.windows(2).all(|w| w[0].started_at >= w[1].started_at));
        let one = crate::jobs::parse_runs(
            &catalog
                .call("jobs.history", &json!({"id": JOB_HEALTH}), &[])
                .unwrap(),
        );
        assert_eq!(one.len(), 4);
        assert!(one.iter().all(|r| r.job_id == JOB_HEALTH));

        // Approve the proposal, pause a job, run one, remove one.
        let proposal = jobs[0].clone();
        catalog
            .call("jobs.upsert", &proposal.approval(), &[])
            .unwrap();
        catalog
            .call("jobs.upsert", &tests.with_enabled(false), &[])
            .unwrap();
        assert_eq!(
            catalog
                .call("jobs.run", &json!({"id": JOB_HEALTH}), &[])
                .unwrap()["started"],
            true
        );
        catalog
            .call("jobs.remove", &json!({"id": JOB_AUDIT}), &[])
            .unwrap();
        let jobs = crate::jobs::parse_list(&catalog.call("jobs.list", &json!({}), &[]).unwrap());
        assert_eq!(jobs.len(), 3);
        assert!(jobs.iter().all(|j| !j.proposal()));
        assert!(jobs.iter().find(|j| j.id == JOB_PRUNE).unwrap().enabled);
        assert!(!jobs.iter().find(|j| j.id == JOB_TESTS).unwrap().enabled);
        let health = jobs.iter().find(|j| j.id == JOB_HEALTH).unwrap();
        assert_eq!(health.last_run.as_ref().unwrap().label(), "skipped");
    }

    #[test]
    fn history_pages_by_cursor_searches_and_forgets() {
        let now = wall_now();
        let mut catalog = Catalog::new(now);
        let rows = live(12);
        let first = crate::features::history_page(
            catalog
                .call("sessions.history", &json!({"limit": 200}), &rows)
                .unwrap(),
            "",
            None,
        )
        .unwrap();
        let page = first["sessions"].as_array().unwrap().clone();
        assert_eq!(page.len(), HISTORY_PAGE);
        let cursor = first["next"].as_str().unwrap().to_owned();
        let second = catalog
            .call(
                "sessions.history",
                &json!({"limit": 200, "cursor": cursor}),
                &rows,
            )
            .unwrap();
        assert!(second.get("next").is_none());
        let mut all = page;
        all.extend(second["sessions"].as_array().unwrap().iter().cloned());
        assert_eq!(all.len(), PAST.len() + rows.len());
        let ids: BTreeSet<_> = all
            .iter()
            .map(|r| r["sessionId"].as_str().unwrap())
            .collect();
        assert_eq!(ids.len(), all.len(), "no row repeats across pages");
        assert!(
            all.windows(2)
                .all(|w| w[0]["updatedAt"].as_i64() >= w[1]["updatedAt"].as_i64())
        );
        assert_eq!(all.iter().filter(|r| r["hidden"] == true).count(), 3);
        for provider in ["claude", "codex"] {
            assert!(all.iter().any(|r| r["provider"] == provider));
        }
        let days: BTreeSet<_> = all
            .iter()
            .map(|r| (now - r["updatedAt"].as_i64().unwrap()) / DAY)
            .collect();
        assert!(days.len() >= 8, "{days:?}");
        // Every row merges into a session the page can title and file.
        for session in sessions(&all) {
            assert!(
                !session.id.is_empty() && !session.title().is_empty() && !session.cwd.is_empty()
            );
        }

        let found = catalog
            .call(
                "sessions.history",
                &json!({"limit": 200, "query": "terraform plan"}),
                &rows,
            )
            .unwrap();
        let found = found["sessions"].as_array().unwrap();
        assert_eq!(found.len(), 1);
        assert_eq!(
            found[0]["sessionId"], "hist-17",
            "first prompt and title both match"
        );

        let gone = catalog
            .call("sessions.forget", &json!({"sessionId": "hist-07"}), &rows)
            .unwrap();
        assert_eq!(gone["ok"], true);
        let running = catalog
            .call("sessions.forget", &json!({"sessionId": "demo-0000"}), &rows)
            .unwrap();
        assert!(running["error"].as_str().unwrap().contains("still running"));
        let ended = catalog
            .call("sessions.forget", &json!({"sessionId": "demo-0010"}), &rows)
            .unwrap();
        assert_eq!(ended["ok"], true);
        let again = catalog
            .call("sessions.forget", &json!({"sessionId": "hist-07"}), &rows)
            .unwrap();
        assert_eq!(again["ok"], false);
        let left = catalog
            .call("sessions.history", &json!({"limit": 200}), &rows)
            .unwrap();
        let left = left["sessions"].as_array().unwrap();
        assert!(
            !left
                .iter()
                .any(|r| r["sessionId"] == "hist-07" || r["sessionId"] == "demo-0010")
        );
    }
}
