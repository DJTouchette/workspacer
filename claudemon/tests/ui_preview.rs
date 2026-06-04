//! Visual preview snapshots. Run with:
//!
//!     cargo test --quiet --test ui_preview -- --nocapture
//!
//! Each test renders a representative state to a TestBackend and prints
//! the buffer so we can eyeball the UI without launching the binary.

use claudemon::session::state::{Pending, PendingOption, PendingQuestion};
use claudemon::session::transcript::{Transcript, TranscriptMessage};
use claudemon::session::{SessionMode, SessionState};
use claudemon::tui::preview::{snapshot_chat, snapshot_dashboard, ScenarioBuilder};
use serde_json::json;
use time::OffsetDateTime;

fn now() -> OffsetDateTime {
    OffsetDateTime::now_utc()
}

fn print_titled(title: &str, s: &str) {
    let bar = "═".repeat(78);
    println!("\n╔{bar}╗");
    println!("║ {:<76} ║", title);
    println!("╚{bar}╝");
    println!("{s}");
}

#[test]
fn preview_dashboard_empty() {
    let snap = snapshot_dashboard(ScenarioBuilder::new(), 100, 24);
    print_titled("DASHBOARD — empty", &snap);
}

#[test]
fn preview_dashboard_three_sessions() {
    let b = ScenarioBuilder::new()
        .session(SessionState {
            session_id: "0a1b2c3d-1111-2222-3333-444455556666".into(),
            cwd: Some("/home/dev/proj-frontend".into()),
            mode: SessionMode::Input,
            pending: None,
            started_at: now(),
            updated_at: now(),
            tool_calls: 2,
            last_event: Some("Stop".into()),
            transcript_path: None,
        })
        .session(SessionState {
            session_id: "9e8d7c6b-aaaa-bbbb-cccc-dddddddddddd".into(),
            cwd: Some("/home/dev/proj-api".into()),
            mode: SessionMode::Responding,
            pending: None,
            started_at: now(),
            updated_at: now(),
            tool_calls: 7,
            last_event: Some("PreToolUse".into()),
            transcript_path: None,
        })
        .session(SessionState {
            session_id: "11223344-eeee-ffff-0000-987654321000".into(),
            cwd: Some("/home/dev/scratch".into()),
            mode: SessionMode::Approval,
            pending: Some(Pending::Approval {
                tool: Some("Bash".into()),
                summary: Some("rm -rf node_modules".into()),
                raw: json!({}),
            }),
            started_at: now(),
            updated_at: now(),
            tool_calls: 12,
            last_event: Some("PreToolUse".into()),
            transcript_path: None,
        })
        .connected();
    let snap = snapshot_dashboard(b, 100, 24);
    print_titled("DASHBOARD — three sessions, one pending approval", &snap);
}

#[test]
fn preview_chat_empty() {
    let b = ScenarioBuilder::new()
        .session(SessionState {
            session_id: "demo-session-id".into(),
            cwd: Some("/home/dev/proj".into()),
            mode: SessionMode::Input,
            pending: None,
            started_at: now(),
            updated_at: now(),
            tool_calls: 0,
            last_event: Some("SessionStart".into()),
            transcript_path: None,
        })
        .connected()
        .chat_for("demo-session-id");
    let snap = snapshot_chat(b, 100, 24);
    print_titled("CHAT — empty transcript", &snap);
}

#[test]
fn preview_chat_with_text_and_tool_calls() {
    let b = ScenarioBuilder::new()
        .session(SessionState {
            session_id: "demo-session-id".into(),
            cwd: Some("/home/dev/proj".into()),
            mode: SessionMode::Responding,
            pending: None,
            started_at: now(),
            updated_at: now(),
            tool_calls: 3,
            last_event: Some("PreToolUse".into()),
            transcript_path: None,
        })
        .connected()
        .chat_for_with_transcript("demo-session-id", Transcript {
            path: Some("/x/y.jsonl".into()),
            messages: vec![
                TranscriptMessage {
                    role: "user".into(),
                    content: json!("Can you check what's in src and add a logger?"),
                    raw: json!({}),
                },
                TranscriptMessage {
                    role: "assistant".into(),
                    content: json!([
                        {"type":"text","text":"Sure — let me look around first."},
                        {"type":"tool_use","name":"Bash","id":"a","input":{"command":"ls -la src/"}},
                    ]),
                    raw: json!({}),
                },
                TranscriptMessage {
                    role: "user".into(),
                    content: json!([
                        {"type":"tool_result","tool_use_id":"a",
                         "content":"main.rs\nlib.rs\nlogger.rs\nCargo.toml"}
                    ]),
                    raw: json!({}),
                },
                TranscriptMessage {
                    role: "assistant".into(),
                    content: json!([
                        {"type":"text","text":"You already have logger.rs. I'll wire it into main."},
                        {"type":"tool_use","name":"Read","id":"b","input":{"file_path":"/home/dev/proj/src/main.rs"}},
                    ]),
                    raw: json!({}),
                },
            ],
        });
    let snap = snapshot_chat(b, 100, 30);
    print_titled("CHAT — text + Bash + tool result + Read", &snap);
}

#[test]
fn preview_chat_with_pending_approval() {
    let b = ScenarioBuilder::new()
        .session(SessionState {
            session_id: "demo-session-id".into(),
            cwd: Some("/home/dev/proj".into()),
            mode: SessionMode::Approval,
            pending: Some(Pending::Approval {
                tool: Some("Bash".into()),
                summary: Some("rm -rf node_modules && npm install".into()),
                raw: json!({}),
            }),
            started_at: now(),
            updated_at: now(),
            tool_calls: 5,
            last_event: Some("PreToolUse".into()),
            transcript_path: None,
        })
        .connected()
        .chat_for_with_transcript("demo-session-id", Transcript {
            path: Some("/x/y.jsonl".into()),
            messages: vec![
                TranscriptMessage {
                    role: "user".into(),
                    content: json!("clean install"),
                    raw: json!({}),
                },
                TranscriptMessage {
                    role: "assistant".into(),
                    content: json!([{"type":"text","text":"I'll wipe node_modules and reinstall."}]),
                    raw: json!({}),
                },
            ],
        });
    let snap = snapshot_chat(b, 100, 30);
    print_titled("CHAT — pending approval (Bash)", &snap);
}

#[test]
fn preview_chat_with_pending_question() {
    let b = ScenarioBuilder::new()
        .session(SessionState {
            session_id: "demo-session-id".into(),
            cwd: Some("/home/dev/proj".into()),
            mode: SessionMode::Question,
            pending: Some(Pending::Question {
                questions: vec![PendingQuestion {
                    question: "Which date library should we use?".into(),
                    header: Some("Library".into()),
                    multi_select: false,
                    options: vec![
                        PendingOption { label: "date-fns".into(), description: Some("Functional".into()) },
                        PendingOption { label: "dayjs".into(), description: Some("Tiny".into()) },
                        PendingOption { label: "luxon".into(), description: Some("Rich (i18n, zones)".into()) },
                    ],
                }],
                raw: json!({}),
            }),
            started_at: now(),
            updated_at: now(),
            tool_calls: 1,
            last_event: Some("PreToolUse".into()),
            transcript_path: None,
        })
        .connected()
        .chat_for("demo-session-id");
    let snap = snapshot_chat(b, 100, 30);
    print_titled("CHAT — pending question (3 options)", &snap);
}

#[test]
fn preview_chat_with_input_typed() {
    let b = ScenarioBuilder::new()
        .session(SessionState {
            session_id: "demo-session-id".into(),
            cwd: Some("/home/dev/proj".into()),
            mode: SessionMode::Input,
            pending: None,
            started_at: now(),
            updated_at: now(),
            tool_calls: 0,
            last_event: Some("SessionStart".into()),
            transcript_path: None,
        })
        .connected()
        .chat_for("demo-session-id")
        .typed("can you refactor the auth middleware so the rate limiter sits in front of it?");
    let snap = snapshot_chat(b, 100, 24);
    print_titled("CHAT — single-line typed input", &snap);
}

#[test]
fn preview_chat_with_multiline_input() {
    let b = ScenarioBuilder::new()
        .session(SessionState {
            session_id: "demo-session-id".into(),
            cwd: Some("/home/dev/proj".into()),
            mode: SessionMode::Input,
            pending: None,
            started_at: now(),
            updated_at: now(),
            tool_calls: 0,
            last_event: Some("SessionStart".into()),
            transcript_path: None,
        })
        .connected()
        .chat_for("demo-session-id")
        .typed("Here are the requirements:\n- support multi-tenant routing\n- ratelimit per tenant\n- emit metrics");
    let snap = snapshot_chat(b, 100, 24);
    print_titled("CHAT — multi-line typed input", &snap);
}
