#![cfg(feature = "rust-hub")]
//! Project reads and writes against a real, isolated Rust hub: its config
//! service (lock, wholesale `projects`, readback), filesystem and git
//! handlers. No provider is started.
use serde_json::Value;
use std::{path::Path, sync::Arc, time::Duration};
use wks_native::{
    controller::{Command, Controller, View},
    features::{Request, RequestState},
    host::{Mode, NativeHost, RustOptions},
    projects::{self, Inspection, Patch},
};

async fn settle(controller: &Controller, key: &'static str, after: u64) -> RequestState {
    let mut views = controller.views.clone();
    tokio::time::timeout(Duration::from_secs(20), async {
        loop {
            let view: Arc<View> = views.borrow_and_update().clone();
            if let Some(state) = view.requests.get(key)
                && !state.loading
                && state.number > after
            {
                return state.clone();
            }
            views.changed().await.unwrap();
        }
    })
    .await
    .unwrap_or_else(|_| panic!("{key} never completed"))
}

async fn run(controller: &Controller, request: Request) -> RequestState {
    let key = request.key();
    let before = controller
        .views
        .borrow()
        .requests
        .get(key)
        .map_or(0, |s| s.number);
    controller.command(Command::Request(request)).unwrap();
    settle(controller, key, before).await
}

fn git(dir: &Path, args: &[&str]) {
    let status = std::process::Command::new("git")
        .args(args)
        .current_dir(dir)
        .env("GIT_CONFIG_GLOBAL", "/dev/null")
        .env("GIT_CONFIG_NOSYSTEM", "1")
        .status()
        .unwrap();
    assert!(status.success(), "git {args:?}");
}

#[tokio::test]
async fn projects_round_trip_through_the_hubs_shared_config() {
    let root = std::env::temp_dir().join(format!(
        "wks-native-projects-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    let mut options = RustOptions::isolated(root.join("state")).unwrap();
    options.home_dir = root.join("home");
    std::fs::create_dir(&options.home_dir).unwrap();
    options.usage_poll_on_boot = Some(false);
    let config_file = options.config_dir.join("config.yaml");
    let repo = options.home_dir.join("repo");
    let plain = options.home_dir.join("plain");
    std::fs::create_dir_all(&repo).unwrap();
    std::fs::create_dir_all(&plain).unwrap();
    git(&repo, &["init", "-q", "-b", "trunk"]);
    // An unborn branch has no name `git rev-parse` will give; commit first.
    std::fs::write(repo.join("README"), "fixture").unwrap();
    git(&repo, &["add", "README"]);
    git(
        &repo,
        &[
            "-c",
            "user.name=fixture",
            "-c",
            "user.email=fixture@example.invalid",
            "commit",
            "-q",
            "-m",
            "init",
        ],
    );
    std::fs::write(repo.join("new.txt"), "untracked").unwrap();

    let host = NativeHost::start(Mode::Rust(options)).unwrap();
    tokio::time::timeout(Duration::from_secs(30), host.ready())
        .await
        .unwrap()
        .unwrap();
    let controller = host.controller();
    let mut views = controller.views.clone();
    tokio::time::timeout(Duration::from_secs(10), async {
        while !views.borrow_and_update().connected {
            views.changed().await.unwrap();
        }
    })
    .await
    .unwrap();

    // A project the desktop configured with identity the native client must
    // carry through its wholesale rewrite.
    let other = run(&controller, Request::Projects).await;
    assert!(other.error.is_none(), "{:?}", other.error);
    let repo_path = repo.to_string_lossy().into_owned();
    let plain_path = plain.to_string_lossy().into_owned();
    let saved = run(
        &controller,
        Request::SaveProject {
            path: repo_path.clone(),
            change: Patch::Pin(true),
        },
    )
    .await;
    assert!(saved.error.is_none(), "{:?}", saved.error);
    let rows = projects::list(Some(&saved.value), &[], &[]);
    assert_eq!(rows[0].path, projects::project_key(&repo_path));
    assert!(rows[0].favourite);

    // Another writer (the desktop) adds a configured project between our
    // writes. JSON is YAML, so the hub reads it like its own file.
    let mut disk = saved.value["projects"].clone();
    disk["/elsewhere/desktop-project"] =
        serde_json::json!({"label": "Desktop", "worktreeSetup": ["make"]});
    std::thread::sleep(Duration::from_millis(20));
    std::fs::write(
        &config_file,
        serde_json::to_string(&serde_json::json!({"projects": disk})).unwrap(),
    )
    .unwrap();

    let touched = run(
        &controller,
        Request::TouchProject {
            path: plain_path.clone(),
            at: 1_700_000_000_000,
        },
    )
    .await;
    assert!(touched.error.is_none(), "{:?}", touched.error);
    let hub: &Value = &touched.value;
    let key = projects::project_key(&repo_path);
    assert_eq!(hub["projects"][key.as_str()]["favourite"], true);
    assert_eq!(
        hub["projects"][projects::project_key(&plain_path).as_str()]["lastOpened"].as_f64(),
        Some(1_700_000_000_000.)
    );
    assert_eq!(
        hub["projects"]["/elsewhere/desktop-project"]["label"], "Desktop",
        "a fresh read precedes every wholesale write"
    );
    let text = std::fs::read_to_string(&config_file).unwrap();
    assert!(
        text.contains("desktop-project") && text.contains(&key) && text.contains("1700000000000")
    );

    // Configured projects are never deleted from this client.
    let refused = run(
        &controller,
        Request::SaveProject {
            path: "/elsewhere/desktop-project".into(),
            change: Patch::Remove,
        },
    )
    .await;
    assert!(refused.error.unwrap().contains("Settings"));
    let removed = run(
        &controller,
        Request::SaveProject {
            path: plain_path.clone(),
            change: Patch::Remove,
        },
    )
    .await;
    assert!(removed.error.is_none(), "{:?}", removed.error);
    assert!(
        projects::list(Some(&removed.value), &[], &[])
            .iter()
            .all(|p| !projects::same_dir(&p.path, &plain_path))
    );

    // An older desktop's legacy-only pin/recent, then a pin and a launch
    // touch accepted together: one serialized round each, so neither
    // wholesale save erases the other.
    let legacy = "/elsewhere/legacy-only";
    let mut disk: Value =
        serde_json::from_str(&serde_json::to_string(&removed.value["projects"]).unwrap()).unwrap();
    disk["/elsewhere/desktop-project"]["label"] = "Desktop".into();
    std::thread::sleep(Duration::from_millis(20));
    std::fs::write(
        &config_file,
        serde_json::to_string(&serde_json::json!({
            "projects": disk,
            "directories": {"favourites": [legacy], "recent": [legacy, "/elsewhere/old"]}
        }))
        .unwrap(),
    )
    .unwrap();
    let view = controller.views.borrow().clone();
    let (save_before, touch_before) = (
        view.requests["project-save"].number,
        view.requests["project-touch"].number,
    );
    controller
        .command(Command::Request(Request::SaveProject {
            path: plain_path.clone(),
            change: Patch::Pin(true),
        }))
        .unwrap();
    controller
        .command(Command::Request(Request::TouchProject {
            path: repo_path.clone(),
            at: 1_800_000_000_000,
        }))
        .unwrap();
    let pinned = settle(&controller, "project-save", save_before).await;
    let touched = settle(&controller, "project-touch", touch_before).await;
    assert!(pinned.error.is_none(), "{:?}", pinned.error);
    assert!(touched.error.is_none(), "{:?}", touched.error);
    let reread = run(&controller, Request::Projects).await;
    let map = &reread.value["projects"];
    assert_eq!(
        map[projects::project_key(&plain_path).as_str()]["favourite"],
        true
    );
    assert_eq!(
        map[key.as_str()]["lastOpened"].as_f64(),
        Some(1_800_000_000_000.)
    );
    assert_eq!(map[key.as_str()]["favourite"], true);
    assert_eq!(map["/elsewhere/desktop-project"]["label"], "Desktop");

    let forgotten = run(
        &controller,
        Request::SaveProject {
            path: legacy.into(),
            change: Patch::Remove,
        },
    )
    .await;
    assert!(forgotten.error.is_none(), "{:?}", forgotten.error);
    assert_eq!(forgotten.value["favourites"], serde_json::json!([]));
    assert_eq!(
        forgotten.value["recent"],
        serde_json::json!(["/elsewhere/old"])
    );
    assert!(
        projects::list(Some(&forgotten.value), &[], &[])
            .iter()
            .all(|p| p.path != legacy)
    );
    let text = std::fs::read_to_string(&config_file).unwrap();
    assert!(!text.contains("legacy-only") && text.contains("desktop-project"));

    // Imported aliases survive config loading, but all must be removed and
    // metadata on other aliases must survive queued recency writes on disk.
    let mut imported = forgotten.value["projects"].clone();
    imported["/imported/"] = serde_json::json!({"lastOpened":1});
    imported["/imported//"] = serde_json::json!({"favourite":true});
    imported["/protected/"] = serde_json::json!({"label":"Keep alias","workflowId":"wf"});
    std::thread::sleep(Duration::from_millis(20));
    std::fs::write(
        &config_file,
        serde_json::to_string(&serde_json::json!({
            "projects":imported,"directories":{"recent":["/imported/"],"favourites":["/imported"]}
        }))
        .unwrap(),
    )
    .unwrap();
    let before = controller.views.borrow().project_touch_receipts.len();
    for (path, at) in [
        ("/protected", 900),
        ("/queued-other", 800),
        ("/protected", 700),
    ] {
        controller
            .command(Command::Request(Request::TouchProject {
                path: path.into(),
                at,
            }))
            .unwrap();
    }
    tokio::time::timeout(Duration::from_secs(20), async {
        let mut views = controller.views.clone();
        while views.borrow_and_update().project_touch_receipts.len() < before + 3 {
            views.changed().await.unwrap();
        }
    })
    .await
    .unwrap();
    let receipts = controller.views.borrow().project_touch_receipts.clone();
    assert!(receipts.iter().skip(before).all(|r| r.error.is_none()));
    let protected = run(
        &controller,
        Request::SaveProject {
            path: "/protected".into(),
            change: Patch::Remove,
        },
    )
    .await;
    assert!(protected.error.as_deref().unwrap().contains("settings"));
    let removed = run(
        &controller,
        Request::SaveProject {
            path: "/imported".into(),
            change: Patch::Remove,
        },
    )
    .await;
    assert!(removed.error.is_none(), "{:?}", removed.error);
    assert_eq!(
        removed.value["projects"]["/protected/"]["label"],
        "Keep alias"
    );
    assert_eq!(removed.value["projects"]["/protected/"]["workflowId"], "wf");
    assert_eq!(removed.value["projects"]["/protected/"]["lastOpened"], 900);
    assert_eq!(
        removed.value["projects"]["/queued-other"]["lastOpened"],
        800
    );
    let text = std::fs::read_to_string(&config_file).unwrap();
    assert!(!text.contains("/imported"));
    assert!(text.contains("Keep alias") && text.contains("900"));

    // What the hub reports about folders before a launch.
    let inspect = |path: String| {
        let controller = controller.clone();
        async move {
            let state = run(&controller, Request::InspectProject { path }).await;
            assert!(state.error.is_none(), "{:?}", state.error);
            projects::parse_inspection(&state.value).unwrap()
        }
    };
    assert_eq!(
        inspect(repo_path.clone()).await,
        Inspection::Repository {
            branch: Some("trunk".into()),
            changes: 1
        }
    );
    assert_eq!(inspect(plain_path.clone()).await, Inspection::NotRepository);
    assert!(matches!(
        inspect(root.join("missing").to_string_lossy().into_owned()).await,
        Inspection::Missing(_)
    ));

    // The hub's folder browser lists children and names its parent.
    let listing = run(
        &controller,
        Request::BrowseFolders {
            path: repo_path.clone(),
        },
    )
    .await;
    let listing = projects::parse_listing(&listing.value).unwrap();
    assert!(listing.dirs.is_empty(), "hidden .git is not offered");
    assert_eq!(
        Path::new(&listing.parent),
        std::fs::canonicalize(repo.parent().unwrap()).unwrap()
    );
    let home = run(
        &controller,
        Request::BrowseFolders {
            path: String::new(),
        },
    )
    .await;
    let home = projects::parse_listing(&home.value).unwrap();
    assert!(home.dirs.contains(&"plain".to_owned()) && home.dirs.contains(&"repo".to_owned()));

    drop(views);
    drop(controller);
    host.shutdown().await.unwrap();
    std::fs::remove_dir_all(root).unwrap();
}

/// A project's name and icon through the real hub: the icon URL is fetched
/// by the hub's own `desktop.downloadProjectIcon` (here from a loopback
/// fixture, never the internet), stored content-addressed, recorded as the
/// desktop's `favicon` + `iconFile` on every alias, and read back through
/// `ui.asset` as a small PNG. Reset removes the fields and keeps the pin.
#[tokio::test]
async fn project_identity_and_icon_round_trip_through_the_hub() {
    use std::io::{Read, Write};
    let root = std::env::temp_dir().join(format!(
        "wks-native-identity-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    let mut options = RustOptions::isolated(root.join("state")).unwrap();
    options.home_dir = root.join("home");
    std::fs::create_dir(&options.home_dir).unwrap();
    options.usage_poll_on_boot = Some(false);
    let config_file = options.config_dir.join("config.yaml");
    let icons_dir = options.config_dir.join("project-icons");

    let mut png = std::io::Cursor::new(Vec::new());
    image::DynamicImage::new_rgb8(32, 32)
        .write_to(&mut png, image::ImageFormat::Png)
        .unwrap();
    let png = png.into_inner();
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let port = listener.local_addr().unwrap().port();
    let served = png.clone();
    std::thread::spawn(move || {
        for stream in listener.incoming().take(2) {
            let mut stream = stream.unwrap();
            let mut request = [0u8; 2048];
            let _ = stream.read(&mut request);
            let head = format!(
                "HTTP/1.1 200 OK\r\nContent-Type: image/png\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                served.len()
            );
            stream.write_all(head.as_bytes()).unwrap();
            stream.write_all(&served).unwrap();
        }
    });

    let host = NativeHost::start(Mode::Rust(options)).unwrap();
    tokio::time::timeout(Duration::from_secs(30), host.ready())
        .await
        .unwrap()
        .unwrap();
    let controller = host.controller();
    let mut views = controller.views.clone();
    tokio::time::timeout(Duration::from_secs(10), async {
        while !views.borrow_and_update().connected {
            views.changed().await.unwrap();
        }
    })
    .await
    .unwrap();
    // Another client's aliases and metadata for the same directory.
    std::fs::write(
        &config_file,
        serde_json::to_string(&serde_json::json!({"projects": {
            "/work/app": {"favourite": true, "color": "#336699"},
            "/work/app/": {"workflowId": "wf"}
        }}))
        .unwrap(),
    )
    .unwrap();
    let url = format!("http://127.0.0.1:{port}/favicon.png");
    let saved = run(
        &controller,
        Request::SaveProject {
            path: "/work/app".into(),
            change: Patch::Identity(projects::Identity {
                label: "My App".into(),
                icon: "🦀".into(),
                favicon: url.clone(),
                icon_file: String::new(),
            }),
        },
    )
    .await;
    assert!(saved.error.is_none(), "{:?}", saved.error);
    let map = &saved.value["projects"];
    let file = map["/work/app"]["iconFile"].as_str().unwrap().to_owned();
    assert!(file.ends_with(".png") && file.len() == 36, "{file}");
    assert_eq!(std::fs::read(icons_dir.join(&file)).unwrap(), png);
    for alias in ["/work/app", "/work/app/"] {
        assert_eq!(map[alias]["label"], "My App");
        assert_eq!(map[alias]["icon"], "🦀");
        assert_eq!(map[alias]["favicon"], url.as_str());
        assert_eq!(map[alias]["iconFile"], file.as_str());
    }
    assert_eq!(map["/work/app"]["color"], "#336699");
    assert_eq!(map["/work/app"]["favourite"], true);
    assert_eq!(map["/work/app/"]["workflowId"], "wf");
    let row = projects::list(Some(&saved.value), &[], &[])
        .into_iter()
        .find(|p| p.path == "/work/app")
        .unwrap();
    assert_eq!(row.title(), "My App");
    assert_eq!(row.icon_file.as_deref(), Some(file.as_str()));

    let icons = run(
        &controller,
        Request::ProjectIcons {
            files: vec![file.clone(), "ffffffffffffffffffffffffffffffff.png".into()],
        },
    )
    .await;
    assert!(icons.error.is_none(), "{:?}", icons.error);
    assert!(
        icons.value[&file]["png"]
            .as_str()
            .is_some_and(|s| !s.is_empty())
    );
    assert!(
        icons.value["ffffffffffffffffffffffffffffffff.png"]["error"].is_string(),
        "a missing icon is reported per file"
    );

    // A refused download (not an image) saves nothing.
    let refused = run(
        &controller,
        Request::SaveProject {
            path: "/work/app".into(),
            change: Patch::Identity(projects::Identity {
                label: "Renamed".into(),
                favicon: "http://127.0.0.1:9/unreachable.png".into(),
                ..Default::default()
            }),
        },
    )
    .await;
    assert!(refused.error.unwrap().contains("download"));
    let text = std::fs::read_to_string(&config_file).unwrap();
    assert!(text.contains("My App") && !text.contains("Renamed"));

    let reset = run(
        &controller,
        Request::SaveProject {
            path: "/work/app".into(),
            change: Patch::Identity(projects::Identity::default()),
        },
    )
    .await;
    assert!(reset.error.is_none(), "{:?}", reset.error);
    assert_eq!(
        reset.value["projects"]["/work/app"],
        serde_json::json!({"favourite": true, "color": "#336699"})
    );
    assert_eq!(
        reset.value["projects"]["/work/app/"],
        serde_json::json!({"workflowId": "wf"})
    );

    drop(views);
    drop(controller);
    host.shutdown().await.unwrap();
    std::fs::remove_dir_all(root).unwrap();
}

/// Settings → Agents → Child agents start with full access reads and writes
/// the hub's shared `agents.childFullAccess` (deep-merged, verified on
/// readback) and leaves the rest of `agents` alone.
#[tokio::test]
async fn child_access_setting_round_trips_through_the_hub() {
    let root = std::env::temp_dir().join(format!(
        "wks-native-child-access-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    let mut options = RustOptions::isolated(root.join("state")).unwrap();
    options.home_dir = root.join("home");
    std::fs::create_dir(&options.home_dir).unwrap();
    options.usage_poll_on_boot = Some(false);
    let config_file = options.config_dir.join("config.yaml");
    let host = NativeHost::start(Mode::Rust(options)).unwrap();
    tokio::time::timeout(Duration::from_secs(30), host.ready())
        .await
        .unwrap()
        .unwrap();
    let controller = host.controller();
    let mut views = controller.views.clone();
    tokio::time::timeout(Duration::from_secs(10), async {
        while !views.borrow_and_update().connected {
            views.changed().await.unwrap();
        }
    })
    .await
    .unwrap();
    let read = run(&controller, Request::ChildAccess { set: None }).await;
    assert!(read.error.is_none(), "{:?}", read.error);
    assert_eq!(read.value["childFullAccess"], false, "off unless chosen");
    let on = run(&controller, Request::ChildAccess { set: Some(true) }).await;
    assert!(on.error.is_none(), "{:?}", on.error);
    assert_eq!(on.value["childFullAccess"], true);
    let text = std::fs::read_to_string(&config_file).unwrap();
    assert!(text.contains("childFullAccess: true"), "{text}");
    let reread = run(&controller, Request::ChildAccess { set: None }).await;
    assert_eq!(reread.value["childFullAccess"], true);
    assert_eq!(reread.value["fleetFullAccess"], false);
    let off = run(&controller, Request::ChildAccess { set: Some(false) }).await;
    assert!(off.error.is_none(), "{:?}", off.error);
    assert_eq!(off.value["childFullAccess"], false);
    drop(views);
    drop(controller);
    host.shutdown().await.unwrap();
    std::fs::remove_dir_all(root).unwrap();
}

/// Settings → Agents → Name sessions automatically / Title model read and
/// write the hub's shared `agents.autoTitle` one field at a time (deep-merged,
/// verified on readback), so a harness's model survives switching the
/// provider, and the legacy single `model` stays in step the way desktop
/// Settings keeps it.
#[tokio::test]
async fn title_settings_round_trip_through_the_hub() {
    use wks_native::features::TitleChange;
    let root = std::env::temp_dir().join(format!(
        "wks-native-titles-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    let mut options = RustOptions::isolated(root.join("state")).unwrap();
    options.home_dir = root.join("home");
    std::fs::create_dir(&options.home_dir).unwrap();
    options.usage_poll_on_boot = Some(false);
    let config_file = options.config_dir.join("config.yaml");
    let host = NativeHost::start(Mode::Rust(options)).unwrap();
    tokio::time::timeout(Duration::from_secs(30), host.ready())
        .await
        .unwrap()
        .unwrap();
    let controller = host.controller();
    let mut views = controller.views.clone();
    tokio::time::timeout(Duration::from_secs(10), async {
        while !views.borrow_and_update().connected {
            views.changed().await.unwrap();
        }
    })
    .await
    .unwrap();
    let read = run(&controller, Request::Titles { set: None }).await;
    assert!(read.error.is_none(), "{:?}", read.error);
    // Shared defaults: on, each agent's own harness, the legacy Claude alias.
    assert_eq!(read.value["enabled"], true);
    assert_eq!(read.value["provider"], "");
    assert_eq!(read.value["legacyModel"], "haiku");
    for change in [
        TitleChange::Model {
            provider: "claude".into(),
            model: "claude-haiku-4-5".into(),
        },
        TitleChange::Provider("codex".into()),
        TitleChange::Model {
            provider: "codex".into(),
            model: "gpt-5.4-mini".into(),
        },
        TitleChange::Enabled(false),
    ] {
        let saved = run(&controller, Request::Titles { set: Some(change) }).await;
        assert!(saved.error.is_none(), "{:?}", saved.error);
    }
    let reread = run(&controller, Request::Titles { set: None }).await;
    assert_eq!(reread.value["enabled"], false);
    assert_eq!(reread.value["provider"], "codex");
    assert_eq!(reread.value["models"]["codex"], "gpt-5.4-mini");
    assert_eq!(
        reread.value["models"]["claude"], "claude-haiku-4-5",
        "a harness's model survives choosing another provider"
    );
    assert_eq!(reread.value["legacyModel"], "gpt-5.4-mini");
    let text = std::fs::read_to_string(&config_file).unwrap();
    assert!(text.contains("provider: codex"), "{text}");
    // Back to each agent's own harness with titles on.
    for change in [
        TitleChange::Provider(String::new()),
        TitleChange::Enabled(true),
    ] {
        assert!(
            run(&controller, Request::Titles { set: Some(change) })
                .await
                .error
                .is_none()
        );
    }
    let last = run(&controller, Request::Titles { set: None }).await;
    assert_eq!(
        (
            last.value["provider"].as_str(),
            last.value["enabled"].as_bool()
        ),
        (Some(""), Some(true))
    );
    drop(views);
    drop(controller);
    host.shutdown().await.unwrap();
    std::fs::remove_dir_all(root).unwrap();
}
