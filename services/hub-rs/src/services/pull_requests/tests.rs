//! Both providers against a local stand-in for their APIs, and detection
//! against a real repository.
use super::{Service, links::Links, model::State, project_git, remote::Kind};
use serde_json::{Value, json};
use std::sync::{Arc, Mutex};

#[derive(Clone, Debug)]
struct Seen {
    method: String,
    path: String,
    query: String,
    auth: String,
    body: Value,
}

type Reply = Arc<dyn Fn(&Seen) -> (u16, Value) + Send + Sync>;

/// A server that records every request and answers with `reply`.
async fn fake(reply: Reply) -> (Service, Arc<Mutex<Vec<Seen>>>) {
    let seen = Arc::new(Mutex::new(Vec::<Seen>::new()));
    let log = seen.clone();
    let app = axum::Router::new().fallback(move |request: axum::extract::Request| {
        let (log, reply) = (log.clone(), reply.clone());
        async move {
            let (parts, body) = request.into_parts();
            let bytes = axum::body::to_bytes(body, 1 << 20).await.unwrap();
            let request = Seen {
                method: parts.method.to_string(),
                path: parts.uri.path().to_owned(),
                query: parts.uri.query().unwrap_or_default().to_owned(),
                auth: parts
                    .headers
                    .get("authorization")
                    .and_then(|v| v.to_str().ok())
                    .unwrap_or_default()
                    .to_owned(),
                body: serde_json::from_slice(&bytes).unwrap_or(Value::Null),
            };
            let (status, value) = reply(&request);
            log.lock().unwrap().push(request);
            (
                axum::http::StatusCode::from_u16(status).unwrap(),
                axum::Json(value),
            )
        }
    });
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let base = format!("http://{}", listener.local_addr().unwrap());
    tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    (Service::for_tests(&base, Arc::new(Links::memory())), seen)
}

fn query_text(seen: &Seen) -> &str {
    seen.body["query"].as_str().unwrap_or_default()
}

#[tokio::test]
async fn github_threads_map_review_threads_and_the_conversation() {
    let (service, seen) = fake(Arc::new(|_| {
        (200, json!({"data": {"repository": {"pullRequest": {
            "reviewThreads": {"pageInfo": {"hasNextPage": false}, "nodes": [
                {"id": "T1", "isResolved": false, "isOutdated": true, "path": "src/lib.rs",
                 "line": null, "originalLine": 12,
                 "comments": {"nodes": [{"id": "C1", "author": {"login": "rev"},
                    "body": "Rename this", "createdAt": "2026-10-16T10:00:00Z", "url": "https://x/c1"}]}},
                {"id": "T2", "isResolved": true, "isOutdated": false, "path": "README.md", "line": 3,
                 "comments": {"nodes": [{"id": "C2", "author": null, "body": "ok"}]}}
            ]},
            "comments": {"pageInfo": {"hasNextPage": true}, "nodes": [
                {"id": "IC1", "author": {"login": "lead"}, "body": "Looks close"}
            ]}
        }}}}))
    }))
    .await;
    let pr = service
        .parse("https://github.com/o/r/pull/9", None)
        .unwrap();
    let threads = service.threads(&pr).await.unwrap();
    assert!(threads.truncated, "the conversation had another page");
    assert_eq!(threads.unresolved(), 1);
    let ids: Vec<_> = threads.threads.iter().map(|t| t.id.as_str()).collect();
    assert_eq!(ids, ["conversation", "T1", "T2"]);
    let t1 = &threads.threads[1];
    assert_eq!(
        (t1.path.as_deref(), t1.line, t1.outdated, t1.resolved),
        (Some("src/lib.rs"), Some(12), true, false)
    );
    assert_eq!(threads.threads[2].comments[0].author, "ghost");
    assert!(!threads.threads[0].resolvable);
    let seen = seen.lock().unwrap();
    assert_eq!(seen[0].path, "/github/graphql");
    assert_eq!(seen[0].auth, "Bearer gh-token");
    assert_eq!(
        seen[0].body["variables"],
        json!({"owner":"o","name":"r","number":9})
    );
}

#[tokio::test]
async fn github_pull_request_states_and_branch_lookup() {
    let (service, seen) = fake(Arc::new(|request| {
        let node = json!({"id": "PR_1", "number": 4, "title": "Add PR links",
            "url": "https://github.com/o/r/pull/4", "state": "OPEN", "isDraft": true,
            "headRefName": "feature/pr", "baseRefName": "main", "author": {"login": "dj"}});
        if query_text(request).contains("pullRequests(") {
            (
                200,
                json!({"data": {"repository": {"pullRequests": {"nodes": [node]}}}}),
            )
        } else {
            (200, json!({"data": {"repository": {"pullRequest": node}}}))
        }
    }))
    .await;
    let pr = service
        .parse("https://github.com/o/r/pull/4", None)
        .unwrap();
    let details = service.pull_request(&pr).await.unwrap();
    assert_eq!(details.state, State::Draft);
    assert_eq!(
        (
            details.source_branch.as_str(),
            details.target_branch.as_str()
        ),
        ("feature/pr", "main")
    );
    let found = with_provider!(service, Kind::Github, |p| {
        super::GitProvider::for_branch(p, &pr.repo, "feature/pr").await
    })
    .unwrap()
    .unwrap();
    assert_eq!(found.number, 4);
    let seen = seen.lock().unwrap();
    assert_eq!(seen[1].body["variables"]["branch"], "feature/pr");
}

#[tokio::test]
async fn github_conversation_replies_become_top_level_comments() {
    let (service, seen) = fake(Arc::new(|request| {
        if query_text(request).contains("addComment") {
            (
                200,
                json!({"data": {"addComment": {"commentEdge": {"node":
                {"id": "IC9", "author": {"login": "dj"}, "body": "Done"}}}}}),
            )
        } else {
            (
                200,
                json!({"data": {"repository": {"pullRequest": {"id": "PR_NODE"}}}}),
            )
        }
    }))
    .await;
    let pr = service
        .parse("https://github.com/o/r/pull/2", None)
        .unwrap();
    let comment = service.reply(&pr, "conversation", "Done").await.unwrap();
    assert_eq!(comment.id, "IC9");
    {
        let seen = seen.lock().unwrap();
        assert_eq!(
            seen[1].body["variables"],
            json!({"subject":"PR_NODE","body":"Done"})
        );
    }
    let refused = service.set_resolved(&pr, "conversation", true).await;
    assert!(
        refused
            .unwrap_err()
            .to_string()
            .contains("cannot be resolved")
    );
}

#[tokio::test]
async fn github_errors_surface_the_graphql_message() {
    let (service, _) = fake(Arc::new(|_| {
        (
            200,
            json!({"errors": [{"message": "Could not resolve to a Repository"}]}),
        )
    }))
    .await;
    let pr = service
        .parse("https://github.com/o/missing/pull/1", None)
        .unwrap();
    let error = service.pull_request(&pr).await.unwrap_err().to_string();
    assert!(
        error.contains("Could not resolve to a Repository"),
        "{error}"
    );
}

const ADO_PR: &str = "https://dev.azure.com/contoso/Fabrikam%20Fiber/_git/web/pullrequest/31";

#[tokio::test]
async fn azure_threads_skip_system_noise_and_read_status() {
    let (service, seen) = fake(Arc::new(|_| {
        (200, json!({"value": [
            {"id": 1, "status": "active", "threadContext": {"filePath": "/src/app.ts",
                "rightFileStart": {"line": 40}},
             "comments": [{"id": 1, "author": {"displayName": "Rev"}, "content": "Null check?",
                "commentType": "text", "publishedDate": "2026-10-16T09:00:00Z"}]},
            {"id": 2, "status": "fixed", "comments": [
                {"id": 1, "author": {"displayName": "Lead"}, "content": "Ship it", "commentType": "text"}]},
            {"id": 3, "comments": [{"id": 1, "content": "Rev voted 10", "commentType": "system"}]},
            {"id": 4, "isDeleted": true, "status": "active", "comments": [
                {"id": 1, "content": "gone", "commentType": "text"}]}
        ]}))
    }))
    .await;
    let pr = service.parse(ADO_PR, None).unwrap();
    let threads = service.threads(&pr).await.unwrap();
    let ids: Vec<_> = threads.threads.iter().map(|t| t.id.as_str()).collect();
    assert_eq!(ids, ["1", "2"]);
    assert_eq!(threads.threads[0].path.as_deref(), Some("src/app.ts"));
    assert_eq!(threads.threads[0].line, Some(40));
    assert!(threads.threads[1].resolved);
    assert_eq!(threads.unresolved(), 1);
    let seen = seen.lock().unwrap();
    assert_eq!(
        seen[0].path,
        "/azure/contoso/Fabrikam%20Fiber/_apis/git/repositories/web/pullrequests/31/threads"
    );
    assert_eq!(seen[0].query, "api-version=7.1");
    assert_eq!(seen[0].auth, "Basic ado-token");
}

#[tokio::test]
async fn azure_writes_reply_resolve_and_find_by_branch() {
    let (service, seen) = fake(Arc::new(|request| match request.method.as_str() {
        "GET" => (
            200,
            json!({"value": [{"pullRequestId": 31, "title": "Feature",
            "status": "active", "isDraft": false, "sourceRefName": "refs/heads/feature/x",
            "targetRefName": "refs/heads/main", "createdBy": {"displayName": "DJ"}}]}),
        ),
        "POST" => (
            200,
            json!({"id": 2, "author": {"displayName": "DJ"}, "content": "Fixed"}),
        ),
        _ => (200, json!({"id": 1, "status": "fixed"})),
    }))
    .await;
    let pr = service.parse(ADO_PR, None).unwrap();
    let found = with_provider!(service, Kind::AzureDevops, |p| {
        super::GitProvider::for_branch(p, &pr.repo, "feature/x").await
    })
    .unwrap()
    .unwrap();
    assert_eq!(
        (found.number, found.state, found.source_branch.as_str()),
        (31, State::Open, "feature/x")
    );
    assert_eq!(found.url, ADO_PR);
    let reply = service.reply(&pr, "1", "Fixed").await.unwrap();
    assert_eq!(reply.id, "2");
    service.set_resolved(&pr, "1", true).await.unwrap();
    assert!(service.reply(&pr, "1/../2", "x").await.is_err());
    let seen = seen.lock().unwrap();
    assert!(
        seen[0]
            .query
            .contains("searchCriteria.sourceRefName=refs%2Fheads%2Ffeature%2Fx")
    );
    assert!(
        seen[1]
            .path
            .ends_with("/pullrequests/31/threads/1/comments")
    );
    assert_eq!(seen[1].body["content"], "Fixed");
    assert_eq!(seen[2].method, "PATCH");
    assert_eq!(seen[2].body, json!({"status": "fixed"}));
}

#[test]
fn the_deepest_project_owns_a_directory() {
    let config = json!({"projects": {
        "/work/mono": {"git": {"provider": "azure-devops", "remote": "upstream"}},
        "/work/mono/tools": {"git": {"provider": "github"}},
        "/work/monolith": {"git": {"provider": "github"}},
    }});
    let tools = project_git(&config, std::path::Path::new("/work/mono/tools/cli"));
    assert_eq!(tools.provider, Some(Kind::Github));
    assert_eq!(tools.project.as_deref(), Some("/work/mono/tools"));
    let mono = project_git(&config, std::path::Path::new("/work/mono/web/"));
    assert_eq!(
        (mono.provider, mono.remote.as_deref()),
        (Some(Kind::AzureDevops), Some("upstream"))
    );
    assert_eq!(
        project_git(&config, std::path::Path::new("/work/other")).project,
        None
    );
}

fn git(dir: &std::path::Path, args: &[&str]) {
    let status = std::process::Command::new("git")
        .current_dir(dir)
        .args(args)
        .output()
        .unwrap();
    assert!(status.status.success(), "git {args:?}");
}

#[tokio::test]
async fn detection_reads_remotes_branch_and_project_choice() {
    let dir = tempfile::tempdir().unwrap();
    let repo = dir.path();
    git(repo, &["init", "-q", "-b", "main"]);
    git(repo, &["remote", "add", "origin", "git@github.com:o/r.git"]);
    git(
        repo,
        &[
            "remote",
            "add",
            "upstream",
            "https://dev.azure.com/contoso/P/_git/r",
        ],
    );
    git(repo, &["checkout", "-q", "-b", "feature/links"]);
    let (service, _) = fake(Arc::new(|_| (200, json!({})))).await;
    let detected = service.detect(repo).await.unwrap();
    assert_eq!(detected.branch.as_deref(), Some("feature/links"));
    let found = detected.repo.unwrap();
    assert_eq!((found.kind, found.display()), (Kind::Github, "o/r".into()));
    let wire = super::Detected {
        repo: None,
        ..service.detect(repo).await.unwrap()
    }
    .wire();
    assert_eq!(wire["candidates"], json!(["github", "azure-devops"]));
}
