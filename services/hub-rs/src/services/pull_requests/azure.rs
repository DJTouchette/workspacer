//! Azure DevOps through its Git REST API (7.1). Pull request threads carry
//! a status: `active` and `pending` are open; `fixed`, `wontFix`, `closed`
//! and `byDesign` count as resolved. Threads made only of system comments
//! (votes, pushes, policy updates) are not discussion and are left out.
use super::{
    GitProvider,
    credentials::Credentials,
    http,
    model::{Comment, PullRequest, State, Thread, Threads},
    remote::{Kind, PrRef, Repo, encode},
};
use anyhow::{Result, bail};
use serde_json::{Value, json};
use std::sync::Arc;

const API_VERSION: &str = "7.1";

pub(crate) struct Azure {
    client: reqwest::Client,
    credentials: Arc<Credentials>,
    /// Tests point every organisation at a local server.
    base: Option<String>,
}

impl Azure {
    pub(crate) fn new(credentials: Arc<Credentials>, base: Option<String>) -> Self {
        Self {
            client: http::client(),
            credentials,
            base,
        }
    }

    fn repository(&self, repo: &Repo) -> String {
        format!(
            "{}/{}/{}/_apis/git/repositories/{}",
            self.base
                .clone()
                .unwrap_or_else(|| format!("https://{}", repo.host)),
            encode(&repo.owner),
            encode(repo.project.as_deref().unwrap_or_default()),
            encode(&repo.name)
        )
    }

    async fn send(
        &self,
        repo: &Repo,
        method: reqwest::Method,
        path: &str,
        query: &[(&str, &str)],
        body: Option<Value>,
    ) -> Result<Value> {
        let url = format!("{}{path}", self.repository(repo));
        http::json(&self.credentials, Kind::AzureDevops, &repo.host, || {
            let request = self
                .client
                .request(method.clone(), &url)
                .query(&[("api-version", API_VERSION)])
                .query(query);
            match &body {
                Some(body) => request.json(body),
                None => request,
            }
        })
        .await
    }
}

impl GitProvider for Azure {
    async fn pull_request(&self, pr: &PrRef) -> Result<PullRequest> {
        let value = self
            .send(
                &pr.repo,
                reqwest::Method::GET,
                &format!("/pullrequests/{}", pr.number),
                &[],
                None,
            )
            .await?;
        Ok(pull_request(&pr.repo, &value))
    }

    async fn for_branch(&self, repo: &Repo, branch: &str) -> Result<Option<PullRequest>> {
        let source = format!("refs/heads/{branch}");
        let value = self
            .send(
                repo,
                reqwest::Method::GET,
                "/pullrequests",
                &[
                    ("searchCriteria.sourceRefName", &source),
                    ("searchCriteria.status", "active"),
                    ("$top", "1"),
                ],
                None,
            )
            .await?;
        Ok(value["value"]
            .as_array()
            .and_then(|prs| prs.first())
            .map(|pr| pull_request(repo, pr)))
    }

    async fn threads(&self, pr: &PrRef) -> Result<Threads> {
        let value = self
            .send(
                &pr.repo,
                reqwest::Method::GET,
                &format!("/pullrequests/{}/threads", pr.number),
                &[],
                None,
            )
            .await?;
        Ok(threads(&value))
    }

    async fn reply(&self, pr: &PrRef, thread: &str, body: &str) -> Result<Comment> {
        if !thread.chars().all(|c| c.is_ascii_digit()) || thread.is_empty() {
            bail!("invalid Azure DevOps thread id");
        }
        let value = self
            .send(
                &pr.repo,
                reqwest::Method::POST,
                &format!("/pullrequests/{}/threads/{thread}/comments", pr.number),
                &[],
                Some(json!({"content": body, "parentCommentId": 1, "commentType": 1})),
            )
            .await?;
        Ok(comment(&value))
    }

    async fn comment(&self, pr: &PrRef, body: &str) -> Result<Comment> {
        let value = self
            .send(
                &pr.repo,
                reqwest::Method::POST,
                &format!("/pullrequests/{}/threads", pr.number),
                &[],
                Some(json!({
                    "comments": [{"parentCommentId": 0, "content": body, "commentType": 1}],
                    "status": "active",
                })),
            )
            .await?;
        Ok(value["comments"]
            .as_array()
            .and_then(|c| c.first())
            .map(comment)
            .unwrap_or_else(|| comment(&Value::Null)))
    }

    async fn set_resolved(&self, pr: &PrRef, thread: &str, resolved: bool) -> Result<()> {
        if !thread.chars().all(|c| c.is_ascii_digit()) || thread.is_empty() {
            bail!("invalid Azure DevOps thread id");
        }
        self.send(
            &pr.repo,
            reqwest::Method::PATCH,
            &format!("/pullrequests/{}/threads/{thread}", pr.number),
            &[],
            Some(json!({"status": if resolved { "fixed" } else { "active" }})),
        )
        .await?;
        Ok(())
    }
}

fn text(node: &Value, key: &str) -> String {
    node[key].as_str().unwrap_or_default().to_owned()
}

fn branch(reference: &str) -> String {
    reference
        .strip_prefix("refs/heads/")
        .unwrap_or(reference)
        .to_owned()
}

fn pull_request(repo: &Repo, node: &Value) -> PullRequest {
    let number = node["pullRequestId"].as_u64().unwrap_or_default();
    let state = match node["status"].as_str() {
        Some("completed") => State::Merged,
        Some("abandoned") => State::Closed,
        _ if node["isDraft"] == true => State::Draft,
        _ => State::Open,
    };
    PullRequest {
        provider: Kind::AzureDevops,
        url: repo.pull_request_url(number),
        repo: repo.display(),
        number,
        title: text(node, "title"),
        state,
        source_branch: branch(node["sourceRefName"].as_str().unwrap_or_default()),
        target_branch: branch(node["targetRefName"].as_str().unwrap_or_default()),
        author: node["createdBy"]["displayName"]
            .as_str()
            .unwrap_or_default()
            .to_owned(),
    }
}

fn comment(node: &Value) -> Comment {
    Comment {
        id: node["id"]
            .as_u64()
            .map(|id| id.to_string())
            .unwrap_or_default(),
        author: node["author"]["displayName"]
            .as_str()
            .unwrap_or_default()
            .to_owned(),
        body: text(node, "content"),
        created_at: node["publishedDate"].as_str().map(str::to_owned),
        url: None,
    }
}

fn threads(value: &Value) -> Threads {
    let threads = value["value"]
        .as_array()
        .map(|threads| {
            threads
                .iter()
                .filter(|t| t["isDeleted"] != true)
                .filter_map(|thread| {
                    let comments: Vec<Comment> = thread["comments"]
                        .as_array()?
                        .iter()
                        .filter(|c| c["isDeleted"] != true && c["commentType"] != "system")
                        .map(comment)
                        .collect();
                    if comments.is_empty() {
                        return None;
                    }
                    let status = thread["status"].as_str().unwrap_or("unknown");
                    let context = &thread["threadContext"];
                    Some(Thread {
                        id: thread["id"].as_u64()?.to_string(),
                        path: context["filePath"]
                            .as_str()
                            .map(|p| p.trim_start_matches('/').to_owned()),
                        line: context["rightFileStart"]["line"]
                            .as_u64()
                            .or_else(|| context["leftFileStart"]["line"].as_u64()),
                        resolvable: status != "unknown",
                        resolved: matches!(status, "fixed" | "wontFix" | "closed" | "byDesign"),
                        outdated: false,
                        comments,
                    })
                })
                .collect()
        })
        .unwrap_or_default();
    Threads {
        threads,
        truncated: false,
    }
}
