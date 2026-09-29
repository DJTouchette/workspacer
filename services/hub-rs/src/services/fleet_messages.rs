//! Fleet wake text is a persisted wire format parsed by every UI client.
use anyhow::{Result, anyhow};
use serde_json::Value;
use std::sync::OnceLock;
fn text<'a>(value: &'a Value, key: &str) -> &'a str {
    value[key].as_str().unwrap_or("")
}
fn assets() -> &'static Value {
    static ASSETS: OnceLock<Value> = OnceLock::new();
    ASSETS.get_or_init(|| {
        serde_json::from_str(include_str!("../../assets/fleet-messages.json")).unwrap()
    })
}
/// Attribution uses a recorded label when supplied; it never invents a name.
pub fn sender_header(session: &str, label: &str) -> String {
    if label.is_empty() {
        format!("[fleet] session:{session} says:\n")
    } else {
        format!("[fleet] session:{session} ({label}) says:\n")
    }
}

pub fn clip(text: &str, max: usize, suffix: &str) -> String {
    if text.encode_utf16().count() <= max {
        return text.into();
    }
    let units: Vec<_> = text.encode_utf16().take(max).collect();
    format!("{}{suffix}", String::from_utf16_lossy(&units))
}
pub fn excerpt(reply: &str) -> String {
    clip(&super::progress::flatten_note(reply), 400, "…")
}
fn full_reply(reply: &str) -> String {
    let reply = reply.trim_matches(super::progress::js_space);
    let size = reply.encode_utf16().count();
    if size <= 32768 {
        return reply.into();
    }
    clip(
        reply,
        32768,
        &format!(
            "\n[truncated: showing the first 32768 of {size} characters — fetch the rest with get_conversation (lastMessage:true)]"
        ),
    )
}
pub fn entry(row: &Value) -> String {
    let where_ = if !text(row, "blockedOn").is_empty() {
        text(row, "blockedOn").into()
    } else {
        format!(
            "cwd {}",
            if text(row, "cwd").is_empty() {
                "?"
            } else {
                text(row, "cwd")
            }
        )
    };
    let mut line = format!(
        "{} (session:{}, {where_})",
        text(row, "label"),
        text(row, "sessionId")
    );
    if row["stopped"] == true {
        line.push_str(" — stopped/killed");
    }
    for (key, label) in [("failed", "FAILED"), ("crossed", "crossed")] {
        if !text(row, key).is_empty() {
            line.push_str(&format!(" — {label}: {}", text(row, key)));
        }
    }
    if row["needsDecision"] == true {
        line.push_str(" — NEEDS A DECISION");
    }
    if !text(row, "note").is_empty() {
        line.push_str(&format!(" — reports: {}", text(row, "note")));
    } else if !text(row, "lastReply").is_empty() {
        line.push_str(&format!(" — last reply: {}", text(row, "lastReply")));
    }
    line
}
pub fn build(kind: &str, entries: &[Value], ordinary_parent: bool) -> Result<String> {
    let assets = assets();
    let mut header = assets["headers"][kind]
        .as_str()
        .ok_or_else(|| anyhow!("unknown fleet message kind"))?;
    if !entries.is_empty() && entries.iter().all(|row| !text(row, "failed").is_empty()) {
        if let Some(alternate) = assets["alternateHeaders"][kind].as_str() {
            header = alternate;
        }
    }
    let tail = if ordinary_parent {
        assets["ordinaryTails"][kind].as_str()
    } else {
        None
    }
    .or_else(|| assets["tails"][kind].as_str())
    .unwrap();
    let mut extras = Vec::new();
    if !ordinary_parent {
        for row in entries {
            if !text(row, "reviewEvidenceId").is_empty() {
                extras.push(format!(
                    "Review evidence — session:{}: {}",
                    text(row, "sessionId"),
                    text(row, "reviewEvidenceId")
                ));
            }
        }
    }
    if entries.iter().any(|row| !text(row, "failed").is_empty()) {
        extras.push(
            text(
                assets,
                if ordinary_parent {
                    "ordinaryFailedNote"
                } else {
                    "failedNote"
                },
            )
            .into(),
        );
    }
    if entries.iter().any(|row| {
        text(row, "failed")
            .to_ascii_lowercase()
            .contains("credit balance is too low")
    }) {
        extras.push(text(assets, "creditBalanceNote").into());
    }
    if entries.iter().any(|row| row["stopped"] == true) {
        extras.push(text(assets, "stoppedNote").into());
    }
    if !ordinary_parent {
        for row in entries {
            if !text(row, "result").is_empty() {
                extras.push(format!(
                    "Structured result — {} (session:{}):\n{}",
                    text(row, "label"),
                    text(row, "sessionId"),
                    text(row, "result")
                ));
            } else if !text(row, "resultError").is_empty() {
                extras.push(format!("Structured result MISSING — {} (session:{}): {}. Read the prose report below/above instead.",text(row,"label"),text(row,"sessionId"),text(row,"resultError")));
            }
        }
    }
    for row in entries {
        if !text(row, "escalation").is_empty() {
            extras.push(format!(
                "Worker escalation — {} (session:{}):\n{}",
                text(row, "label"),
                text(row, "sessionId"),
                text(row, "escalation")
            ));
        } else if !text(row, "escalationError").is_empty() {
            extras.push(format!("Worker escalation INVALID — {} (session:{}): {}. The terminal marker was rejected; treat the prose as an ordinary completion or refusal.",text(row,"label"),text(row,"sessionId"),text(row,"escalationError")));
        }
    }
    for row in entries {
        if !text(row, "fullReply").is_empty() {
            extras.push(format!(
                "Full final message — {} (session:{}):\n{}",
                text(row, "label"),
                text(row, "sessionId"),
                full_reply(text(row, "fullReply"))
            ));
        }
    }
    if !ordinary_parent {
        for row in entries {
            if !text(row, "workflowInstructions").is_empty() {
                extras.push(text(row, "workflowInstructions").into());
            }
        }
    }
    let head = format!(
        "{header}\n{}",
        entries
            .iter()
            .map(|row| format!("- {}", entry(row)))
            .collect::<Vec<_>>()
            .join("\n")
    );
    Ok(if extras.is_empty() {
        format!("{head}\n{tail}")
    } else {
        format!("{head}\n\n{}\n\n{tail}", extras.join("\n\n"))
    })
}
