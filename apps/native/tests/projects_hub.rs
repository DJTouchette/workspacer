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
