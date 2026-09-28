//! Local protocol fixture for repeatable development. Never starts an agent.
use anyhow::Result;
use futures_util::{SinkExt, StreamExt};
use serde_json::{Value, json};
use std::collections::{BTreeMap, BTreeSet};
use tokio::net::TcpListener;
use tokio_tungstenite::{accept_async, tungstenite::Message};

pub async fn serve(listener: TcpListener, sessions: usize, turns: usize) -> Result<()> {
    serve_with_transcript(listener, sessions, turns, false).await
}

pub async fn serve_with_transcript(
    listener: TcpListener,
    sessions: usize,
    turns: usize,
    rich: bool,
) -> Result<()> {
    loop {
        let (stream, _) = listener.accept().await?;
        tokio::spawn(async move {
            let Ok(mut socket) = accept_async(stream).await else {
                return;
            };
            if socket
                .send(Message::Text(
                    json!({"op":"hello", "scope":"operator"}).to_string(),
                ))
                .await
                .is_err()
            {
                return;
            }
            let mut topics = BTreeSet::<String>::new();
            let mut items: Vec<Value> = (0..turns).map(|i| {
                if i % 2 == 0 { json!({"kind":"user_message", "text":format!("Review step {} and show the implementation.", i / 2 + 1)}) }
                else { json!({"kind":"assistant_text", "text":"The native client keeps network work off the UI thread.\n\n```rust\nlet snapshot = controller.views.recv().await?;\n```\n\nOnly visible rows are rendered. Select this text or copy the message."}) }
            }).collect();
            if rich {
                items.extend(rich_items());
            }
            let seed = items.clone();
            let mut history: BTreeMap<String, (Vec<Value>, u64)> = BTreeMap::new();
            let mut active_id = "demo-0000".to_owned();
            let mut seq = items.len() as u64;
            let mut pending = true;
            let mut tick = tokio::time::interval(std::time::Duration::from_millis(100));
            tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
            let mut streaming = false;
            let mut remaining = 0;
            loop {
                tokio::select! {
                    _ = tick.tick(), if streaming => {
                        let fragment = " Streaming native text.";
                        seq += 1;
                        if let Some(last) = items.last_mut() { let text = format!("{}{fragment}", last["text"].as_str().unwrap_or_default()); last["text"] = json!(text); }
                        let topic = format!("agent.conversation.{active_id}");
                        let delta = json!({"session_id":active_id, "seq":seq, "items":[{"kind":"assistant_text", "text":fragment}], "reset":false});
                        if topics.contains(&topic) && socket.send(event(&topic, delta)).await.is_err() { return; }
                        remaining -= 1;
                        if remaining == 0 {
                            streaming = false;
                            let _ = socket.send(event("agent.snapshot", json!({"sessionId":active_id,"mode":"input"}))).await;
                        }
                    }
                    message = socket.next() => {
                        let Some(Ok(message)) = message else { return; };
                        let text = match message {
                            Message::Text(text) => text,
                            Message::Ping(p) => { let _ = socket.send(Message::Pong(p)).await; continue; }
                            Message::Close(_) => return,
                            _ => continue,
                        };
                        let Ok(frame) = serde_json::from_str::<Value>(&text) else { return; };
                        match frame["op"].as_str() {
                            Some("subscribe" | "unsubscribe") => {
                                for topic in frame["topics"].as_array().into_iter().flatten().filter_map(Value::as_str) {
                                    if frame["op"] == "unsubscribe" { topics.remove(topic); }
                                    else {
                                        topics.insert(topic.into());
                                        if topic.starts_with("agent.conversation.") && socket.send(event(topic, json!({"ready":true}))).await.is_err() { return; }
                                    }
                                }
                            }
                            Some("call") => {
                                let id = frame["params"]["sessionId"].as_str().unwrap_or_default();
                                let result = match frame["method"].as_str().unwrap_or_default() {
                                    "sessions.recent" => json!([
                                        {"sessionId":"demo-0000","provider":"claude","name":"Native client experiment","cwd":"/workspaces/project-0","mode":"input","transport":"stream"},
                                        {"sessionId":"past-session","provider":"codex","name":"Yesterday’s investigation","cwd":"/workspaces/project-1","mode":"stopped","transport":"stream"}
                                    ]),
                                    "fs.readImage" => preview_image(),
                                    "fs.read" => json!({"path":frame["params"]["path"],"contents":"fn main() {\n    restore_workspace();\n    start();\n}\n","size":58}),
                                    "desktop.htmlCardReadDiff" => json!({"ok":true,"path":frame["params"]["target"],"before":"start();","after":"restore_workspace();\nstart();"}),
                                    "git.numstat" => json!({"files":[{"path":"src/main.rs","added":2,"deleted":1}]}),
                                    "git.status" => json!({"branch":"feature/native-basics","files":[{"path":"src/main.rs","staged":" ","unstaged":"M"},{"path":"README.md","staged":"M","unstaged":" "},{"path":"tests/session.rs","staged":"?","unstaged":"?"}]}),
                                    "git.diff" => json!({"diff":"diff --git a/src/main.rs b/src/main.rs\n--- a/src/main.rs\n+++ b/src/main.rs\n@@ -1,3 +1,4 @@\n fn main() {\n-    start();\n+    restore_workspace();\n+    start();\n }"}),
                                    "providers.checkAll" => json!([{"provider":"claude","found":true},{"provider":"codex","found":false}]),
                                    "desktop.providerReadiness" => json!({"state":"unchecked"}),
                                    "claude.setModel" => json!({"ok":true,"disposition":"queued"}),
                                    "sessions.snapshots" => Value::Array((0..sessions).map(|i| json!({
                                        "sessionId":format!("demo-{i:04}"), "label":if i == 0 {"Native client experiment".into()} else {format!("Worker {i}")},
                                        "cwd":format!("/workspaces/project-{}", i % 8), "parentSessionId":if i==1 {"demo-0000"}else{""}, "provider":"claude", "model":"sonnet", "transport":"stream", "mode":if streaming && active_id == format!("demo-{i:04}") {"responding"} else {"input"},
                                        "pendingApproval":if i == 0 && pending {json!({"toolName":"Bash", "toolInput":{"command":"cargo test"}})} else {Value::Null}
                                    })).collect()),
                                    "sessions.conversation" => {
                                        if id == active_id { json!({"seq":seq, "first_seq":1, "items":items}) }
                                        else if let Some((items, seq)) = history.get(id) { json!({"seq":seq,"first_seq":1,"items":items}) }
                                        else { json!({"seq":seed.len(),"first_seq":1,"items":seed}) }
                                    }
                                    "agents.sendMessage" => {
                                        if active_id != id {
                                            let (next, next_seq) = history.remove(id).unwrap_or_else(|| (seed.clone(),seed.len() as u64));
                                            history.insert(active_id, (std::mem::replace(&mut items,next),seq));
                                            seq = next_seq;
                                            active_id = id.to_owned();
                                        }
                                        items.push(json!({"kind":"user_message", "text":frame["params"]["text"]}));
                                        items.push(json!({"kind":"assistant_text", "text":"Message received."}));
                                        seq += 2;
                                        streaming = true; remaining = 30;
                                        tick.reset();
                                        let topic = format!("agent.conversation.{active_id}");
                                        if topics.contains(&topic) {
                                            let _ = socket.send(event(&topic, json!({"seq":seq,"items":&items[items.len()-2..]}))).await;
                                        }
                                        json!({"ok":true})
                                    }
                                    "claude.approve" => { pending = false; json!({"ok":true}) }
                                    "claude.signal" => { streaming = false; json!({"ok":true}) }
                                    "claude.answer" => json!({"ok":true}),
                                    _ => json!({"ok":false,"error":"Unknown fixture method"}),
                                };
                                if socket.send(Message::Text(json!({"op":"result","id":frame["id"],"result":result}).to_string())).await.is_err() { return; }
                            }
                            _ => {}
                        }
                    }
                }
            }
        });
    }
}

pub fn event(topic: &str, data: Value) -> Message {
    Message::Text(json!({"op":"event", "event":{"type":topic, "data":data}}).to_string())
}

/// Mixed-provider presentation fixture; useful in real-window smoke and tests.
pub fn rich_items() -> Vec<Value> {
    let card = json!({"v":1,"title":"Review complete","bodyHtml":"<table><tr><th>Check</th><th>Result</th></tr><tr><td>Native transcript</td><td>Ready for review</td></tr></table>","fallback":"The native transcript is ready for review.","actions":[{"kind":"fill_composer","label":"Continue","text":"Please continue with the next task."},{"kind":"view_diff","label":"main.rs","path":"src/main.rs"},{"kind":"open_worker","label":"Worker 1","sessionId":"demo-0001"}]});
    vec![
        json!({"kind":"user_message","text":"Keep **this literal**, including <tags>. Review the attached screenshot.\n[Image: /workspaces/project-0/screen.png]"}),
        json!({"kind":"assistant_text","text":"I’ll review the implementation and its tests."}),
        json!({"kind":"tool_use","id":"read-1","name":"Read","input":{"file_path":"/workspaces/project-0/src/main.rs"}}),
        json!({"kind":"tool_result","tool_use_id":"read-1","content":"1 fn main() {\n2     start();\n3 }"}),
        json!({"kind":"tool_use","id":"edit-1","name":"Edit","input":{"file_path":"src/main.rs","old_string":"    start();","new_string":"    restore_workspace();\n    start();"}}),
        json!({"kind":"tool_result","tool_use_id":"edit-1","content":"File updated."}),
        json!({"kind":"tool_use","id":"skill-1","name":"Skill","input":{"skill":"review","args":"Check the transcript"}}),
        json!({"kind":"tool_result","tool_use_id":"skill-1","content":"Review completed."}),
        json!({"kind":"assistant_text","text":format!("Implemented the change in [main.rs](src/main.rs:2).\n\n```wks-html-card\n{card}\n```\n")}),
    ]
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
