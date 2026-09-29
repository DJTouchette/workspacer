use serde_json::{Value, json};
use std::sync::Arc;
use workspacer_hub::services::{
    briefs::{Briefs, cards, document, report},
    config::Config,
};
#[test]
fn shared_document_archive_and_check_contract() {
    #[path = "support/sweepguard.rs"]
    mod sweepguard;
    let corpus: Value =
        serde_json::from_str(include_str!("../../../contracts/brief-board-cases.json")).unwrap();
    let mut entries = sweepguard::Tally::default();
    let mut archive_tally = sweepguard::Tally::default();
    let mut check = sweepguard::Tally::default();
    for row in corpus["entries"].as_array().unwrap() {
        let doc = document::parse(row["brief"].as_str().unwrap());
        entries.ran("other");
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
        archive_tally.ran("other");
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
        check.ran("other");
        for key in ["path", "section", "note"] {
            actual.as_object_mut().unwrap().remove(key);
        }
        for f in actual["findings"].as_array_mut().unwrap() {
            f.as_object_mut().unwrap().remove("detail");
        }
        assert_eq!(actual, row["expect"], "{}", row["name"]);
    }
    entries.require_every("brief entries", 4).unwrap();
    archive_tally.require_every("brief archive", 5).unwrap();
    check.require_every("brief check", 6).unwrap();
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
#[cfg(any(unix, windows))]
#[test]
fn composed_path_symlink_escape_is_refused() {
    let dir = tempfile::tempdir().unwrap();
    let home = dir.path().join("home");
    let other = dir.path().join("outside");
    std::fs::create_dir(&home).unwrap();
    std::fs::create_dir(&other).unwrap();
    #[cfg(unix)]
    std::os::unix::fs::symlink(&other, home.join(".workspacer")).unwrap();
    #[cfg(windows)]
    std::os::windows::fs::symlink_dir(&other, home.join(".workspacer"))
        .expect("Windows contract CI must provide symlink privilege");
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

#[test]
fn legacy_result_vectors_preserve_significance_facts_and_session_links() {
    for invalid in [
        "6a-round2",
        "session:6a-round2",
        "",
        "   ",
        "session:",
        "abc",
        "12345",
        "the-parser-worker",
        "ffff-2",
        "zzzzzzzz",
    ] {
        assert!(report::session_ref(invalid).is_err(), "{invalid}");
    }
    assert!(
        report::session_ref("round2")
            .unwrap_err()
            .to_string()
            .contains("\"round2\"")
    );
    for (raw, expected) in [
        ("c03bd8ce-1f4a-4b2c-9d3e-0123456789ab", "c03bd8ce"),
        ("  session:C03BD8CE  ", "c03bd8ce"),
        ("a1b2c3d4e5f6a7b8", "a1b2c3d4"),
        ("a1b2c3", "a1b2c3"),
    ] {
        assert_eq!(report::session_ref(raw).unwrap(), expected);
    }
    let params = json!({"sessionId":"c03bd8ce-1f4a-4b2c-9d3e-0123456789ab","result":{"commit":"a1b2c3d4e5f6a7b8c9d0e1f2a3b4c5d6e7f8a9b0","filesChanged":["src/parser.ts","src/lexer.ts"],"checksRun":["vitest","tsc"],"caveats":[],"followUps":["delete the v1 path"]}});
    let sentence = "the parser no longer allocates per token, which unblocks the mobile client";
    assert_eq!(
        report::compose(sentence, &params, "2026-08-26").unwrap(),
        format!(
            "2026-08-26  {sentence} — commit: a1b2c3d4e5f6; filesChanged: src/parser.ts, src/lexer.ts; checksRun: vitest, tsc; followUps: delete the v1 path (session:c03bd8ce)"
        )
    );
    for blank in ["", "   ", "\n\t"] {
        assert!(
            report::compose(blank, &params, "2026-08-26")
                .unwrap_err()
                .to_string()
                .contains("one-sentence significance")
        );
    }
    let caveats: Vec<_> = (0..5)
        .map(|i| format!("caveat {i}: {}", "x".repeat(600)))
        .collect();
    let output=report::compose("judgement",&json!({"sessionId":"","result":{"commit":"tag ".repeat(80),"filesChanged":["a","b","c","d","e","f"],"caveats":caveats,"notes":"z".repeat(600),"benchmarkMs":12.0,"empty":null,"unused":[]}}),"2026-08-26").unwrap();
    for caveat in &caveats {
        assert!(output.contains(caveat));
    }
    assert!(output.contains("commit: ") && output.contains("tag ".repeat(79).as_str()));
    assert!(output.contains("filesChanged: a, b, c, +3 more"));
    assert!(output.contains("… (600 chars)") && output.contains("benchmarkMs: 12;"));
    assert!(
        !output.contains("empty:") && !output.contains("unused:") && !output.contains("(session:")
    );
    let duplicate = report::compose(
        "2026-08-24  backfilled, see session:c03bd8ce",
        &params,
        "2026-08-26",
    )
    .unwrap();
    assert!(duplicate.starts_with("2026-08-24  backfilled"));
    assert_eq!(duplicate.matches("session:c03bd8ce").count(), 1);
    let fact_ref = report::compose(
        "judgement",
        &json!({"sessionId":"c03bd8ce","result":{"note":"mentions session:c03bd8ce"}}),
        "2026-08-26",
    )
    .unwrap();
    assert!(fact_ref.ends_with("(session:c03bd8ce)"));
    assert_eq!(report::compose("judgement",&json!({"result":{"zzz":"last","followUps":["f"],"commit":"abc1234","filesChanged":["a"]}}),"2026-08-26").unwrap(),"2026-08-26  judgement — commit: abc1234; filesChanged: a; followUps: f; zzz: last");
    assert!(report::compose("judgement", &json!({"result":[1,2]}), "2026-08-26").is_err());
}

#[test]
fn archive_bounds_and_noop_preserve_files_and_null_append_metadata_is_absent() {
    let root = tempfile::tempdir().unwrap();
    let service = Briefs::new(
        Arc::new(Config::open(root.path().join("config.yaml"))),
        root.path().into(),
    );
    for patch in [
        json!({}),
        json!({"count":1,"keep":1}),
        json!({"count":1.5}),
        json!({"count":-1}),
        json!({"count":"1"}),
    ] {
        let mut params = json!({"project":root.path(),"section":"Now"});
        params
            .as_object_mut()
            .unwrap()
            .extend(patch.as_object().unwrap().clone());
        let error = service
            .call("brief.archive", params, &[])
            .unwrap_err()
            .to_string();
        let expected = if patch.get("count").is_none() || patch.get("keep").is_some() {
            "give either count"
        } else {
            "whole number"
        };
        assert!(error.contains(expected), "{error}");
        assert!(!root.path().join(".workspacer").exists());
    }
    service.call("brief.append",json!({"project":root.path(),"section":"Now","line":"plain","sessionId":null,"result":null}),&[]).unwrap();
    let path = root.path().join(".workspacer/brief.md");
    assert!(
        std::fs::read_to_string(&path)
            .unwrap()
            .starts_with("## Now\n- plain\n")
    );
    let content = "## Now\n- first\n  with a continuation\n- second\n\n## Direction\n- keep me\n";
    std::fs::write(&path, content).unwrap();
    let result = service
        .call(
            "brief.archive",
            json!({"project":root.path(),"section":"Now","count":2.0,"keep":null}),
            &[],
        )
        .unwrap();
    assert_eq!(result["archived"], 2);
    let archive = root.path().join(".workspacer/brief.archive.md");
    let archived = std::fs::read(&archive).unwrap();
    let remaining = std::fs::read(&path).unwrap();
    let archive_text = String::from_utf8(archived.clone()).unwrap();
    for line in ["- first", "  with a continuation", "- second"] {
        assert!(archive_text.contains(line));
    }
    assert!(
        String::from_utf8(remaining.clone())
            .unwrap()
            .contains("- keep me")
    );
    assert_eq!(
        service
            .call(
                "brief.archive",
                json!({"project":root.path(),"section":"Now","keep":0,"count":null}),
                &[]
            )
            .unwrap()["archived"],
        0
    );
    assert_eq!(std::fs::read(&archive).unwrap(), archived);
    assert_eq!(std::fs::read(&path).unwrap(), remaining);
    assert!(
        service
            .call(
                "brief.archive",
                json!({"project":root.path(),"section":"Recently","count":1}),
                &[]
            )
            .unwrap_err()
            .to_string()
            .contains("no \"## Recently\" section")
    );
    assert!(
        service
            .call(
                "brief.append",
                json!({"project":root.path(),"section":"Landed","line":"no"}),
                &[]
            )
            .unwrap_err()
            .to_string()
            .contains("unknown section")
    );
    let long = json!({"project":root.path(),"section":"Now","line":"judgement","result":{"caveats":["x".repeat(4100)]}});
    assert!(service.call("brief.append", long, &[]).is_err());
    assert_eq!(std::fs::read(&path).unwrap(), remaining);
}

#[tokio::test]
async fn live_brief_check_keeps_snake_case_ids_and_reports_unavailable_sources_without_writes() {
    use workspacer_hub::{Hub, Options, client::Client};
    let brief = "## Now\n- Dispatched A (session:aaaaaaaa)\n- Dispatched B (session:bbbbbbbb)\n- Dispatched bad (session:6a-round2)\n- Dispatched without an id\n";
    for response in [
        None,
        Some(json!({"bad":"shape"})),
        Some(
            json!([{"session_id":"AAAAAAAA","mode":"input"},{"sessionId":"bbbbbbbb","mode":"stopped"}]),
        ),
    ] {
        let root = tempfile::tempdir().unwrap();
        let hidden = root.path().join(".workspacer");
        std::fs::create_dir(&hidden).unwrap();
        let path = hidden.join("brief.md");
        std::fs::write(&path, brief).unwrap();
        let mut options = Options::default();
        options.home_dir = Some(root.path().into());
        options.config_dir = Some(root.path().join("config"));
        let available = response.as_ref().is_some_and(Value::is_array);
        if let Some(response) = response {
            options = options.handler("sessions.snapshots", move |_, _| {
                let response = response.clone();
                async move { Ok(response) }
            });
        }
        let hub = Hub::start(options).unwrap();
        hub.ready().await.unwrap();
        let client = Client::connect(&hub.handle()).await.unwrap();
        let report = client
            .call("brief.check", json!({"project":root.path()}))
            .await
            .unwrap();
        assert_eq!(report["entriesChecked"], 4);
        let reasons: Vec<_> = report["findings"]
            .as_array()
            .unwrap()
            .iter()
            .map(|f| f["reason"].as_str().unwrap())
            .collect();
        assert!(reasons.contains(&"malformed") && reasons.contains(&"unreferenced"));
        if available {
            assert_eq!(report["entriesLive"], 1);
            assert_eq!(report["liveSessions"], 1);
            assert!(reasons.contains(&"stale"));
            assert!(report.get("unavailableChecks").is_none());
        } else {
            assert!(!reasons.contains(&"stale"));
            assert_eq!(report["unavailableChecks"], json!(["stale"]));
            assert!(
                report["note"]
                    .as_str()
                    .unwrap()
                    .contains("ONE CHECK DID NOT RUN")
            );
        }
        assert_eq!(std::fs::read_to_string(&path).unwrap(), brief);
        assert_eq!(std::fs::read_dir(&hidden).unwrap().count(), 1);
        std::fs::remove_file(&path).unwrap();
        let empty = client
            .call("brief.check", json!({"project":root.path()}))
            .await
            .unwrap();
        assert_eq!(empty["entriesChecked"], 0);
        assert_eq!(empty["findings"], json!([]));
        assert_eq!(std::fs::read_dir(&hidden).unwrap().count(), 0);
        client.close();
        hub.shutdown().unwrap();
    }
}

#[test]
fn image_confinement_reference_refuses_secret_extensions_and_keeps_png_control() {
    use base64::Engine;
    let root = tempfile::tempdir().unwrap();
    let secret = "BRIEF_REFERENCE_SECRET_MARKER";
    let encoded = base64::engine::general_purpose::STANDARD.encode(secret);
    for name in [".env", "id_rsa"] {
        let path = root.path().join(name);
        std::fs::write(&path, secret).unwrap();
        let error = workspacer_hub::services::files::call(
            "fs.readImage",
            json!({"path":path}),
            root.path(),
        )
        .unwrap_err()
        .to_string();
        assert!(error.contains("not a previewable browser image"));
        assert!(!error.contains(secret) && !error.contains(&encoded));
    }
    let path = root.path().join("shot.png");
    let png: &[u8] = &[
        0x89, b'P', b'N', b'G', 0x0d, 0x0a, 0x1a, 0x0a, 0, 0, 0, 0x0d, b'I', b'H', b'D', b'R', 0,
        0, 0, 1, 0, 0, 0, 1, 8, 6, 0, 0, 0, 0x1f, 0x15, 0xc4, 0x89, 0, 0, 0, 0, b'I', b'E', b'N',
        b'D', 0xae, 0x42, 0x60, 0x82,
    ];
    std::fs::write(&path, png).unwrap();
    let result =
        workspacer_hub::services::files::call("fs.readImage", json!({"path":path}), root.path())
            .unwrap();
    assert_eq!(
        result["dataUrl"],
        format!(
            "data:image/png;base64,{}",
            base64::engine::general_purpose::STANDARD.encode(png)
        )
    );
    assert_eq!(result["width"], 1);
    assert_eq!(result["height"], 1);
}

#[cfg(windows)]
#[test]
fn ambient_windows_project_spellings_and_ordinal_containment_match_reference() {
    use std::path::Path;
    use workspacer_hub::services::paths;
    let root = tempfile::tempdir().unwrap();
    let home = root.path().join("FleetManager");
    let service = Briefs::new(
        Arc::new(Config::open(root.path().join("config.yaml"))),
        home.clone(),
    );
    for project in [
        home.join("Projects/Client"),
        root.path().join("Outside"),
        root.path().join("FleetManagerOther"),
    ] {
        std::fs::create_dir_all(&project).unwrap();
        let variant = project
            .to_string_lossy()
            .replace('\\', "/")
            .chars()
            .map(|c| {
                if c.is_ascii_lowercase() {
                    c.to_ascii_uppercase()
                } else if c.is_ascii_uppercase() {
                    c.to_ascii_lowercase()
                } else {
                    c
                }
            })
            .collect::<String>();
        service
            .call(
                "brief.append",
                json!({"project":variant,"section":"Recently","line":"ambient update"}),
                &[],
            )
            .unwrap();
        service
            .call("brief.check", json!({"project":variant}), &[])
            .unwrap();
        assert_eq!(
            service
                .call(
                    "brief.archive",
                    json!({"project":variant,"section":"Recently","count":1}),
                    &[]
                )
                .unwrap()["archived"],
            1
        );
        assert!(project.join(".workspacer/brief.archive.md").exists());
    }
    for (root, target, want) in [
        (r"C:\Work\FleetManager", r"c:\work\fleetmanager", true),
        (
            r"C:\Work\FleetManager",
            r"c:\WORK\fleetMANAGER\Projects\Client",
            true,
        ),
        (
            r"C:\Work\FleetManager",
            r"C:\Work\FleetManagerOther\loot.txt",
            false,
        ),
        (
            r"C:\Work\FleetManager",
            r"c:\work\fleetmanagerother\loot.txt",
            false,
        ),
        (r"C:\Work\FleetManager", r"C:\Work\Outside\loot.txt", false),
        (r"C:\", r"C:\Work\loot.txt", true),
        (r"C:\Work", r"D:\Work\loot.txt", false),
        (r"C:\Работа\ПРОЕКТ", r"c:\работа\проект", true),
        (r"C:\Work\Ärger", r"c:\work\ärGER", true),
        (r"C:\Work\Kelvin", r"c:\work\kelvin", false),
    ] {
        assert_eq!(
            paths::contained(Path::new(target), Path::new(root)),
            want,
            "{root}/{target}"
        );
    }
}
