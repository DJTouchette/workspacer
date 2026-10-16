//! The pull request tools: the caller's session (or `sessionId`) names the
//! linked pull request, so an agent never has to carry its URL around.
use crate::client::Client;
use anyhow::{Result, anyhow, bail};
use serde_json::{Value, json};

pub(super) fn handles(name: &str) -> bool {
    matches!(
        name,
        "pull_request"
            | "link_pull_request"
            | "reply_pull_request"
            | "resolve_pull_request_thread"
            | "comment_pull_request"
    )
}

/// The bus method each tool is authorized as.
pub(super) fn method(name: &str) -> Option<&'static str> {
    Some(match name {
        "pull_request" => "pr.threads",
        "link_pull_request" => "pr.link",
        "reply_pull_request" => "pr.reply",
        "resolve_pull_request_thread" => "pr.resolve",
        "comment_pull_request" => "pr.comment",
        _ => return None,
    })
}

fn text<'a>(params: &'a Value, key: &str) -> Result<&'a str> {
    params[key]
        .as_str()
        .map(str::trim)
        .filter(|t| !t.is_empty())
        .ok_or_else(|| anyhow!("{key} is required"))
}

async fn linked(client: &Client, session: &str) -> Result<Option<Value>> {
    let reply = client
        .call("pr.session", json!({"sessionId": session}))
        .await?;
    Ok(Some(reply["pullRequest"].clone()).filter(|link| link["url"].is_string()))
}

async fn url(client: &Client, session: &str) -> Result<String> {
    linked(client, session)
        .await?
        .and_then(|link| link["url"].as_str().map(str::to_owned))
        .ok_or_else(|| {
            anyhow!(
                "no pull request is linked to this session; open one and call link_pull_request"
            )
        })
}

pub(super) async fn call(
    client: &Client,
    name: &str,
    params: Value,
    caller: &str,
) -> Result<Value> {
    let session = params["sessionId"]
        .as_str()
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .unwrap_or(caller)
        .to_owned();
    if session.is_empty() {
        bail!("sessionId is required when the caller is not a session");
    }
    match name {
        "pull_request" => {
            let Some(link) = linked(client, &session).await? else {
                let snapshot = client
                    .call("sessions.snapshot", json!({"sessionId": session}))
                    .await?;
                let cwd = snapshot["cwd"]
                    .as_str()
                    .or_else(|| snapshot["snapshot"]["cwd"].as_str())
                    .unwrap_or_default();
                let suggestion = if cwd.is_empty() {
                    Value::Null
                } else {
                    client
                        .call("pr.forBranch", json!({"cwd": cwd}))
                        .await
                        .unwrap_or_else(|error| json!({"error": error.to_string()}))
                };
                return Ok(json!({
                    "linked": false,
                    "suggestion": suggestion["pullRequest"],
                    "branch": suggestion["detected"]["branch"],
                    "problem": suggestion["detected"]["problem"].as_str().or(suggestion["error"].as_str()),
                    "next": "link the pull request for this work with link_pull_request",
                }));
            };
            let url = link["url"].as_str().unwrap_or_default();
            let mut threads = client.call("pr.threads", json!({"url": url})).await?;
            if params["includeResolved"] != true
                && let Some(list) = threads["threads"].as_array_mut()
            {
                list.retain(|t| t["resolved"] != true);
            }
            Ok(json!({
                "linked": true,
                "pullRequest": link,
                "unresolved": threads["unresolved"],
                "truncated": threads["truncated"],
                "threads": threads["threads"],
            }))
        }
        "link_pull_request" => {
            client
                .call(
                    "pr.link",
                    json!({"sessionId": session, "url": text(&params, "url")?, "source": "agent"}),
                )
                .await
        }
        "reply_pull_request" => {
            let url = url(client, &session).await?;
            client
                .call(
                    "pr.reply",
                    json!({"url": url, "threadId": text(&params, "threadId")?, "body": text(&params, "body")?}),
                )
                .await
        }
        "resolve_pull_request_thread" => {
            let url = url(client, &session).await?;
            client
                .call(
                    "pr.resolve",
                    json!({
                        "url": url,
                        "threadId": text(&params, "threadId")?,
                        "resolved": params["resolved"].as_bool().unwrap_or(true),
                    }),
                )
                .await
        }
        "comment_pull_request" => {
            let url = url(client, &session).await?;
            client
                .call(
                    "pr.comment",
                    json!({"url": url, "body": text(&params, "body")?}),
                )
                .await
        }
        _ => bail!("unknown pull request tool {name}"),
    }
}
