use serde_json::{Value, json};
use std::sync::Arc;
use workspacer_hub::services::{
    briefs::{Briefs, cards, document, report},
    config::Config,
};
#[test]
fn shared_document_archive_and_check_contract() {
    let corpus: Value =
        serde_json::from_str(include_str!("../../../contracts/brief-board-cases.json")).unwrap();
    for row in corpus["entries"].as_array().unwrap() {
        let doc = document::parse(row["brief"].as_str().unwrap());
        assert_eq!(doc.lines.join("\n"), row["brief"]);
        let actual: Vec<_> = doc
            .entries
            .iter()
            .map(|e| {
                let mut v = json!({"column":e.column,"start":e.start,"end":e.end,"lines":e.lines});
                if let Some(g) = &e.group {
                    v["group"] = json!(g);
                }
                v
            })
            .collect();
        assert_eq!(json!(actual), row["expect"], "{}", row["name"]);
    }
    for row in corpus["archive"].as_array().unwrap() {
        let (next, archive, n) = document::archive(
            row["brief"].as_str().unwrap(),
            row["archive"].as_str().unwrap_or(""),
            row["section"].as_str().unwrap(),
            row["count"].as_u64().map(|n| n as usize),
            row["keep"].as_u64().map(|n| n as usize),
            row["date"].as_str().unwrap(),
        )
        .unwrap();
        let mut actual = document::stats(&next, row["section"].as_str().unwrap());
        actual["brief"] = json!(next);
        actual["archive"] = json!(archive);
        actual["archived"] = json!(n);
        assert_eq!(actual, row["expect"], "{}", row["name"]);
    }
    for row in corpus["check"].as_array().unwrap() {
        let mut actual = report::check(
            row["brief"].as_str().unwrap(),
            row["sessions"].as_array().unwrap(),
            "fixture",
        );
        for key in ["path", "section", "note"] {
            actual.as_object_mut().unwrap().remove(key);
        }
        for f in actual["findings"].as_array_mut().unwrap() {
            f.as_object_mut().unwrap().remove("detail");
        }
        assert_eq!(actual, row["expect"], "{}", row["name"]);
    }
}
#[test]
fn cards_match_shipping_typescript_reference() {
    let rows: Vec<Value> = serde_json::from_str(include_str!("fixtures/brief-cards.json")).unwrap();
    for row in rows {
        let content = row["content"].as_str().unwrap();
        let (cards, extras) = cards::cards(content, &row["index"], false);
        assert_eq!(
            json!({"cards":cards,"extras":extras}),
            row["expected"],
            "{content}"
        );
        assert_eq!(
            json!(
                document::parse(content)
                    .entries
                    .iter()
                    .map(|e| &e.id)
                    .collect::<Vec<_>>()
            ),
            row["ids"]
        );
    }
}
#[test]
fn real_parallel_appends_archive_and_board_move_preserve_text() {
    let dir = tempfile::tempdir().unwrap();
    let home = dir.path().join("home");
    std::fs::create_dir(&home).unwrap();
    let cfg = Arc::new(Config::open(dir.path().join("config.yaml")));
    let service = Arc::new(Briefs::new(cfg, home.clone()));
    let threads:Vec<_>=(0..12).map(|i|{let service=service.clone();let home=home.clone();std::thread::spawn(move||service.call("brief.append",json!({"project":home,"section":"Recently","line":format!("entry {i}  unchanged")}),&[]).unwrap())}).collect();
    for thread in threads {
        thread.join().unwrap();
    }
    let before = std::fs::read_to_string(home.join(".workspacer/brief.md")).unwrap();
    assert_eq!(document::parse(&before).entries.len(), 12);
    let result = service
        .call(
            "brief.archive",
            json!({"project":home,"section":"Recently","keep":10}),
            &[],
        )
        .unwrap();
    assert_eq!(result["archived"], 2);
    assert_eq!(
        service
            .call(
                "brief.archive",
                json!({"project":home,"section":"Recently","keep":10}),
                &[]
            )
            .unwrap()["archived"],
        0
    );
    let board = service
        .call("desktop.loadBriefBoard", json!({}), &[])
        .unwrap();
    assert_eq!(board["lanes"][0]["cards"].as_array().unwrap().len(), 12);
    let id = board["lanes"][0]["cards"][0]["id"].clone();
    let moved = service
        .call(
            "desktop.moveBriefCard",
            json!({"request":{"key":home,"entryId":id,"to":"Now"}}),
            &[],
        )
        .unwrap();
    assert!(
        moved["cards"]
            .as_array()
            .unwrap()
            .iter()
            .any(|c| c["id"] == id && c["column"] == "Now")
    );
    let bytes = std::fs::read(home.join(".workspacer/brief.md")).unwrap();
    assert!(
        service
            .call(
                "brief.append",
                json!({"project":home,"section":"Now","line":"x".repeat(4001)}),
                &[]
            )
            .is_err()
    );
    assert_eq!(
        bytes,
        std::fs::read(home.join(".workspacer/brief.md")).unwrap()
    );
    assert!(
        service
            .call(
                "desktop.moveBriefCard",
                json!({"key":dir.path(),"entryId":id,"to":"Now"}),
                &[]
            )
            .is_err()
    );
}
#[cfg(unix)]
#[test]
fn composed_path_symlink_escape_is_refused() {
    let dir = tempfile::tempdir().unwrap();
    let home = dir.path().join("home");
    let other = dir.path().join("outside");
    std::fs::create_dir(&home).unwrap();
    std::fs::create_dir(&other).unwrap();
    std::os::unix::fs::symlink(&other, home.join(".workspacer")).unwrap();
    let service = Briefs::new(
        Arc::new(Config::open(dir.path().join("config.yaml"))),
        home.clone(),
    );
    for method in ["brief.append", "brief.archive", "brief.check"] {
        assert!(
            service
                .call(
                    method,
                    json!({"project":home,"section":"Now","line":"unsafe","keep":0}),
                    &[]
                )
                .is_err()
        );
    }
    assert_eq!(std::fs::read_dir(&other).unwrap().count(), 0);
}
#[test]
fn additive_crlf_and_result_facts_keep_significance_and_all_caveats() {
    let text = "# Brief\r\n\r\n## Now\r\n- untouched\r\n\r\n## Recently\r\n";
    let next = document::append(text, "Now", "one  line\nwith wrap").unwrap();
    assert_eq!(next.replace("- one  line with wrap\n", ""), text);
    let params = json!({"sessionId":"ABCD1234-1234-5678-1234-123456789012","result":{"commit":"a".repeat(40),"filesChanged":["one","two","three","four"],"caveats":["critical ".repeat(40)]}});
    let line = report::compose("Makes the migration possible", &params, "2026-09-28").unwrap();
    assert!(line.contains("Makes the migration possible"));
    assert!(line.contains("+1 more"));
    assert!(line.contains("critical ".repeat(39).as_str()));
    assert!(line.ends_with("(session:abcd1234)"));
    assert!(report::compose("", &params, "2026-09-28").is_err());
}

#[test]
fn reference_writer_lock_is_respected_and_stale_lease_recovers() {
    let dir = tempfile::tempdir().unwrap();
    let hidden = dir.path().join(".workspacer");
    std::fs::create_dir(&hidden).unwrap();
    let lock = hidden.join("brief.md.lock");
    std::fs::write(&lock, "123 2026-09-28T00:00:00Z\n").unwrap();
    let service = Briefs::new(
        Arc::new(Config::open(dir.path().join("config.yaml"))),
        dir.path().into(),
    );
    let release = lock.clone();
    let thread = std::thread::spawn(move || {
        std::thread::sleep(std::time::Duration::from_millis(100));
        std::fs::write(hidden.join("brief.md"), "## Now\n- reference writer\n").unwrap();
        std::fs::remove_file(release).unwrap();
    });
    service
        .call(
            "brief.append",
            json!({"project":dir.path(),"section":"Now","line":"rust writer"}),
            &[],
        )
        .unwrap();
    thread.join().unwrap();
    assert_eq!(
        std::fs::read_to_string(dir.path().join(".workspacer/brief.md")).unwrap(),
        "## Now\n- reference writer\n- rust writer\n"
    );
    assert!(!lock.exists());
    std::fs::write(&lock, "old reference writer\n").unwrap();
    let old = std::time::SystemTime::now() - std::time::Duration::from_secs(16);
    std::fs::OpenOptions::new()
        .write(true)
        .open(&lock)
        .unwrap()
        .set_modified(old)
        .unwrap();
    service
        .call(
            "brief.append",
            json!({"project":dir.path(),"section":"Now","line":"after restart"}),
            &[],
        )
        .unwrap();
    assert!(!lock.exists());
}

#[test]
fn complete_reports_match_typescript_reference_including_advisory_wording() {
    let rows: Vec<Value> =
        serde_json::from_str(include_str!("fixtures/brief-reports.json")).unwrap();
    for row in rows {
        assert_eq!(
            report::check(
                row["brief"].as_str().unwrap(),
                row["sessions"].as_array().unwrap(),
                "fixture"
            ),
            row["expected"],
            "{}",
            row["name"]
        );
    }
}
