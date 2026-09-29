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

#[test]
fn collector_retains_unicode_ascii_trim_defaults_and_malformed_output_refusal() {
    let root = std::env::temp_dir();
    for text in ["😀".repeat(400), format!("a{}", "😀".repeat(400))] {
        let mut collector = Collector::new(root.clone(), 10);
        collector.add(&json!({"type":"match","data":{"path":{"text":"unicode.txt"},"lines":{"text":format!(" \t{text}\r\n")},"line_number":1,"submatches":[]}}).to_string());
        let result = collector.result();
        let matched = &result["results"][0]["matches"][0];
        assert_eq!(matched["text"].as_str().unwrap().chars().count(), 300);
        assert!(matched["text"].as_str().unwrap().ends_with('😀'));
        assert_eq!(matched["column"], 1);
    }
    let row = json!({"type":"match","data":{"path":{"text":"plain"},"lines":{"text":" \u{feff}NEEDLE\u{85}\t\r\n"},"line_number":7,"submatches":[{"start":0}]}});
    let mut collector = Collector::new(root.clone(), 1);
    assert!(!collector.add(&row.to_string()));
    assert_eq!(
        collector.result()["results"][0]["matches"][0]["text"],
        "\u{feff}NEEDLE\u{85}"
    );
    for limit in [0, -1, -500] {
        let mut collector = Collector::new(root.clone(), limit);
        for _ in 0..500 {
            assert!(!collector.add(&row.to_string()));
        }
        assert!(collector.add(&row.to_string()));
        let result = collector.result();
        assert_eq!(
            result["results"][0]["matches"].as_array().unwrap().len(),
            500
        );
        assert_eq!(result["truncated"], true);
    }
    for (field, invalid) in [
        ("submatches", json!([{"start":u64::MAX}])),
        ("submatches", json!([{"start":i64::MAX}])),
        ("submatches", json!([{"start":"0"}])),
        ("submatches", json!([true])),
        ("line_number", json!("7")),
        ("lines", json!({"text":true})),
        ("lines", json!([])),
    ] {
        let mut bad = row.clone();
        bad["data"][field] = invalid;
        let mut collector = Collector::new(root.clone(), 10);
        assert!(!collector.add(&bad.to_string()));
        assert_eq!(collector.result(), json!({"results":[],"truncated":false}));
    }
}

#[tokio::test]
async fn empty_query_and_typed_flags_preserve_request_contract() {
    let dir = tempfile::tempdir().unwrap();
    for query in [json!(""), serde_json::Value::Null] {
        assert_eq!(
            search(json!({"cwd":dir.path(),"query":query}))
                .await
                .unwrap(),
            json!({"results":[],"truncated":false})
        );
    }
    for (key, bad) in [
        ("query", json!(false)),
        ("cwd", json!(42)),
        ("caseSensitive", json!("true")),
        ("wholeWord", json!(1)),
        ("regex", json!([])),
        ("maxResults", json!(1.5)),
    ] {
        let mut request = json!({"cwd":dir.path(),"query":""});
        request[key] = bad;
        assert!(search(request).await.is_err(), "{key}");
    }
    assert!(search(json!({"cwd":"relative","query":""})).await.is_err());
    assert!(search(json!([])).await.is_err());
    std::fs::write(
        dir.path().join("text.txt"),
        "Needle needle needles\na.b axb\n",
    )
    .unwrap();
    for (request, count) in [
        (json!({"query":"needle"}), 3),
        (json!({"query":"needle","wholeWord":true}), 2),
        (
            json!({"query":"needle","caseSensitive":true,"wholeWord":true}),
            1,
        ),
        (json!({"query":"a.b","regex":true}), 2),
        (json!({"query":"a.b"}), 1),
    ] {
        let mut request = request;
        request["cwd"] = json!(dir.path());
        let found = search(request).await.unwrap();
        assert_eq!(
            found["results"][0]["matches"].as_array().unwrap().len(),
            count
        );
    }
    assert!(
        search(json!({"cwd":dir.path(),"query":"[","regex":true}))
            .await
            .unwrap_err()
            .to_string()
            .contains("ripgrep failed")
    );
    std::fs::write(
        dir.path().join("minified.txt"),
        format!("{}NEEDLE", "x".repeat(100_000)),
    )
    .unwrap();
    let found = search(json!({"cwd":dir.path(),"query":"NEEDLE","caseSensitive":true}))
        .await
        .unwrap();
    assert_eq!(found["results"][0]["matches"][0]["column"], 100_001);
    assert_eq!(
        found["results"][0]["matches"][0]["text"]
            .as_str()
            .unwrap()
            .len(),
        300
    );
}
