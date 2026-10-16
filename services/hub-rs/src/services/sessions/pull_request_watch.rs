//! Keeps linked pull requests current and the agent informed:
//!
//! - a session with no link (and no explicit unlink) is linked to the open
//!   pull request for its branch (`pullRequests.autoLink`);
//! - each linked pull request's threads are re-read every few minutes, which
//!   refreshes the snapshot's state and unresolved count;
//! - comments nobody has told the agent about yet are sent to it once it is
//!   idle, batched and rate-limited (`pullRequests.notifyAgent`).
//!
//! Comments present when a link is made are taken as read, and comments this
//! hub posted (the app's or an agent's replies) are never news.
use super::super::pull_requests::{
    links::{Link, Source},
    model::Threads,
};
use super::Sessions;
use claudemon::session::Delivery;
use serde_json::Value;
use std::{
    collections::HashMap,
    sync::Arc,
    time::{Duration, Instant},
};

const TICK: Duration = Duration::from_secs(60);
const BRANCH_EVERY: Duration = Duration::from_secs(5 * 60);
const THREADS_EVERY: Duration = Duration::from_secs(2 * 60);
/// A merged or closed pull request rarely changes again.
const SETTLED_EVERY: Duration = Duration::from_secs(30 * 60);
const NOTIFY_GAP: Duration = Duration::from_secs(5 * 60);
const MESSAGE_LIMIT: usize = 8_000;

#[derive(Default)]
pub(super) struct WatchState {
    branch_checked: HashMap<String, Instant>,
    threads_checked: HashMap<String, Instant>,
    notified: HashMap<String, Instant>,
}

fn due(map: &HashMap<String, Instant>, key: &str, every: Duration, now: Instant) -> bool {
    map.get(key)
        .is_none_or(|at| now.duration_since(*at) >= every)
}

/// The session can take a message without interrupting anything.
fn idle(row: &Value) -> bool {
    let mode = row["mode"].as_str().unwrap_or("");
    !matches!(
        mode,
        "responding" | "working" | "thinking" | "streaming" | "running" | "stopped"
    ) && row["status"] != "ended"
        && row["pendingApproval"].is_null()
        && row["pendingQuestions"].is_null()
}

fn live(row: &Value) -> bool {
    row["mode"] != "stopped"
        && row["status"] != "ended"
        && row["hub"].as_str().is_none_or(str::is_empty)
}

/// The agent's message for `fresh` comments: where each one is, who wrote
/// it, and what to do.
fn message(link: &Link, threads: &Threads, fresh: &[(String, String)]) -> String {
    let pr = link.pull_request.as_ref();
    let mut out = format!(
        "[pull request] New review comments on {} {}:\n",
        pr.map_or("the linked pull request".to_owned(), |pr| format!(
            "{} #{}",
            pr.repo, pr.number
        )),
        link.url.as_deref().unwrap_or_default()
    );
    for thread in &threads.threads {
        let comments: Vec<_> = thread
            .comments
            .iter()
            .filter(|c| fresh.iter().any(|(t, id)| *t == thread.id && *id == c.id))
            .collect();
        if comments.is_empty() {
            continue;
        }
        let place = match (&thread.path, thread.line) {
            (Some(path), Some(line)) => format!("{path}:{line}"),
            (Some(path), None) => path.clone(),
            _ => "conversation".into(),
        };
        out.push_str(&format!("\n- {place} (thread {})\n", thread.id));
        for comment in comments {
            for (i, line) in comment.body.lines().enumerate() {
                if i == 0 {
                    out.push_str(&format!("  > {}: {line}\n", comment.author));
                } else {
                    out.push_str(&format!("  > {line}\n"));
                }
            }
        }
        if out.len() > MESSAGE_LIMIT {
            out.truncate(MESSAGE_LIMIT);
            out.push_str("\n  … (more in the pull_request tool)\n");
            break;
        }
    }
    out.push_str(
        "\nRead the full threads with the pull_request tool. Address each comment, \
         reply_pull_request with what you changed, and resolve_pull_request_thread \
         once it is fully handled.",
    );
    out
}

impl Sessions {
    pub(super) async fn run_pull_request_watch(self: Arc<Self>) {
        let mut state = WatchState::default();
        let mut tick = tokio::time::interval(TICK);
        tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
        loop {
            tick.tick().await;
            self.watch_pull_requests(&mut state).await;
        }
    }

    pub(super) async fn watch_pull_requests(&self, state: &mut WatchState) {
        let Some(service) = self.pull_requests.clone() else {
            return;
        };
        let settings = service.settings();
        let rows: Vec<(String, Value)> = self
            .rows
            .read()
            .unwrap()
            .iter()
            .filter(|(_, row)| live(row))
            .map(|(id, row)| (id.clone(), row.clone()))
            .collect();
        let now = Instant::now();
        for (id, row) in rows {
            match service.links.get(&id) {
                None if settings.auto_link => {
                    let cwd = row["cwd"].as_str().unwrap_or_default().to_owned();
                    if cwd.is_empty() || !due(&state.branch_checked, &cwd, BRANCH_EVERY, now) {
                        continue;
                    }
                    state.branch_checked.insert(cwd.clone(), now);
                    let Ok((_, Some(found))) = service.for_cwd(std::path::Path::new(&cwd)).await
                    else {
                        continue;
                    };
                    let mut link = Link::new(found.url.clone(), Source::Branch, Some(found));
                    // What is already there is not news.
                    if let Ok(pr) = service.parse(link.url.as_deref().unwrap_or_default(), None)
                        && let Ok(threads) = service.threads(&pr).await
                    {
                        link.seen = threads.comment_ids().collect();
                        link.unresolved = Some(threads.unresolved());
                        link.checked_at = Some(chrono::Utc::now().timestamp_millis());
                    }
                    if service.links.get(&id).is_none() && service.links.set(&id, link).is_ok() {
                        self.pull_request_changed(&id).await;
                    }
                }
                Some(link) => {
                    let Some(url) = link.url.clone() else {
                        continue;
                    };
                    let settled = link.pull_request.as_ref().is_some_and(|pr| {
                        matches!(
                            pr.state,
                            super::super::pull_requests::model::State::Merged
                                | super::super::pull_requests::model::State::Closed
                        )
                    });
                    let every = if settled {
                        SETTLED_EVERY
                    } else {
                        THREADS_EVERY
                    };
                    if !due(&state.threads_checked, &url, every, now) {
                        continue;
                    }
                    state.threads_checked.insert(url.clone(), now);
                    self.refresh_link(&id, &row, link, settings.notify, state)
                        .await;
                }
                None => {}
            }
        }
    }

    async fn refresh_link(
        &self,
        id: &str,
        row: &Value,
        link: Link,
        notify: bool,
        state: &mut WatchState,
    ) {
        let Some(service) = self.pull_requests.clone() else {
            return;
        };
        let Ok(pr) = service.parse(link.url.as_deref().unwrap_or_default(), None) else {
            return;
        };
        let (details, threads) = tokio::join!(service.pull_request(&pr), service.threads(&pr));
        let Ok(threads) = threads else {
            return;
        };
        let first = link.checked_at.is_none();
        let fresh: Vec<(String, String)> = if first {
            Vec::new()
        } else {
            threads
                .threads
                .iter()
                .flat_map(|t| t.comments.iter().map(move |c| (t.id.clone(), c.id.clone())))
                .filter(|(_, comment)| {
                    !link.seen.contains(comment) && !service.posted_by_us(comment)
                })
                .collect()
        };
        let now = Instant::now();
        let deliver =
            notify && !fresh.is_empty() && idle(row) && due(&state.notified, id, NOTIFY_GAP, now);
        let mut delivered = false;
        if deliver {
            let text = message(&link, &threads, &fresh);
            let outcome = self
                .send_message(id, Some(&text), Delivery::default(), None, None, &[])
                .await;
            delivered = matches!(outcome, super::send::MessageOutcome::Delivered(_));
            if delivered {
                state.notified.insert(id.to_owned(), now);
            }
        }
        let before = link.wire();
        let changed = service.links.modify(id, |current| {
            if current.url != link.url {
                return;
            }
            if let Ok(details) = &details {
                current.pull_request = Some(details.clone());
            }
            current.unresolved = Some(threads.unresolved());
            current.checked_at = Some(chrono::Utc::now().timestamp_millis());
            // Read on first sight; otherwise read once delivered, or when the
            // user turned notifications off.
            if first || delivered || !notify {
                current.seen.extend(threads.comment_ids());
            }
        });
        let after = service.links.get(id).and_then(|l| l.wire());
        if changed.is_ok() && after != before {
            self.pull_request_changed(id).await;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::services::pull_requests::model::{Comment, Thread};
    use serde_json::json;

    #[test]
    fn busy_or_blocked_sessions_are_not_idle() {
        assert!(idle(&json!({"mode": "input"})));
        assert!(!idle(&json!({"mode": "responding"})));
        assert!(!idle(
            &json!({"mode": "input", "pendingApproval": {"toolName": "Bash"}})
        ));
        assert!(!live(&json!({"mode": "input", "hub": "peer"})));
    }

    #[test]
    fn the_message_quotes_only_fresh_comments() {
        let threads = Threads {
            truncated: false,
            threads: vec![Thread {
                id: "T1".into(),
                path: Some("src/lib.rs".into()),
                line: Some(4),
                resolvable: true,
                resolved: false,
                outdated: false,
                comments: vec![
                    Comment {
                        id: "old".into(),
                        author: "rev".into(),
                        body: "Seen before".into(),
                        created_at: None,
                        url: None,
                    },
                    Comment {
                        id: "new".into(),
                        author: "rev".into(),
                        body: "Also rename the test\nplease".into(),
                        created_at: None,
                        url: None,
                    },
                ],
            }],
        };
        let link = Link::new("https://github.com/o/r/pull/3".into(), Source::Agent, None);
        let text = message(&link, &threads, &[("T1".into(), "new".into())]);
        assert!(text.contains("- src/lib.rs:4 (thread T1)"));
        assert!(text.contains("  > rev: Also rename the test\n  > please"));
        assert!(!text.contains("Seen before"));
        assert!(text.contains("resolve_pull_request_thread"));
    }
}
