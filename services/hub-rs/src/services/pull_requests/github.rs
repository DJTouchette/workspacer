//! GitHub through its GraphQL API: one query shape for pull requests, review
//! threads (which carry resolution, unlike REST review comments) and the
//! general conversation.
use super::{
    GitProvider,
    credentials::Credentials,
    http,
    model::{Comment, PullRequest, State, Thread, Threads},
    remote::{Kind, PrRef, Repo},
};
use anyhow::{Result, anyhow, bail};
use serde_json::{Value, json};
use std::sync::Arc;

/// The id the general conversation answers to as a thread.
pub(crate) const CONVERSATION: &str = "conversation";

const PR_FIELDS: &str =
    "id number title url state isDraft headRefName baseRefName author { login }";
const COMMENT_FIELDS: &str = "id author { login } body createdAt url";

pub(crate) struct GitHub {
    client: reqwest::Client,
    credentials: Arc<Credentials>,
    /// Tests point every host at a local server.
    api: Option<String>,
}

impl GitHub {
    pub(crate) fn new(credentials: Arc<Credentials>, api: Option<String>) -> Self {
        Self {
            client: http::client(),
            credentials,
            api,
        }
    }

    fn endpoint(&self, host: &str) -> String {
        match &self.api {
            Some(api) => format!("{api}/graphql"),
            None if host == "github.com" => "https://api.github.com/graphql".into(),
            None => format!("https://{host}/api/graphql"),
        }
    }

    async fn query(&self, host: &str, query: &str, variables: Value) -> Result<Value> {
        let endpoint = self.endpoint(host);
        let body = json!({"query": query, "variables": variables});
        let reply = http::json(&self.credentials, Kind::Github, host, || {
            self.client.post(&endpoint).json(&body)
        })
        .await?;
        if let Some(error) = reply["errors"].as_array().and_then(|e| e.first()) {
            bail!(
                "GitHub: {}",
                error["message"].as_str().unwrap_or("request failed")
            );
        }
        Ok(reply["data"].clone())
    }

    fn repo_vars(repo: &Repo) -> Value {
        json!({"owner": repo.owner, "name": repo.name})
    }

    async fn node_id(&self, pr: &PrRef) -> Result<String> {
        let data = self
            .query(
                &pr.repo.host,
                "query($owner:String!,$name:String!,$number:Int!){repository(owner:$owner,name:$name){pullRequest(number:$number){id}}}",
                json!({"owner": pr.repo.owner, "name": pr.repo.name, "number": pr.number}),
            )
            .await?;
        data["repository"]["pullRequest"]["id"]
            .as_str()
            .map(str::to_owned)
            .ok_or_else(|| anyhow!("GitHub has no pull request #{}", pr.number))
    }
}

impl GitProvider for GitHub {
    async fn pull_request(&self, pr: &PrRef) -> Result<PullRequest> {
        let data = self
            .query(
                &pr.repo.host,
                &format!("query($owner:String!,$name:String!,$number:Int!){{repository(owner:$owner,name:$name){{pullRequest(number:$number){{{PR_FIELDS}}}}}}}"),
                json!({"owner": pr.repo.owner, "name": pr.repo.name, "number": pr.number}),
            )
            .await?;
        let node = &data["repository"]["pullRequest"];
        if node.is_null() {
            bail!(
                "GitHub has no pull request #{} in {}",
                pr.number,
                pr.repo.display()
            );
        }
        Ok(pull_request(&pr.repo, node))
    }

    async fn for_branch(&self, repo: &Repo, branch: &str) -> Result<Option<PullRequest>> {
        let mut variables = Self::repo_vars(repo);
        variables["branch"] = json!(branch);
        let data = self
            .query(
                &repo.host,
                &format!("query($owner:String!,$name:String!,$branch:String!){{repository(owner:$owner,name:$name){{pullRequests(headRefName:$branch,first:1,states:[OPEN],orderBy:{{field:UPDATED_AT,direction:DESC}}){{nodes{{{PR_FIELDS}}}}}}}}}"),
                variables,
            )
            .await?;
        Ok(data["repository"]["pullRequests"]["nodes"]
            .as_array()
            .and_then(|nodes| nodes.first())
            .map(|node| pull_request(repo, node)))
    }

    async fn threads(&self, pr: &PrRef) -> Result<Threads> {
        let data = self
            .query(
                &pr.repo.host,
                &format!("query($owner:String!,$name:String!,$number:Int!){{repository(owner:$owner,name:$name){{pullRequest(number:$number){{reviewThreads(first:100){{pageInfo{{hasNextPage}} nodes{{id isResolved isOutdated path line originalLine comments(first:100){{nodes{{{COMMENT_FIELDS}}}}}}}}} comments(first:100){{pageInfo{{hasNextPage}} nodes{{{COMMENT_FIELDS}}}}}}}}}}}"),
                json!({"owner": pr.repo.owner, "name": pr.repo.name, "number": pr.number}),
            )
            .await?;
        let node = &data["repository"]["pullRequest"];
        if node.is_null() {
            bail!(
                "GitHub has no pull request #{} in {}",
                pr.number,
                pr.repo.display()
            );
        }
        Ok(threads(node))
    }

    async fn reply(&self, pr: &PrRef, thread: &str, body: &str) -> Result<Comment> {
        if thread == CONVERSATION {
            return self.comment(pr, body).await;
        }
        let data = self
            .query(
                &pr.repo.host,
                &format!("mutation($thread:ID!,$body:String!){{addPullRequestReviewThreadReply(input:{{pullRequestReviewThreadId:$thread,body:$body}}){{comment{{{COMMENT_FIELDS}}}}}}}"),
                json!({"thread": thread, "body": body}),
            )
            .await?;
        Ok(comment(&data["addPullRequestReviewThreadReply"]["comment"]))
    }

    async fn comment(&self, pr: &PrRef, body: &str) -> Result<Comment> {
        let subject = self.node_id(pr).await?;
        let data = self
            .query(
                &pr.repo.host,
                &format!("mutation($subject:ID!,$body:String!){{addComment(input:{{subjectId:$subject,body:$body}}){{commentEdge{{node{{{COMMENT_FIELDS}}}}}}}}}"),
                json!({"subject": subject, "body": body}),
            )
            .await?;
        Ok(comment(&data["addComment"]["commentEdge"]["node"]))
    }

    async fn set_resolved(&self, pr: &PrRef, thread: &str, resolved: bool) -> Result<()> {
        if thread == CONVERSATION {
            bail!("the pull request's conversation cannot be resolved");
        }
        let mutation = if resolved {
            "resolveReviewThread"
        } else {
            "unresolveReviewThread"
        };
        self.query(
            &pr.repo.host,
            &format!("mutation($thread:ID!){{{mutation}(input:{{threadId:$thread}}){{thread{{id isResolved}}}}}}"),
            json!({"thread": thread}),
        )
        .await?;
        Ok(())
    }
}

fn login(node: &Value) -> String {
    node["author"]["login"]
        .as_str()
        .unwrap_or("ghost")
        .to_owned()
}

fn text(node: &Value, key: &str) -> String {
    node[key].as_str().unwrap_or_default().to_owned()
}

fn pull_request(repo: &Repo, node: &Value) -> PullRequest {
    let state = match node["state"].as_str() {
        Some("MERGED") => State::Merged,
        Some("CLOSED") => State::Closed,
        _ if node["isDraft"] == true => State::Draft,
        _ => State::Open,
    };
    let number = node["number"].as_u64().unwrap_or_default();
    PullRequest {
        provider: Kind::Github,
        url: node["url"]
            .as_str()
            .map_or_else(|| repo.pull_request_url(number), str::to_owned),
        repo: repo.display(),
        number,
        title: text(node, "title"),
        state,
        source_branch: text(node, "headRefName"),
        target_branch: text(node, "baseRefName"),
        author: login(node),
    }
}

fn comment(node: &Value) -> Comment {
    Comment {
        id: text(node, "id"),
        author: login(node),
        body: text(node, "body"),
        created_at: node["createdAt"].as_str().map(str::to_owned),
        url: node["url"].as_str().map(str::to_owned),
    }
}

fn comments(connection: &Value) -> Vec<Comment> {
    connection["nodes"]
        .as_array()
        .map(|nodes| nodes.iter().map(comment).collect())
        .unwrap_or_default()
}

fn threads(node: &Value) -> Threads {
    let review = &node["reviewThreads"];
    let conversation = &node["comments"];
    let mut threads: Vec<Thread> = review["nodes"]
        .as_array()
        .map(|nodes| {
            nodes
                .iter()
                .map(|thread| Thread {
                    id: text(thread, "id"),
                    path: thread["path"].as_str().map(str::to_owned),
                    line: thread["line"]
                        .as_u64()
                        .or_else(|| thread["originalLine"].as_u64()),
                    resolvable: true,
                    resolved: thread["isResolved"] == true,
                    outdated: thread["isOutdated"] == true,
                    comments: comments(&thread["comments"]),
                })
                .collect()
        })
        .unwrap_or_default();
    let general = comments(conversation);
    if !general.is_empty() {
        threads.insert(
            0,
            Thread {
                id: CONVERSATION.into(),
                path: None,
                line: None,
                resolvable: false,
                resolved: false,
                outdated: false,
                comments: general,
            },
        );
    }
    Threads {
        threads,
        truncated: review["pageInfo"]["hasNextPage"] == true
            || conversation["pageInfo"]["hasNextPage"] == true,
    }
}
