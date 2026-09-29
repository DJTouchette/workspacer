use serde_json::{Value, json};
use workspacer_hub::services::{files, library::Library, stores::Stores};
#[path = "support/sweepguard.rs"]
mod sweepguard;

fn fixture() -> Value {
    serde_json::from_str(include_str!(
        "../../../contracts/provider-parity-cases.json"
    ))
    .unwrap()
}

#[test]
fn shared_provider_order_scalar_and_suffix_contract() {
    let corpus = fixture();
    let mut order = sweepguard::Tally::default();
    let mut scalar = sweepguard::Tally::default();
    let mut suffix = sweepguard::Tally::default();
    for row in corpus["order"].as_array().unwrap() {
        let mut input: Vec<&str> = row["input"]
            .as_array()
            .unwrap()
            .iter()
            .map(|v| v.as_str().unwrap())
            .collect();
        input.sort();
        assert_eq!(json!(input), row["expected"], "{}", row["name"]);
        order.ran("other");
    }
    for row in corpus["scalar"].as_array().unwrap() {
        assert_eq!(
            row["value"].as_str().unwrap_or(""),
            row["expected"].as_str().unwrap()
        );
        scalar.ran("other");
    }
    for row in corpus["suffix"].as_array().unwrap() {
        let input = row["value"].as_str().unwrap();
        let ending = row["suffix"].as_str().unwrap();
        let actual = if row["fold"] == true
            && input
                .to_ascii_lowercase()
                .ends_with(&ending.to_ascii_lowercase())
        {
            &input[..input.len() - ending.len()]
        } else {
            input.strip_suffix(ending).unwrap_or(input)
        };
        assert_eq!(actual, row["expected"].as_str().unwrap(), "{}", row["name"]);
        suffix.ran("other");
    }
    order.require_every("provider order", 4).unwrap();
    scalar.require_every("provider scalar", 7).unwrap();
    suffix.require_every("provider suffix", 10).unwrap();
}

fn empty_library(root: &std::path::Path) -> Library {
    let directory = root.join("config");
    let library = Library::new(directory.clone());
    library.list(&json!({})).unwrap();
    std::fs::remove_dir_all(directory.join("library")).unwrap();
    std::fs::create_dir_all(directory.join("library")).unwrap();
    library
}

#[test]
fn actual_library_and_directory_listers_use_shared_byte_order() {
    let corpus = fixture();
    for index in [0, 3] {
        let row = &corpus["order"][index];
        let root = tempfile::tempdir().unwrap();
        let library = empty_library(root.path());
        let folder = root.path().join("directories");
        std::fs::create_dir(&folder).unwrap();
        for (index, title) in row["input"].as_array().unwrap().iter().enumerate() {
            let title = title.as_str().unwrap();
            std::fs::create_dir(folder.join(title)).unwrap();
            std::fs::write(
                root.path().join(format!("config/library/item-{index}.md")),
                format!(
                    "---\ntitle: {}\n---\nx\n",
                    serde_json::to_string(title).unwrap()
                ),
            )
            .unwrap();
        }
        let items = library.list(&json!({})).unwrap();
        assert_eq!(
            json!(
                items
                    .as_array()
                    .unwrap()
                    .iter()
                    .map(|r| r["title"].clone())
                    .collect::<Vec<_>>()
            ),
            row["expected"]
        );
        assert_eq!(
            files::call("fs.listDir", json!({"path":folder}), root.path()).unwrap()["dirs"],
            row["expected"]
        );
    }
}

#[test]
fn actual_library_suffix_and_blank_title_fields_match_desktop() {
    let root = tempfile::tempdir().unwrap();
    let library = empty_library(root.path());
    for (name, title) in [
        ("notes.md", "Notes"),
        ("readme.Md", "Readme mixed"),
        ("guide.mD", "Guide"),
        ("spec.MD", "Spec"),
        ("wsp.md", "   "),
        ("ascii.md", "\t\n\u{b}\u{c}\r "),
        ("nel.md", "\u{85}"),
        ("bom.md", "\u{feff}"),
        ("padded.md", "  Notes  "),
    ] {
        std::fs::write(
            root.path().join("config/library").join(name),
            format!(
                "---\ntitle: {}\n---\nx\n",
                serde_json::to_string(title)
                    .unwrap()
                    .replace('\u{85}', "\\u0085")
                    .replace('\u{feff}', "\\ufeff")
            ),
        )
        .unwrap();
    }
    let rows = library.list(&json!({})).unwrap();
    for (id, title) in [
        ("notes", "Notes"),
        ("readme", "Readme mixed"),
        ("guide", "Guide"),
        ("spec", "Spec"),
        ("wsp", "wsp"),
        ("ascii", "ascii"),
        ("nel", "\u{85}"),
        ("bom", "\u{feff}"),
        ("padded", "  Notes  "),
    ] {
        assert_eq!(
            rows.as_array()
                .unwrap()
                .iter()
                .find(|row| row["id"] == id)
                .unwrap()["title"],
            title
        );
    }
}

#[test]
fn actual_store_listers_keep_non_timestamp_scalars_last_and_preserve_dates() {
    let root = tempfile::tempdir().unwrap();
    let stores = Stores::new(root.path().into());
    for (kind, field) in [("layouts", "createdAt"), ("sessions", "timestamp")] {
        let folder = root.path().join(kind);
        std::fs::create_dir(&folder).unwrap();
        for (i, time) in [
            "2026-03-01T00:00:00.000Z",
            "2026-02-01T00:00:00.000Z",
            "2026-01-01T00:00:00.000Z",
        ]
        .iter()
        .enumerate()
        {
            std::fs::write(
                folder.join(format!("real{}.yaml", i + 1)),
                format!(
                    "id: real{0}\nname: real{0}\n{field}: '{time}'\nagents: []\n",
                    i + 1
                ),
            )
            .unwrap();
        }
        let odd = if kind == "layouts" {
            "5"
        } else {
            "2026-03-01T00:00:00.000Z"
        };
        std::fs::write(
            folder.join("aaa.yaml"),
            format!("id: aaa\nname: aaa\n{field}: {odd}\nagents: []\n"),
        )
        .unwrap();
        let rows = stores.call(&format!("{kind}.list"), json!({})).unwrap();
        let key = if kind == "layouts" { "id" } else { "name" };
        let names = rows
            .as_array()
            .unwrap()
            .iter()
            .map(|r| r[key].as_str().unwrap())
            .collect::<Vec<_>>();
        assert_eq!(
            names,
            if kind == "layouts" {
                vec!["real1", "real2", "real3", "aaa"]
            } else {
                vec!["aaa", "real1", "real2", "real3"]
            },
            "{kind}"
        );
    }
}

#[test]
fn actual_saved_timestamps_are_quoted_and_roundtrip() {
    let root = tempfile::tempdir().unwrap();
    let stores = Stores::new(root.path().into());
    for (store, field) in [("layouts", "createdAt"), ("sessions", "timestamp")] {
        let saved = stores
            .call(
                &format!("{store}.save"),
                json!({"name":"😀 Café", "agents":[]}),
            )
            .unwrap();
        let (filename, document) = if store == "layouts" {
            (format!("{}.yaml", saved["id"].as_str().unwrap()), saved)
        } else {
            let filename = saved.as_str().unwrap().to_owned();
            let loaded = stores
                .call("sessions.load", json!({"filename":filename}))
                .unwrap();
            (filename, loaded)
        };
        let bytes = std::fs::read(root.path().join(store).join(filename)).unwrap();
        let yaml = String::from_utf8(bytes).unwrap();
        assert!(yaml.contains(&format!("{field}: \"")), "{yaml}");
        assert_eq!(serde_yaml::from_str::<Value>(&yaml).unwrap(), document);
        let listed = stores.call(&format!("{store}.list"), json!({})).unwrap();
        assert_eq!(listed[0][field], document[field]);
        println!(
            "WKS_STORE_YAML_PARITY={}",
            json!({"store":store,"yaml":yaml,"document":document})
        );
    }
    emit_timestamp_matrix();
}

fn emit_timestamp_matrix() {
    let cases = [
        ("canonical", "", "2026-03-01T00:00:00.000Z"),
        ("date-only", "", "2026-03-01"),
        ("short-valid-fields", "", "2026-3-1t4:05:06Z"),
        ("rejected-short-time", "", "2026-3-1t4:5:6Z"),
        ("rejected-short-date", "", "2026-3-1"),
        ("space-time", "", "2026-3-1 4:05:06"),
        ("short-offset", "", "2026-03-01T00:00:00-5"),
        ("colon-offset", "", "2026-03-01T00:00:00+05:30"),
        ("fraction", "", "2026-03-01T00:00:00.123456789Z"),
        ("empty-fraction", "", "2026-03-01T00:00:00.Z"),
        ("year-zero", "", "0000-01-01"),
        ("year-99", "", "0099-12-31"),
        ("year-100", "", "0100-01-01"),
        ("extended-year-rollover", "", "9999-99-99T99:99:99+99:99"),
        ("calendar-rollover", "", "2026-00-00T25:61:61Z"),
        ("ordinary-text", "", "not-a-date"),
        ("quoted-offset", "", "'2026-03-01T00:00:00+05:30'"),
        ("explicit-string", "", "!!str 2026-03-01"),
        ("explicit-time", "", "!!timestamp '2026-03-01'"),
        (
            "directive-string",
            "%TAG !s! tag:yaml.org,2002:\n---\n",
            "!s!str 2026-03-01",
        ),
        (
            "directive-time",
            "%TAG !t! tag:yaml.org,2002:\n---\n",
            "!t!timestamp 2026-03-01",
        ),
        ("plain-anchor", "value: &when 2026-03-01\n", "*when"),
        ("quoted-anchor", "value: &when '2026-03-01'\n", "*when"),
        ("number", "", "5"),
        ("null", "", "null"),
        ("mapping", "", "{ unexpected: true }"),
        ("array", "", "[unexpected]"),
        ("invalid-explicit-time", "", "!!timestamp not-a-date"),
        ("invalid-explicit-short-date", "", "!!timestamp '2026-3-1'"),
        ("flow-map", "", "2026-03-01T00:00:00Z"),
        ("offset-whitespace", "", "2026-03-01 04:05:06 \t+5:30"),
        ("rejected-offset", "", "2026-03-01T00:00:00+0530"),
        ("uri-time", "", "!<tag:yaml.org,2002:timestamp> 2026-03-01"),
        ("uri-string", "", "!<tag:yaml.org,2002:str> 2026-03-01"),
    ];
    let mut receipts = Vec::new();
    for (store, field) in [("layouts", "createdAt"), ("sessions", "timestamp")] {
        for (name, prefix, scalar) in cases {
            let root = tempfile::tempdir().unwrap();
            let folder = root.path().join(store);
            std::fs::create_dir(&folder).unwrap();
            let yaml = if name == "flow-map" {
                format!("{{id: item, name: item, agents: [], {field}: {scalar}}}\n")
            } else {
                format!("{prefix}id: item\nname: item\nagents: []\n{field}: {scalar}\n")
            };
            let file = folder.join("item.yaml");
            std::fs::write(&file, &yaml).unwrap();
            let rows = Stores::new(root.path().into())
                .call(&format!("{store}.list"), json!({}))
                .unwrap();
            let row = rows.as_array().unwrap().first();
            receipts.push(json!({"case":name,"store":store,"yaml":yaml,"present":row.is_some(),"projection":row.map(|r|r[field].clone())}));
            assert_eq!(
                std::fs::read_to_string(&file).unwrap(),
                yaml,
                "LIST must not rewrite {name}"
            );
        }
    }
    println!("WKS_STORE_TIMESTAMP_MATRIX={}", json!(receipts));
}

#[test]
fn old_rust_saved_timestamps_keep_dates_order_and_load_documents() {
    let root = tempfile::tempdir().unwrap();
    let stores = Stores::new(root.path().into());
    for (store, field) in [("sessions", "timestamp"), ("layouts", "createdAt")] {
        let folder = root.path().join(store);
        std::fs::create_dir(&folder).unwrap();
        let mut original_files = Vec::new();
        for (id, stamp) in [
            ("older", "2025-01-01T00:00:00.000Z"),
            ("newer", "2026-03-01T00:00:00.000Z"),
        ] {
            let document = json!({"id":id,"name":id,field:stamp,"agents":[],"schemaVersion":1});
            // Exact previous Rust writer strategy, before root-field quoting.
            let raw = serde_yaml::to_string(&document).unwrap();
            assert!(
                raw.contains(&format!("{field}: {stamp}")),
                "old-writer control must be unquoted: {raw}"
            );
            let file = folder.join(format!("{id}.yaml"));
            std::fs::write(&file, &raw).unwrap();
            if store == "sessions" {
                assert_eq!(
                    stores
                        .call("sessions.load", json!({"filename":format!("{id}.yaml")}))
                        .unwrap(),
                    document
                );
            }
            assert_eq!(std::fs::read_to_string(&file).unwrap(), raw);
            original_files.push((file, raw));
        }
        let rows = stores.call(&format!("{store}.list"), json!({})).unwrap();
        assert_eq!(rows[0]["name"], "newer");
        assert_eq!(rows[0][field], "2026-03-01T00:00:00.000Z");
        assert_eq!(rows[1][field], "2025-01-01T00:00:00.000Z");
        for (file, raw) in original_files {
            assert_eq!(
                std::fs::read_to_string(file).unwrap(),
                raw,
                "LIST must not rewrite an old Rust save"
            );
        }
    }
}
