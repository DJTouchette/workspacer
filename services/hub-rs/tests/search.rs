use serde_json::json;
use workspacer_hub::services::search::{Collector, search};

#[test]
fn collector_preserves_every_submatch_and_reports_truncation_only_on_overflow() {
    let mut collector = Collector::new(std::env::temp_dir(), 2);
    let row = json!({"type":"match","data":{"path":{"text":"./file"},"lines":{"text":"  one one one\n"},"line_number":3,"submatches":[{"start":2},{"start":6},{"start":10}]}});
    assert!(collector.add(&row.to_string()));
    let result = collector.result();
    assert_eq!(result["truncated"], true);
    assert_eq!(
        result["results"][0]["matches"],
        json!([{"line":3,"column":3,"text":"one one one"},{"line":3,"column":7,"text":"one one one"}])
    );
}
#[tokio::test]
async fn live_search_obeys_ignore_rules_and_never_treats_query_as_a_flag() {
    let dir = tempfile::tempdir().unwrap();
    assert!(
        std::process::Command::new("git")
            .args(["init", "--quiet"])
            .arg(dir.path())
            .status()
            .unwrap()
            .success()
    );
    std::fs::write(dir.path().join(".gitignore"), "ignored.txt\n").unwrap();
    std::fs::write(dir.path().join("visible.txt"), "--flag --flag\n").unwrap();
    std::fs::write(dir.path().join("ignored.txt"), "--flag\n").unwrap();
    let result = search(json!({"cwd":dir.path(),"query":"--flag","maxResults":1}))
        .await
        .unwrap();
    assert_eq!(result["results"].as_array().unwrap().len(), 1);
    assert_eq!(result["truncated"], true);
    let absent = search(json!({"cwd":dir.path(),"query":"not found"}))
        .await
        .unwrap();
    assert_eq!(absent, json!({"results":[],"truncated":false}));
}
