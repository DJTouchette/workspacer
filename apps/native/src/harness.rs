//! Local protocol fixture for repeatable development. Never starts an agent.
use anyhow::Result;
use futures_util::{SinkExt, StreamExt};
use serde_json::{Value, json};
use std::collections::{BTreeMap, BTreeSet};
use std::time::Duration;
use tokio::net::{TcpListener, TcpStream};
use tokio::time::{Instant, MissedTickBehavior, interval};
use tokio_tungstenite::{WebSocketStream, accept_async, tungstenite::Message};

type Socket = WebSocketStream<TcpStream>;

/// The client went away (or sent something unreadable): the connection ends.
struct Closed;

async fn send(socket: &mut Socket, message: Message) -> Result<(), Closed> {
    socket.send(message).await.map_err(|_| Closed)
}

/// What a file reads as until the editor saves over it.
const FIXTURE_SOURCE: &str = "fn main() {\n    restore_workspace();\n    start();\n}\n";

pub async fn serve(listener: TcpListener, sessions: usize, turns: usize) -> Result<()> {
    serve_with_transcript(listener, sessions, turns, false).await
}

pub async fn serve_with_transcript(
    listener: TcpListener,
    sessions: usize,
    turns: usize,
    rich: bool,
) -> Result<()> {
    serve_feedback_fixture(listener, sessions, turns, rich, false, false, false).await
}

/// Visual acceptance fixture. The optional request exercises the native
/// unavailable-session notice through the actual bus event path.
/// `pending_questions` gives the first sessions question sets (see
/// [`fixture_questions`]); each `claude.answer` is printed to stderr as one
/// `fixture claude.answer <params>` line and resolves that session's set.
/// `child_lifecycle` (rich only) finishes the first session's native
/// children and ends its turn a few seconds after connecting, then replays
/// that identical snapshot every 1.5s, for child-row stability captures.
pub async fn serve_feedback_fixture(
    listener: TcpListener,
    sessions: usize,
    turns: usize,
    rich: bool,
    missing_session_request: bool,
    pending_questions: bool,
    child_lifecycle: bool,
) -> Result<()> {
    let options = Options {
        sessions,
        turns,
        rich,
        missing_session_request,
        pending_questions,
        child_lifecycle: child_lifecycle && rich,
    };
    loop {
        let (stream, _) = listener.accept().await?;
        tokio::spawn(async move {
            let Ok(mut socket) = accept_async(stream).await else {
                return;
            };
            let _ = Fixture::new(options).run(&mut socket).await;
        });
    }
}

#[derive(Clone, Copy)]
struct Options {
    sessions: usize,
    turns: usize,
    rich: bool,
    missing_session_request: bool,
    pending_questions: bool,
    child_lifecycle: bool,
}

/// One connection's fixture hub: what it serves and remembers between frames.
struct Fixture {
    options: Options,
    topics: BTreeSet<String>,
    /// The active session's transcript and sequence; other sessions keep
    /// theirs in `history`, or read as `seed`.
    items: Vec<Value>,
    seq: u64,
    active_id: String,
    seed: Vec<Value>,
    history: BTreeMap<String, (Vec<Value>, u64)>,
    /// The first session's pending approval.
    pending: bool,
    answered: BTreeSet<String>,
    streaming: bool,
    /// Fragments left in the reply being streamed.
    remaining: u32,
    /// A new reply starts its fragments on a fresh beat.
    restart_stream: bool,
    /// Editor saves land in memory; nothing touches the disk.
    written: BTreeMap<String, String>,
    /// Fixture shells: id → (cwd, typed line, replay). They only echo.
    shells: BTreeMap<String, (String, String, Vec<u8>)>,
    config: Value,
    /// Sessions started through `agents.spawn`, and fixture events due
    /// later (a reply finishing, the hub's title landing).
    spawned: Vec<Value>,
    openings: BTreeMap<String, String>,
    due: Vec<(Instant, String, Value)>,
    finish_at: Instant,
    finished_parent: Option<Value>,
}

impl Fixture {
    fn new(options: Options) -> Self {
        let mut items: Vec<Value> = (0..options.turns)
            .map(|i| {
                if i % 2 == 0 {
                    json!({
                        "kind": "user_message",
                        "text": format!("Review step {} and show the implementation.", i / 2 + 1),
                    })
                } else {
                    json!({
                        "kind": "assistant_text",
                        "text": "The native client keeps network work off the UI thread.\n\n```rust\nlet snapshot = controller.views.recv().await?;\n```\n\nOnly visible rows are rendered. Select this text or copy the message.",
                    })
                }
            })
            .collect();
        if options.rich {
            items.extend(rich_items());
        }
        Self {
            options,
            topics: BTreeSet::new(),
            seq: items.len() as u64,
            seed: items.clone(),
            items,
            active_id: "demo-0000".to_owned(),
            history: BTreeMap::new(),
            pending: !options.pending_questions,
            answered: BTreeSet::new(),
            streaming: false,
            remaining: 0,
            restart_stream: false,
            written: BTreeMap::new(),
            shells: BTreeMap::new(),
            config: json!({
                "projects": {},
                "agents": {
                    "childFullAccess": false,
                    "autoTitle": {"enabled": true, "model": "haiku"},
                },
            }),
            spawned: Vec::new(),
            openings: BTreeMap::new(),
            due: Vec::new(),
            finish_at: Instant::now() + Duration::from_secs(4),
            finished_parent: None,
        }
    }

    async fn run(mut self, socket: &mut Socket) -> Result<(), Closed> {
        let hello = json!({"op": "hello", "scope": "operator"});
        send(socket, Message::Text(hello.to_string())).await?;
        let mut stream = interval(Duration::from_millis(100));
        stream.set_missed_tick_behavior(MissedTickBehavior::Skip);
        let mut clock = interval(Duration::from_millis(100));
        clock.set_missed_tick_behavior(MissedTickBehavior::Skip);
        let mut replay = interval(Duration::from_millis(1500));
        loop {
            tokio::select! {
                _ = replay.tick(), if self.options.child_lifecycle => self.replay_finished(socket).await?,
                _ = clock.tick(), if !self.due.is_empty() => self.deliver_due(socket).await?,
                _ = stream.tick(), if self.streaming => self.stream_fragment(socket).await?,
                message = socket.next() => {
                    let Some(Ok(message)) = message else {
                        return Err(Closed);
                    };
                    let text = match message {
                        Message::Text(text) => text,
                        Message::Ping(bytes) => {
                            let _ = socket.send(Message::Pong(bytes)).await;
                            continue;
                        }
                        Message::Close(_) => return Err(Closed),
                        _ => continue,
                    };
                    let Ok(frame) = serde_json::from_str::<Value>(&text) else {
                        return Err(Closed);
                    };
                    self.frame(socket, &frame).await?;
                    if std::mem::take(&mut self.restart_stream) {
                        stream.reset();
                    }
                }
            }
        }
    }

    /// Finishes the first session's children and turn once it is time, then
    /// replays that identical snapshot.
    async fn replay_finished(&mut self, socket: &mut Socket) -> Result<(), Closed> {
        if self.finished_parent.is_none() && Instant::now() >= self.finish_at {
            self.pending = false;
            self.finished_parent = Some(finished_rich_parent());
        }
        if let Some(row) = &self.finished_parent {
            send(socket, event("agent.snapshot", row.clone())).await?;
        }
        Ok(())
    }

    async fn deliver_due(&mut self, socket: &mut Socket) -> Result<(), Closed> {
        let now = Instant::now();
        let (ready, later): (Vec<_>, Vec<_>) =
            self.due.drain(..).partition(|(at, _, _)| *at <= now);
        self.due = later;
        for (_, id, patch) in ready {
            let Some(row) = self
                .spawned
                .iter_mut()
                .find(|row| row["sessionId"] == id.as_str())
            else {
                continue;
            };
            for (key, value) in patch.as_object().into_iter().flatten() {
                row[key] = value.clone();
            }
            // The fixture hub's titler: one name after the first answer,
            // never over a launch label, with the configured harness/model
            // recorded truthfully.
            if row["mode"] == "input"
                && row["autoTitle"]["state"] == "pending"
                && self.config["agents"]["autoTitle"]["enabled"] != false
            {
                let opening = self.openings.get(&id).map(String::as_str);
                let (title, record) = fixture_title(&self.config, row, opening.unwrap_or_default());
                if !title.is_empty() {
                    row["label"] = json!(title);
                }
                row["autoTitle"] = record;
            }
            let row = row.clone();
            send(socket, event("agent.snapshot", row)).await?;
        }
        Ok(())
    }

    async fn stream_fragment(&mut self, socket: &mut Socket) -> Result<(), Closed> {
        let fragment = " Streaming native text.";
        self.seq += 1;
        if let Some(last) = self.items.last_mut() {
            let text = format!("{}{fragment}", last["text"].as_str().unwrap_or_default());
            last["text"] = json!(text);
        }
        let topic = format!("agent.conversation.{}", self.active_id);
        let delta = json!({
            "session_id": self.active_id,
            "seq": self.seq,
            "items": [{"kind": "assistant_text", "text": fragment}],
            "reset": false,
        });
        if self.topics.contains(&topic) {
            send(socket, event(&topic, delta)).await?;
        }
        self.remaining -= 1;
        if self.remaining == 0 {
            self.streaming = false;
            let done = json!({"sessionId": self.active_id, "mode": "input"});
            let _ = socket.send(event("agent.snapshot", done)).await;
        }
        Ok(())
    }

    async fn frame(&mut self, socket: &mut Socket, frame: &Value) -> Result<(), Closed> {
        match frame["op"].as_str() {
            Some("subscribe" | "unsubscribe") => {
                let topics = frame["topics"].as_array().into_iter().flatten();
                for topic in topics.filter_map(Value::as_str) {
                    if frame["op"] == "unsubscribe" {
                        self.topics.remove(topic);
                        continue;
                    }
                    self.topics.insert(topic.into());
                    if self.options.missing_session_request && topic == "command.focus_agent" {
                        let missing = json!({"sessionId": "missing-feedback-session"});
                        send(socket, event(topic, missing)).await?;
                    }
                    if topic.starts_with("agent.conversation.") {
                        send(socket, event(topic, json!({"ready": true}))).await?;
                    }
                }
                Ok(())
            }
            Some("call") => {
                if self.options.child_lifecycle {
                    let method = frame["method"].as_str().unwrap_or("unknown");
                    eprintln!("fixture child-lifecycle call {method}");
                }
                let result = self.call(socket, frame).await?;
                let reply = json!({"op": "result", "id": frame["id"], "result": result});
                send(socket, Message::Text(reply.to_string())).await
            }
            _ => Ok(()),
        }
    }

    async fn call(&mut self, socket: &mut Socket, frame: &Value) -> Result<Value, Closed> {
        let params = &frame["params"];
        let id = params["sessionId"].as_str().unwrap_or_default();
        Ok(match frame["method"].as_str().unwrap_or_default() {
            "config.get" => self.config.clone(),
            "config.save" => self.save_config(params),
            "agents.spawn" => self.spawn(socket, params).await?,
            "claude.listModels" => json!({"aliases": [
                {"model": "sonnet"},
                {"model": "sonnet[1m]"},
                {"model": "opus"},
                {"model": "opus[1m]"},
                {"model": "haiku"},
            ]}),
            "providers.listModels" => json!([
                {
                    "id": "gpt-6-astra",
                    "label": "GPT-6 Astra",
                    "default": true,
                    "effortLevels": ["low", "medium", "high", "xhigh", "max"],
                    "defaultEffort": "medium",
                },
                {
                    "id": "gpt-6.1-sol",
                    "label": "GPT-6.1 Sol",
                    "effortLevels": ["low", "medium", "high"],
                    "defaultEffort": "low",
                },
            ]),
            "claude.setEffort" => json!({"ok": true, "disposition": "queued"}),
            "desktop.downloadProjectIcon" => {
                json!({"ok": true, "file": "fixture-project-icon.png"})
            }
            "files.upload" => {
                let name = params["name"].as_str().unwrap_or("image.png");
                json!({"path": format!("/fixture/uploads/{name}")})
            }
            "fs.listDir" => json!({"path": params["path"], "dirs": ["src", "docs"]}),
            "sessions.recent" => self.recent(),
            "fs.readImage" => preview_image(),
            "fs.read" => {
                let path = params["path"].as_str().unwrap_or_default();
                let contents = self.contents(path);
                json!({"path": path, "contents": contents, "size": contents.len()})
            }
            "fs.compareWrite" => {
                let path = params["path"].as_str().unwrap_or_default().to_owned();
                let current = self.contents(&path);
                if params["force"] != true && params["expected"].as_str() != Some(&current) {
                    json!({"saved": false, "conflict": "changed", "current": current})
                } else {
                    let contents = params["contents"].as_str().unwrap_or_default().to_owned();
                    self.written.insert(path, contents.clone());
                    json!({"saved": true, "contents": contents, "size": contents.len()})
                }
            }
            "fs.write" => {
                let path = params["path"].as_str().unwrap_or_default().to_owned();
                let contents = params["contents"].as_str().unwrap_or_default().to_owned();
                self.written.insert(path, contents);
                json!({"ok": true})
            }
            "fs.listEntries" => fixture_listing(params["path"].as_str().unwrap_or_default()),
            "terminals.create" => {
                let shell = format!("fixture-shell-{}", self.shells.len() + 1);
                let cwd = params["cwd"].as_str().unwrap_or_default().to_owned();
                let banner = format!(
                    "\x1b[2mFixture shell (echo only; nothing runs) in\x1b[0m {cwd}\r\n\x1b[32m$\x1b[0m "
                )
                .into_bytes();
                self.shells
                    .insert(shell.clone(), (cwd, String::new(), banner));
                json!({"sessionId": shell})
            }
            "sessions.attachTerminal" => match self.shells.get(id) {
                Some((_, _, replay)) => {
                    let topic = format!("pty.bytes.{id}");
                    if self.topics.contains(&topic) {
                        send(socket, event(&topic, json!(base64_bytes(replay)))).await?;
                    }
                    json!({"ok": true})
                }
                None => json!({"ok": false, "error": "no PTY buffer for that session"}),
            },
            "sessions.terminalInput" => {
                use base64::Engine;
                let bytes = params["bytesB64"]
                    .as_str()
                    .and_then(|b| base64::engine::general_purpose::STANDARD.decode(b).ok())
                    .unwrap_or_default();
                if let Some((cwd, line, replay)) = self.shells.get_mut(id) {
                    let output = fixture_echo(cwd, line, &bytes);
                    replay.extend_from_slice(&output);
                    if replay.len() > 1024 * 1024 {
                        replay.drain(..replay.len() - 1024 * 1024);
                    }
                    let topic = format!("pty.bytes.{id}");
                    if self.topics.contains(&topic) {
                        send(socket, event(&topic, json!(base64_bytes(&output)))).await?;
                    }
                }
                json!({"ok": true})
            }
            "sessions.terminalKeepalive" => json!({"ok": self.shells.contains_key(id)}),
            "sessions.detachTerminal" | "sessions.terminalResize" => json!({"ok": true}),
            "desktop.htmlCardReadDiff" => json!({
                "ok": true,
                "path": params["target"],
                "before": "start();",
                "after": "restore_workspace();\nstart();",
            }),
            "git.numstat" => json!({"files": [{"path": "src/main.rs", "added": 2, "deleted": 1}]}),
            "git.status" => json!({
                "branch": "feature/native-basics",
                "root": params["cwd"],
                "files": [
                    {"path": "src/main.rs", "staged": " ", "unstaged": "M"},
                    {"path": "README.md", "staged": "M", "unstaged": " "},
                    {"path": "tests/session.rs", "staged": "?", "unstaged": "?"},
                ],
            }),
            "git.diff" => {
                json!({"diff": "diff --git a/src/main.rs b/src/main.rs\n--- a/src/main.rs\n+++ b/src/main.rs\n@@ -1,3 +1,4 @@\n fn main() {\n-    start();\n+    restore_workspace();\n+    start();\n }"})
            }
            // A fixture brief path: nothing is written and no provider runs;
            // the successor is the fake spawn above.
            "claude.handoffBrief" | "claude.handoffAgentBrief" => json!({
                "ok": true,
                "path": "/fixture/home/.workspacer/handoffs/20261005-120000-fixture.md",
            }),
            "providers.checkAll" => json!([
                {"provider": "claude", "found": true},
                {"provider": "codex", "found": false},
            ]),
            "desktop.providerReadiness" => json!({"state": "unchecked"}),
            "claude.setModel" => json!({"ok": true, "disposition": "queued"}),
            "sessions.snapshots" => self.snapshots(),
            "sessions.subagentConversation" if self.options.rich => {
                rich_subagent_conversation(id, params["agentId"].as_str().unwrap_or_default())
            }
            "sessions.taskOutput" if self.options.rich => rich_task_output(id, params),
            "sessions.taskStop" if self.options.rich => {
                json!({"ok": true, "task_id": params["taskId"]})
            }
            "sessions.conversation" => {
                let limit = params["limit"].as_u64().map(|l| l as usize);
                if id == self.active_id {
                    page(&self.items, self.seq, limit)
                } else if let Some((items, seq)) = self.history.get(id) {
                    page(items, *seq, limit)
                } else {
                    page(&self.seed, self.seed.len() as u64, limit)
                }
            }
            "usage.report" => usage_report(),
            "agents.sendMessage" => {
                self.send_message(socket, id, &params["text"]).await;
                json!({"ok": true})
            }
            "claude.approve" => {
                self.pending = false;
                json!({"ok": true})
            }
            "claude.signal" => {
                self.streaming = false;
                json!({"ok": true})
            }
            "claude.answer" => {
                eprintln!("fixture claude.answer {params}");
                self.answered.insert(id.to_owned());
                let resolved = json!({"sessionId": id, "mode": "input", "pendingQuestions": null});
                if self.topics.contains("agent.snapshot") {
                    send(socket, event("agent.snapshot", resolved)).await?;
                }
                json!({"ok": true})
            }
            _ => json!({"ok": false, "error": "Unknown fixture method"}),
        })
    }

    fn contents(&self, path: &str) -> String {
        self.written
            .get(path)
            .cloned()
            .unwrap_or_else(|| FIXTURE_SOURCE.into())
    }

    fn save_config(&mut self, params: &Value) -> Value {
        if let Some(projects) = params.get("projects") {
            self.config["projects"] = projects.clone();
        }
        if let Some(enabled) = params.pointer("/agents/childFullAccess") {
            self.config["agents"]["childFullAccess"] = enabled.clone();
        }
        let auto = params
            .pointer("/agents/autoTitle")
            .and_then(Value::as_object);
        for (key, value) in auto.into_iter().flatten() {
            let titles = &mut self.config["agents"]["autoTitle"];
            if key == "models" {
                for (provider, model) in value.as_object().into_iter().flatten() {
                    titles["models"][provider] = model.clone();
                }
            } else {
                titles[key] = value.clone();
            }
        }
        self.config.clone()
    }

    /// A launched session: answers its opening at once, and finishes its
    /// turn (and gets its title) a moment later.
    async fn spawn(&mut self, socket: &mut Socket, params: &Value) -> Result<Value, Closed> {
        let id = format!("fixture-spawn-{}", self.spawned.len() + 1);
        let message = params["message"].as_str().unwrap_or_default().to_owned();
        let mut row = json!({
            "sessionId": id,
            "provider": params["provider"],
            "cwd": params["cwd"],
            "model": params["model"].as_str().unwrap_or("sonnet"),
            "transport": "stream",
            "mode": "responding",
        });
        if let Some(label) = params["label"].as_str().filter(|l| !l.trim().is_empty()) {
            row["label"] = json!(label);
        } else if params["autoTitle"] == true {
            row["autoTitle"] = json!({"state": "pending"});
        }
        let reply = "I traced the redirect to the session cookie check and will patch the guard.";
        let opening = vec![
            json!({"kind": "user_message", "text": message}),
            json!({"kind": "assistant_text", "text": reply}),
        ];
        self.history.insert(id.clone(), (opening, 2));
        self.openings.insert(id.clone(), message.clone());
        self.spawned.push(row.clone());
        send(socket, event("agent.snapshot", row)).await?;
        let finish = Instant::now() + Duration::from_millis(1200);
        self.due
            .push((finish, id.clone(), json!({"mode": "input"})));
        Ok(json!({"sessionId": id, "messageQueued": !message.is_empty()}))
    }

    fn recent(&self) -> Value {
        let mut rows = vec![
            json!({
                "sessionId": "demo-0000",
                "provider": "claude",
                "name": "Native client experiment",
                "cwd": "/workspaces/project-0",
                "mode": "input",
                "transport": "stream",
            }),
            json!({
                "sessionId": "past-session",
                "provider": "codex",
                "name": "Yesterday’s investigation",
                "cwd": "/workspaces/project-1",
                "mode": "stopped",
                "transport": "stream",
            }),
        ];
        rows.extend(self.spawned.iter().map(|row| {
            json!({
                "sessionId": row["sessionId"],
                "provider": row["provider"],
                "name": row["label"].as_str().unwrap_or(""),
                "cwd": row["cwd"],
                "mode": row["mode"],
                "transport": "stream",
            })
        }));
        Value::Array(rows)
    }

    fn snapshots(&self) -> Value {
        let fixed = (0..self.options.sessions).map(|i| {
            let id = format!("demo-{i:04}");
            let snapshot = json!({
                "sessionId": id,
                "label": if i == 0 {
                    "Native client experiment".into()
                } else {
                    format!("Worker {i}")
                },
                "cwd": format!("/workspaces/project-{}", i % 8),
                "parentSessionId": if i == 1 { "demo-0000" } else { "" },
                "provider": "claude",
                "model": "sonnet",
                "transport": "stream",
                "mode": if self.streaming && self.active_id == id {
                    "responding"
                } else {
                    "input"
                },
                "pendingApproval": if i == 0 && self.pending {
                    json!({"toolName": "Bash", "toolInput": {"command": "cargo test"}})
                } else {
                    Value::Null
                },
            });
            let mut snapshot = if self.options.rich {
                rich_snapshot(i, snapshot)
            } else {
                snapshot
            };
            if i == 0
                && let Some(row) = &self.finished_parent
            {
                snapshot = row.clone();
            }
            if self.options.pending_questions
                && !self.answered.contains(&id)
                && let Some(questions) = fixture_questions(i)
            {
                snapshot["pendingQuestions"] = questions;
                snapshot["mode"] = json!("question");
            }
            snapshot
        });
        Value::Array(fixed.chain(self.spawned.iter().cloned()).collect())
    }

    /// A message to any session makes it the active one and streams a reply.
    async fn send_message(&mut self, socket: &mut Socket, id: &str, text: &Value) {
        if self.active_id != id {
            let (next, next_seq) = self
                .history
                .remove(id)
                .unwrap_or_else(|| (self.seed.clone(), self.seed.len() as u64));
            let previous = std::mem::replace(&mut self.items, next);
            self.history
                .insert(std::mem::take(&mut self.active_id), (previous, self.seq));
            self.seq = next_seq;
            self.active_id = id.to_owned();
        }
        self.items
            .push(json!({"kind": "user_message", "text": text}));
        self.items
            .push(json!({"kind": "assistant_text", "text": "Message received."}));
        self.seq += 2;
        self.streaming = true;
        self.remaining = 30;
        self.restart_stream = true;
        let topic = format!("agent.conversation.{}", self.active_id);
        if self.topics.contains(&topic) {
            let delta = json!({"seq": self.seq, "items": &self.items[self.items.len() - 2..]});
            let _ = socket.send(event(&topic, delta)).await;
        }
    }
}

/// The fixture hub's stand-in titler: deterministic, no model call. Names the
/// session from the first words of its opening request and records which
/// harness and model the hub's config would have used.
fn fixture_title(config: &Value, row: &Value, request: &str) -> (String, Value) {
    let auto = &config["agents"]["autoTitle"];
    let provider = auto["provider"]
        .as_str()
        .filter(|p| !p.trim().is_empty())
        .unwrap_or(row["provider"].as_str().unwrap_or("claude"))
        .to_owned();
    let model = auto["models"][provider.as_str()]
        .as_str()
        .filter(|m| !m.is_empty())
        .map(str::to_owned)
        .or_else(|| (provider == "claude").then(|| "haiku".into()));
    let words: Vec<&str> = request
        .lines()
        .next()
        .unwrap_or_default()
        .split_whitespace()
        .skip_while(|w| w.eq_ignore_ascii_case("please"))
        .take(5)
        .collect();
    let mut title = words.join(" ");
    if let Some(first) = title.get(..1) {
        title = first.to_uppercase() + &title[1..];
    }
    let record = if title.is_empty() {
        json!({"state":"skipped","source":"none","provider":provider,"model":model,"reason":"empty"})
    } else {
        json!({"state":"titled","title":title,"source":"model","provider":provider,"model":model})
    };
    (title, record)
}

fn base64_bytes(bytes: &[u8]) -> String {
    use base64::Engine;
    base64::engine::general_purpose::STANDARD.encode(bytes)
}

/// A small synthetic project for the file explorer, under any folder.
fn fixture_listing(path: &str) -> Value {
    let path = path.trim_end_matches('/');
    let entry =
        |name: &str, dir: bool| json!({"name":name,"path":format!("{path}/{name}"),"isDir":dir});
    let entries = match path.rsplit('/').next().unwrap_or_default() {
        "src" => vec![
            entry("main.rs", false),
            entry("lib.rs", false),
            entry("viewer.rs", false),
        ],
        "tests" => vec![entry("session.rs", false)],
        "docs" => vec![entry("guide.md", false)],
        _ => vec![
            entry("docs", true),
            entry("src", true),
            entry("tests", true),
            entry("Cargo.toml", false),
            entry("README.md", false),
        ],
    };
    json!({"path":path,"entries":entries,"includeIgnored":false})
}

/// The fixture shell's echo of typed bytes: Enter answers `pwd`/`ls` and
/// says every other command was not run.
fn fixture_echo(cwd: &str, line: &mut String, bytes: &[u8]) -> Vec<u8> {
    let mut out = Vec::new();
    for &byte in bytes {
        match byte {
            b'\r' | b'\n' => {
                let command = std::mem::take(line);
                let answer = match command.trim() {
                    "" => String::new(),
                    "pwd" => format!("{cwd}\r\n"),
                    "ls" => "\x1b[1;34mdocs\x1b[0m  \x1b[1;34msrc\x1b[0m  \x1b[1;34mtests\x1b[0m  Cargo.toml  README.md\r\n".into(),
                    other => format!("fixture: {other}: not run (this fixture starts no processes)\r\n"),
                };
                out.extend(format!("\r\n{answer}\x1b[32m$\x1b[0m ").into_bytes());
            }
            0x7f | 0x08 => {
                if line.pop().is_some() {
                    out.extend(b"\x08 \x08");
                }
            }
            0x03 => {
                line.clear();
                out.extend(b"^C\r\n\x1b[32m$\x1b[0m ");
            }
            byte if byte >= 0x20 => {
                line.push(byte as char);
                out.push(byte);
            }
            _ => {}
        }
    }
    out
}

/// Two Claude logins (one needing sign-in) and Codex, shaped like `usage.report`.
fn usage_report() -> Value {
    let now = chrono::Utc::now().timestamp();
    let window = |pct: f64, reset_in: i64, pace: &str, expected: f64| {
        json!({"used_percent":{"state":"ok","value":pct},"resets_at":now+reset_in,"is_current":true,
            "pace":{"known":true,"state":pace,"expectedPct":expected}})
    };
    let off = json!({"used_percent":{"state":"unavailable","reason":"extra usage is off"}});
    let reauth =
        json!({"used_percent":{"state":"unknown","reason":"NeedsReauth: oauth token expired"}});
    json!({"generated_at":now,"evaluated_at":now,"valid_until":now+60,"providers":[
        {"provider":"claude","accounts":[
            {"account":"","label":"default","is_default":true,"fresh":true,"windows":{
                "five_hour":window(42.,8_040,"on_track",45.),
                "seven_day":window(76.,3*86_400+4*3_600,"overspending",58.),
                "monthly":off}},
            // A second login whose token expired: listed with its state,
            // as claudemon reports it, rather than dropped.
            {"account":"work","label":"work","is_default":false,"fresh":null,
                "failure":{"kind":"needs_reauth","detail":"oauth token expired (the CLI refreshes it on its next turn)"},
                "windows":{"five_hour":reauth,"seven_day":reauth,"monthly":reauth}}
        ]},
        {"provider":"codex","accounts":[
            {"account":"","label":"default","is_default":true,"fresh":true,"windows":{
                "five_hour":window(93.,2_400,"overspending",70.),
                "seven_day":window(55.,2*86_400,"ahead",48.)}}
        ]}
    ]})
}

/// A claudemon-shaped conversation read: the newest `limit` items and the
/// first returned item's sequence (one sequence per fixture item).
fn page(items: &[Value], seq: u64, limit: Option<usize>) -> Value {
    let start = limit.map_or(0, |limit| items.len().saturating_sub(limit));
    let window_first_seq = if start == 0 {
        1
    } else {
        (seq + 1)
            .saturating_sub((items.len() - start) as u64)
            .max(2)
    };
    json!({"seq":seq,"first_seq":1,"window_first_seq":window_first_seq,"items":&items[start..]})
}

pub fn event(topic: &str, data: Value) -> Message {
    Message::Text(json!({"op":"event", "event":{"type":topic, "data":data}}).to_string())
}

/// Mixed-provider presentation fixture; useful in real-window smoke and tests.
pub fn rich_items() -> Vec<Value> {
    let card = json!({"v":1,"title":"Review complete","bodyHtml":"<table><tr><th>Check</th><th>Result</th></tr><tr><td>Native transcript</td><td>Ready for review</td></tr></table>","fallback":"The native transcript is ready for review.","actions":[{"kind":"fill_composer","label":"Continue","text":"Please continue with the next task."},{"kind":"view_diff","label":"main.rs","path":"src/main.rs"},{"kind":"open_worker","label":"Worker 1","sessionId":"demo-0001"}]});
    let mut items = vec![
        json!({"kind":"user_message","text":"Keep **this literal**, including <tags>. Review the attached screenshot.\n[Image: /workspaces/project-0/screen.png]"}),
        json!({"kind":"assistant_text","text":"I’ll review the implementation and its tests."}),
        json!({"kind":"tool_use","id":"read-1","name":"Read","input":{"file_path":"/workspaces/project-0/src/main.rs"}}),
        json!({"kind":"tool_result","tool_use_id":"read-1","content":"1 fn main() {\n2     start();\n3 }"}),
        json!({"kind":"tool_use","id":"edit-1","name":"Edit","input":{"file_path":"src/main.rs","old_string":"    start();","new_string":"    restore_workspace();\n    start();"}}),
        json!({"kind":"tool_result","tool_use_id":"edit-1","content":"File updated."}),
        json!({"kind":"assistant_text","text":"Now I’ll confirm nothing else calls the old entry point."}),
        json!({"kind":"tool_use","id":"search-1","name":"Grep","input":{"pattern":"restore_workspace","path":"src","description":"Find workspace restoration call sites"}}),
        json!({"kind":"tool_result","tool_use_id":"search-1","content":"src/main.rs:2: restore_workspace();"}),
        json!({"kind":"tool_use","id":"skill-1","name":"Skill","input":{"skill":"review","args":"Check the transcript"}}),
        json!({"kind":"tool_result","tool_use_id":"skill-1","content":"Review completed."}),
        json!({"kind":"assistant_text","text":format!("Implemented the change in [main.rs](src/main.rs:2).\n\n```wks-html-card\n{card}\n```\n")}),
        json!({"kind":"tool_use","id":"workspacer-spawn","name":"mcp__workspacer__spawn_agent","input":{"message":"Review session creation and model selection","label":"Session creation review","trackTask":false}}),
        json!({"kind":"tool_result","tool_use_id":"workspacer-spawn","content":"{\"sessionId\":\"demo-0001\",\"messageQueued\":true,\"taskTracking\":false}"}),
        // Fleet wakes arrive as user turns; native renders them as worker cards.
        json!({"kind":"user_message","text":"[supervisor] An agent is now blocked on a decision:\n- Session creation review (session:demo-0001, approval)\nRun a /supervise pass now."}),
        json!({"kind":"user_message","text":"[fleet] Worker finished:\n- Session creation review (session:demo-0001, cwd /workspaces/project-1) — last reply: Reviewed session creation; two follow-ups noted.\n- Model selection audit (session:demo-0002, cwd /workspaces/project-2) — FAILED: Credit balance is too low\n\nStructured result — Session creation review (session:demo-0001):\n{\"commit\":\"abc1234\",\"checksRun\":[\"cargo test\"]}\n\nReview each result."}),
        json!({"kind":"tool_use","id":"subagent-running","name":"Agent","input":{"description":"Review chat rendering and regression coverage","prompt":"Review the native chat rendering pass.\n\n1. Read apps/native/src/ui/markdown.rs and the vendored text renderer.\n2. Compare tables, blockquotes and code fences with the desktop renderer.\n3. Check every theme (Dark, Light, Nord) for contrast regressions.\n4. Run the ui-tests suite and report failures verbatim.\n5. Note anything that looks unpolished, with file and line.\n\nDo not edit files; report findings only."}}),
        json!({"kind":"assistant_text","text":"## Ready for review\n\nThe conversation is easier to scan, with quieter controls and a little more room to read.\n\n- **Clear hierarchy** for headings and paragraphs.\n- Round bullets, comfortable spacing, and `inline code`.\n- File links open a preview in this workspace.\n\n| Area | Status | Notes |\n|:--|:-:|--:|\n| Tables | Done | Header, stripes, wrapping |\n| Blockquotes | Done | Accent rail |\n| Work cards | Done | `Skill` and `Agent` too |\n\n> Quotes read as asides: a slim rail and muted italic copy,\n> so they never compete with the answer.\n\nSee [README.md](README.md:12) or the [session tests](tests/session.rs).\n\n```rust\nlet workspace = connect().await?;\nworkspace.restore_session();\n```"}),
    ];
    let start = chrono::Utc::now().timestamp_millis() - 60_000;
    for (index, item) in items.iter_mut().enumerate() {
        item["timestamp"] = json!(
            chrono::DateTime::from_timestamp_millis(start + index as i64 * 80)
                .unwrap()
                .to_rfc3339()
        );
    }
    items
}

/// Rich-only child metadata. Keep load/benchmark fixtures unchanged.
/// Pending AskUserQuestion sets for question-picker captures: a mixed set
/// (single choice with descriptions, multiple choice, free text), one
/// two-option question with long descriptions, and a free-text question.
fn fixture_questions(index: usize) -> Option<Value> {
    Some(match index {
        0 => json!([
            {"header":"Approach","question":"Which migration strategy should I use for the session store?","multiSelect":false,"options":[
                {"label":"Online backfill","description":"Copy rows in batches while the hub keeps serving. Slower, no downtime."},
                {"label":"Stop-the-world","description":"Pause writers, migrate in one transaction, restart. About 30 seconds of downtime."},
                {"label":"Skip for now","description":"Keep the old schema behind a compatibility shim."}
            ]},
            {"header":"Checks","question":"Which checks should run before I commit?","multiSelect":true,"options":[
                {"label":"cargo test"},{"label":"Clippy -D warnings"},{"label":"rustfmt --check"},{"label":"Desktop vitest"}
            ]},
            {"header":"Reviewer","question":"Anything the reviewer should know?","multiSelect":false,"options":[]}
        ]),
        2 => json!([
            {"header":"Rollout","question":"The new picker changes how answers are sent. Should I ship it behind a setting first, or enable it for everyone in the next nightly?","multiSelect":false,"options":[
                {"label":"Behind a setting","description":"Default off for one nightly so early users can compare the old and new pickers side by side before it becomes the only behavior."},
                {"label":"Everyone, next nightly","description":"Replace the old picker outright. Faster feedback, but anyone who relies on the old layout loses it immediately."}
            ]}
        ]),
        3 => {
            json!([{"header":"Name","question":"What should I call the new release branch?","multiSelect":false,"options":[]}])
        }
        _ => return None,
    })
}

fn rich_snapshot(index: usize, mut snapshot: Value) -> Value {
    match index {
        0 => {
            snapshot["statusLine"] = json!({"contextUsedPct":42.0,"contextWindowSize":200000});
            // A chosen 1M window, so Change model shows its context choices.
            snapshot["requestedSelection"] = json!({"model":"sonnet","contextWindow":1000000});
            snapshot["subagents"] = json!([
                {"id":"fixture-native-review","toolUseId":"subagent-running","type":"Explore","description":"Inspect transcript parsing","status":"running","model":"gpt-5.6-luna","startedAt":1790852400000i64,"toolCalls":4,"tokens":12400,"costUSD":0.018,"lastToolName":"Read","lastToolSummary":"apps/native/src/model.rs"},
                {"id":"fixture-native-tests","toolUseId":"subagent-running","type":"Test","description":"Check regression coverage","status":"complete","model":"claude-sonnet-4-6","startedAt":1790852400000i64,"completedAt":1790852442000i64,"toolCalls":7,"tokens":28300,"costUSD":0.084},
                // No spawning call in view: lands in a timeline overview card.
                {"id":"fixture-native-audit","type":"Explore","description":"Audit settings search","status":"running","model":"claude-haiku-4-5","startedAt":chrono::Utc::now().timestamp_millis() - 59_000,"toolCalls":3}
            ]);
            // Background work beside the turn (stream transport): a dev
            // server with a proven pid, the running audit child, and two
            // finished tasks. Logs are answered by `sessions.taskOutput`.
            let now = chrono::Utc::now().timestamp_millis();
            snapshot["background_tasks"] = json!(2);
            snapshot["background_task_list"] = json!([
                {"id":"bdevsrv01","taskType":"local_bash","status":"running","description":"npm run dev -- --port 5173","startedAt":now - 754_000,"hasOutput":true,"pid":48213,"toolUseId":"fixture-bg-dev"},
                {"id":"fixture-native-audit","taskType":"local_agent","status":"running","description":"Audit settings search","startedAt":now - 59_000,"subagentId":"fixture-native-audit","usage":{"totalTokens":9100,"toolUses":3,"durationMs":59000},"lastToolName":"Grep","summary":"Running Search settings labels"},
                {"id":"btestwch2","taskType":"local_bash","status":"failed","description":"cargo test --locked -- --test-threads=1","startedAt":now - 1_900_000,"endedAt":now - 1_640_000,"hasOutput":true,"summary":"Background command \"cargo test --locked -- --test-threads=1\" failed with exit code 101"},
                {"id":"wfrelease3","taskType":"local_workflow","status":"completed","description":"Release checklist","workflowName":"release-checklist","startedAt":now - 3_600_000,"endedAt":now - 3_420_000,"hasOutput":true}
            ]);
        }
        1 => {
            snapshot["label"] = json!("Session creation review");
            snapshot["provider"] = json!("codex");
            snapshot["model"] = json!("gpt-5.6-sol");
            snapshot["mode"] = json!("responding");
            snapshot["ambientState"] = json!("streaming");
            snapshot["startedAt"] = json!(1790852400000i64);
            snapshot["lastToolName"] = json!("Read");
            snapshot["lastToolSummary"] = json!("apps/native/src/launch.rs");
            snapshot["usage"] = json!({"inputTokens":15800,"outputTokens":2400,"costUSD":0.046,"contextTokens":183000,"contextLimit":200000});
        }
        _ => (),
    }
    snapshot
}

/// A background task's log for the rich fixture: a dev server's output,
/// served like claudemon's bounded read (tail first, then from `offset`).
fn rich_task_output(session: &str, params: &Value) -> Value {
    let task = params["taskId"].as_str().unwrap_or_default();
    let text = match task {
        "bdevsrv01" => {
            let mut lines = vec![
                "> project@0.4.0 dev".to_owned(),
                "> vite --port 5173".to_owned(),
                String::new(),
                "  VITE v6.2.1  ready in 412 ms".to_owned(),
                String::new(),
                "  ➜  Local:   http://localhost:5173/".to_owned(),
                "  ➜  Network: use --host to expose".to_owned(),
            ];
            for i in 0..36 {
                lines.push(format!(
                    "{:02}:{:02}:{:02} [vite] hmr update /src/{}.tsx",
                    9 + i / 30,
                    (12 + i * 7) % 60,
                    (i * 13) % 60,
                    ["App", "Sidebar", "TaskPanel", "routes/index"][i % 4]
                ));
            }
            lines.push("10:41:07 [vite] page reload src/main.tsx".to_owned());
            lines.join("\n") + "\n"
        }
        "btestwch2" => "running 214 tests\ntest ui::tests::title::chip ... ok\ntest model::merge ... FAILED\n\nfailures:\n    model::merge\n\ntest result: FAILED. 213 passed; 1 failed\n\n[exited with code 101]\n".to_owned(),
        "wfrelease3" => "step 1/3 changelog ... done\nstep 2/3 version bump ... done\nstep 3/3 tag ... done\n".to_owned(),
        _ => return json!({"ok": false, "error": "background task not found for that session"}),
    };
    let size = text.len() as u64;
    let max = params["maxBytes"].as_u64().unwrap_or(65_536).max(1);
    let start = match params["offset"].as_u64() {
        Some(offset) if offset <= size => offset,
        _ => size.saturating_sub(max),
    };
    let (mut start, mut end) = (start as usize, (start + max).min(size) as usize);
    while !text.is_char_boundary(start) {
        start += 1;
    }
    while !text.is_char_boundary(end) {
        end -= 1;
    }
    let running = task == "bdevsrv01";
    json!({
        "session_id": session, "task_id": task,
        "status": if running { "running" } else if task == "btestwch2" { "failed" } else { "completed" },
        "running": running, "offset": start, "next_offset": end, "size": size,
        "text": &text[start..end], "reset": false,
        "done": !running && end as u64 >= size,
        "process": if running {
            json!({"pid":48213,"alive":true,"processes":3,"rss_bytes":187_432_960u64,"cpu_seconds":41.2,"cpu_percent":2.4})
        } else {
            Value::Null
        },
    })
}

/// The first rich session after its native children finish and its turn
/// ends: the same identities with terminal status and fixed times, so every
/// replay is byte-identical.
fn finished_rich_parent() -> Value {
    let base = json!({
        "sessionId":"demo-0000","label":"Native client experiment","cwd":"/workspaces/project-0",
        "parentSessionId":"","provider":"claude","model":"sonnet","transport":"stream",
        "mode":"input","pendingApproval":Value::Null
    });
    let mut parent = rich_snapshot(0, base);
    let now = chrono::Utc::now().timestamp_millis();
    for child in parent["subagents"].as_array_mut().into_iter().flatten() {
        if child["status"] == "running" {
            child["status"] = json!("complete");
            child["completedAt"] = json!(now);
        }
    }
    parent
}

fn rich_subagent_conversation(session: &str, agent: &str) -> Value {
    if session != "demo-0000"
        || !matches!(
            agent,
            "fixture-native-review" | "fixture-native-tests" | "fixture-native-audit"
        )
    {
        return json!({"ok":false,"error":"Subagent does not belong to this fixture session"});
    }
    let complete = agent == "fixture-native-tests";
    let items = vec![
        json!({"kind":"user_message","text":if complete {"Check the native regression tests."} else {"Inspect the native transcript parser."}}),
        json!({"kind":"assistant_text","text":"I’m checking the relevant source and focused regression coverage."}),
        json!({"kind":"tool_use","id":"child-read","name":"Read","input":{"file_path":"apps/native/src/model.rs"}}),
        json!({"kind":"tool_result","tool_use_id":"child-read","content":"The transcript keeps tool results paired with their tool IDs."}),
        json!({"kind":"assistant_text","text":if complete {"Regression checks passed. Tool output stays literal and child rows attach to their dispatch tool."} else {"The tool/result boundary looks correct. I’m checking replay handling next."}}),
    ];
    json!({"session_id":session,"agent_id":agent,"seq":items.len(),"first_seq":1,"items":items})
}

fn preview_image() -> Value {
    use base64::Engine;
    let image = image::RgbImage::from_fn(320, 160, |x, y| {
        image::Rgb(if x < 160 {
            [40, (80 + y / 2) as u8, 150]
        } else {
            [40, 150, (80 + y / 2) as u8]
        })
    });
    let mut encoded = std::io::Cursor::new(Vec::new());
    if image
        .write_to(&mut encoded, image::ImageFormat::Png)
        .is_err()
    {
        return Value::Null;
    }
    json!({"dataUrl":format!("data:image/png;base64,{}",base64::engine::general_purpose::STANDARD.encode(encoded.into_inner()))})
}

/// Exercise the same WebSocket/controller boundary without a GUI or provider.
/// UI intent receipt is not permission to start an invisible terminal/agent.
pub async fn ui_intent_probe() -> Result<Value> {
    use crate::{bus::Config, controller::Controller, ui_requests::Intent};
    use std::time::Duration;
    let listener = TcpListener::bind("127.0.0.1:0").await?;
    let config = Config::new(format!("ws://{}/bus", listener.local_addr()?), None)?;
    let (stop, mut stopping) = tokio::sync::oneshot::channel();
    let server = tokio::spawn(async move {
        let (stream, _) = listener.accept().await?;
        let mut socket = accept_async(stream).await?;
        socket
            .send(Message::Text(
                json!({"op":"hello","scope":"operator"}).to_string(),
            ))
            .await?;
        let mut published = false;
        loop {
            tokio::select! {
             _=&mut stopping=>return anyhow::Ok(published),
             message=socket.next()=>{let Some(message)=message else{return anyhow::Ok(published)};let message=message?;
              let Message::Text(text)=message else{continue};let frame:Value=serde_json::from_str(&text)?;
              match frame["op"].as_str(){
               Some("subscribe")=>{if !published&&frame["topics"].as_array().is_some_and(|a|a.iter().any(|v|v=="facade.openTerminal")){
                socket.send(event("facade.openTerminal",json!({"cwd":"/fixture/project","command":"echo request-only","label":"Fixture","parentSessionId":"fixture-parent"}))).await?;
                socket.send(event("command.open_spawn_dialog",json!({"cwd":"/fixture/project"}))).await?;published=true;
               }},
               Some("call")=>{anyhow::ensure!(frame["method"]=="sessions.snapshots","Headless UI intent unexpectedly issued a backend mutation");socket.send(Message::Text(json!({"op":"result","id":frame["id"],"result":[]}).to_string())).await?;},
               _=>(),
              }
             }
            }
        }
    });
    let controller = Controller::start(config);
    let mut views = controller.views.clone();
    let probe=tokio::time::timeout(Duration::from_secs(5),async{
  loop {let view=views.borrow_and_update().clone();if view.ui_requests.len()==2{
    anyhow::ensure!(view.ui_requests.iter().any(|r|matches!(&r.intent,Intent::Terminal{cwd,command,label,parent_session_id} if cwd=="/fixture/project"&&command=="echo request-only"&&label=="Fixture"&&parent_session_id=="fixture-parent")),"Terminal request metadata changed");
    anyhow::ensure!(!view.creating&&view.spawn_receipt.is_none(),"Display request created an agent");
    tokio::time::sleep(Duration::from_millis(100)).await;
    break anyhow::Ok(json!({"queuedUiRequests":2,"terminalParametersRetained":true,"backendMutations":0,"visiblePaneAcknowledged":false}));
   }views.changed().await?;
  }
 }).await.map_err(|_|anyhow::anyhow!("UI intent probe timed out")).and_then(|r|r);
    let _ = stop.send(());
    let published = server.await??;
    drop(controller);
    drop(views);
    anyhow::ensure!(published, "UI topics were not subscribed");
    probe
}

/// A dense, provider-shaped conversation: prose with fenced code, tool calls
/// and their multi-line results. Tool ids are unique per `tag`.
pub fn dense_items(count: usize, tag: &str) -> Vec<Value> {
    let output = (1..=40)
        .map(|n| format!("{n:>4} let value_{n} = compute({n});"))
        .collect::<Vec<_>>()
        .join("\n");
    let start = chrono::Utc::now().timestamp_millis() - count as i64 * 1000;
    (0..count)
        .map(|i| {
            let id = format!("{tag}-tool-{}", i / 5);
            let mut item = match i % 5 {
                0 => json!({"kind":"user_message","text":format!("Step {}: review the next module and report regressions.", i / 5 + 1)}),
                1 => json!({"kind":"tool_use","id":id,"name":"Read","input":{"file_path":format!("src/module_{}.rs", i / 5)}}),
                2 => json!({"kind":"tool_result","tool_use_id":id,"content":output}),
                3 => json!({"kind":"tool_use","id":format!("{id}-edit"),"name":"Edit","input":{"file_path":format!("src/module_{}.rs", i / 5),"old_string":"compute(1)","new_string":"compute_checked(1)?"}}),
                _ => json!({"kind":"assistant_text","text":format!("## Module {}\n\nThe change keeps **ordering** stable and adds `compute_checked`.\n\n- Callers updated\n- Tests cover the error path\n\n```rust\nfn compute_checked(n: u32) -> Result<u32> {{\n    n.checked_mul(2).context(\"overflow\")\n}}\n```\n\n| Check | Result |\n|---|---|\n| unit | pass |\n| clippy | pass |", i / 5 + 1)}),
            };
            item["timestamp"] = json!(
                chrono::DateTime::from_timestamp_millis(start + i as i64 * 1000)
                    .unwrap()
                    .to_rfc3339()
            );
            item
        })
        .collect()
}

/// Parent/child chat switching through the real controller and bus client
/// against an in-process dense fixture. Each cycle visits two new children,
/// returns to the first, then to the parent. `shown_ms` is the wall time from
/// `ViewChild` to the first published view showing the target's transcript;
/// `reconciled_ms` to the view after that switch's hub read was folded in.
/// Excludes GPUI layout/paint and any real backend's read latency (the fixture
/// answers immediately), so it isolates client-side churn.
pub async fn bench_switch(cycles: usize, parent_items: usize, child_items: usize) -> Result<Value> {
    use crate::controller::{ChildTarget, Command, Controller, View};
    use std::sync::{Arc, Mutex};
    use std::time::{Duration, Instant};
    const PARENT: &str = "bench-parent";
    let listener = TcpListener::bind("127.0.0.1:0").await?;
    let config = crate::bus::Config::new(format!("ws://{}/bus", listener.local_addr()?), None)?;
    let calls = Arc::new(Mutex::new(BTreeMap::<String, usize>::new()));
    let parent = dense_items(parent_items, "parent");
    let child_ids: Vec<String> = (0..cycles)
        .flat_map(|i| [format!("bench-child-{i}-a"), format!("bench-child-{i}-b")])
        .collect();
    let children: BTreeMap<String, Vec<Value>> = child_ids
        .iter()
        .map(|id| (id.clone(), dense_items(child_items, id)))
        .collect();
    let row = json!({"sessionId":PARENT,"label":"Dense parent","cwd":"/fixture/project",
        "provider":"claude","model":"sonnet","transport":"stream","mode":"input",
        "subagents":child_ids.iter().map(|id| json!({"id":id,"type":"Explore","description":format!("Audit {id}"),
            "status":"complete","startedAt":1790852400000i64,"completedAt":1790852442000i64,"toolCalls":40})).collect::<Vec<_>>()});
    let counter = calls.clone();
    let server = tokio::spawn(async move {
        let (stream, _) = listener.accept().await?;
        let mut socket = accept_async(stream).await?;
        socket
            .send(Message::Text(
                json!({"op":"hello","scope":"operator"}).to_string(),
            ))
            .await?;
        while let Some(message) = socket.next().await {
            let Message::Text(text) = message? else {
                continue;
            };
            let frame: Value = serde_json::from_str(&text)?;
            match frame["op"].as_str() {
                Some("subscribe") => {
                    for topic in frame["topics"].as_array().into_iter().flatten() {
                        if let Some(topic) = topic
                            .as_str()
                            .filter(|t| t.starts_with("agent.conversation."))
                        {
                            socket.send(event(topic, json!({"ready":true}))).await?;
                        }
                    }
                }
                Some("call") => {
                    let method = frame["method"].as_str().unwrap_or_default();
                    *counter.lock().unwrap().entry(method.into()).or_default() += 1;
                    let params = &frame["params"];
                    let result = match method {
                        "sessions.snapshots" => json!([row]),
                        "sessions.conversation" => page(
                            &parent,
                            parent.len() as u64,
                            params["limit"].as_u64().map(|l| l as usize),
                        ),
                        "sessions.subagentConversation" => {
                            let agent = params["agentId"].as_str().unwrap_or_default();
                            match children.get(agent) {
                                Some(items) => {
                                    json!({"session_id":PARENT,"agent_id":agent,"seq":items.len(),"first_seq":1,"items":items})
                                }
                                None => json!({"ok":false,"error":"unknown child"}),
                            }
                        }
                        "usage.report" => json!({"providers":[]}),
                        _ => Value::Null,
                    };
                    socket
                        .send(Message::Text(
                            json!({"op":"result","id":frame["id"],"result":result}).to_string(),
                        ))
                        .await?;
                }
                _ => (),
            }
        }
        anyhow::Ok(())
    });
    let controller = Controller::start(config);
    let mut views = controller.views.clone();
    async fn until(
        views: &mut tokio::sync::watch::Receiver<Arc<View>>,
        done: impl Fn(&View) -> bool,
    ) -> Result<Arc<View>> {
        tokio::time::timeout(Duration::from_secs(20), async {
            loop {
                let view = views.borrow_and_update().clone();
                if done(&view) {
                    return anyhow::Ok(view);
                }
                views.changed().await?;
            }
        })
        .await
        .map_err(|_| anyhow::anyhow!("switch did not settle"))?
    }
    // Parent rows coalesce (a tool result folds into its call).
    let parent_rows = parent_items.min(crate::controller::CONVERSATION_PAGE) * 3 / 5;
    let child_rows = child_items * 3 / 5;
    let showing = |child: Option<String>| {
        let rows = if child.is_some() {
            child_rows
        } else {
            parent_rows
        };
        move |view: &View| {
            view.connected
                && view.selected.as_deref() == Some(PARENT)
                && view.child.as_ref().map(|c| &c.agent) == child.as_ref()
                && !view.loading
                && view.transcript.rows.len() >= rows
        }
    };
    until(&mut views, showing(None)).await?;
    tokio::time::sleep(Duration::from_millis(100)).await;
    let mut samples: BTreeMap<&str, (Vec<f64>, Vec<f64>)> = BTreeMap::new();
    let mut reads: BTreeMap<&str, BTreeMap<String, usize>> = BTreeMap::new();
    for cycle in 0..cycles {
        let (a, b) = (
            format!("bench-child-{cycle}-a"),
            format!("bench-child-{cycle}-b"),
        );
        for (name, child) in [
            ("parent_to_new_child", Some(a.clone())),
            ("child_to_new_sibling", Some(b)),
            ("child_to_revisited_sibling", Some(a)),
            ("child_to_parent", None),
        ] {
            calls.lock().unwrap().clear();
            views.borrow_and_update();
            let started = Instant::now();
            controller.command(Command::ViewChild(child.clone().map(|agent| ChildTarget {
                parent: PARENT.into(),
                agent,
            })))?;
            let shown = until(&mut views, showing(child.clone())).await?;
            let shown_ms = started.elapsed().as_secs_f64() * 1000.;
            // A held transcript shows before its read; the read's fold then
            // publishes a later revision. A fresh one shows only as that fold.
            let revision = shown.transcript.revision;
            let folded = tokio::time::timeout(
                Duration::from_millis(250),
                until(&mut views, |v| v.transcript.revision > revision),
            )
            .await;
            let reconciled_ms = match folded {
                Ok(Ok(_)) => started.elapsed().as_secs_f64() * 1000.,
                _ => shown_ms,
            };
            let entry = samples.entry(name).or_default();
            entry.0.push(shown_ms);
            entry.1.push(reconciled_ms);
            // Let reads the transition started but did not wait for arrive.
            tokio::time::sleep(Duration::from_millis(60)).await;
            for (method, count) in calls.lock().unwrap().iter() {
                *reads
                    .entry(name)
                    .or_default()
                    .entry(method.clone())
                    .or_default() += count;
            }
        }
    }
    drop(controller);
    drop(views);
    server.abort();
    let mut report = json!({"cycles":cycles,"parent_items":parent_items,"child_items":child_items,
        "debug_assertions":cfg!(debug_assertions),
        "scope":"real controller + bus client against an in-process fixture that answers immediately; ms from ViewChild to the first published view showing the target (shown) and to the view with that switch's read folded in (reconciled); includes the controller's 33ms publish tick; excludes GPUI layout/paint and real backend read cost"});
    for (name, (mut shown, mut reconciled)) in samples {
        shown.sort_by(f64::total_cmp);
        reconciled.sort_by(f64::total_cmp);
        let at = |v: &[f64], p: usize| v[(v.len() - 1) * p / 100];
        let per_switch: BTreeMap<_, _> = reads
            .remove(name)
            .unwrap_or_default()
            .into_iter()
            .map(|(method, count)| (method, count as f64 / cycles as f64))
            .collect();
        report[name] = json!({"shown_p50_ms":at(&shown,50),"shown_p95_ms":at(&shown,95),
            "reconciled_p50_ms":at(&reconciled,50),"reconciled_p95_ms":at(&reconciled,95),
            "reads_per_switch":per_switch});
    }
    Ok(report)
}

#[cfg(test)]
mod child_fixture_tests {
    use super::*;

    #[test]
    fn child_lifecycle_finishes_the_same_children_and_ends_the_turn() {
        let running = rich_snapshot(0, json!({"sessionId":"demo-0000"}));
        let finished = finished_rich_parent();
        assert_eq!(finished["mode"], "input");
        assert!(finished["pendingApproval"].is_null());
        let ids = |v: &Value| -> Vec<Value> {
            v["subagents"]
                .as_array()
                .unwrap()
                .iter()
                .map(|c| c["id"].clone())
                .collect()
        };
        assert_eq!(ids(&running), ids(&finished), "identities are kept");
        for child in finished["subagents"].as_array().unwrap() {
            assert_eq!(child["status"], "complete");
            assert!(child["completedAt"].as_i64().is_some());
        }
    }

    #[test]
    fn rich_children_have_dispatch_anchors_and_parent_scoped_replay() {
        let items = rich_items();
        let parent = rich_snapshot(0, json!({"sessionId":"demo-0000"}));
        for child in parent["subagents"].as_array().unwrap() {
            // One child deliberately has no spawning call (timeline overview).
            assert!(
                child["toolUseId"].is_null()
                    || items.iter().any(|item| {
                        item["kind"] == "tool_use" && item["id"] == child["toolUseId"]
                    })
            );
            let agent = child["id"].as_str().unwrap();
            let replay = rich_subagent_conversation("demo-0000", agent);
            let replay_items = replay["items"].as_array().unwrap();
            assert_eq!(replay["seq"].as_u64().unwrap(), replay_items.len() as u64);
            assert!(replay_items.len() < 20);
            assert_eq!(rich_subagent_conversation("demo-0001", agent)["ok"], false);
        }
        let result = items
            .iter()
            .find(|item| item["tool_use_id"] == "workspacer-spawn")
            .unwrap();
        let receipt: Value = serde_json::from_str(result["content"].as_str().unwrap()).unwrap();
        assert_eq!(receipt["sessionId"], "demo-0001");
        assert_eq!(
            rich_snapshot(1, json!({"parentSessionId":"demo-0000"}))["parentSessionId"],
            "demo-0000"
        );
    }
}
