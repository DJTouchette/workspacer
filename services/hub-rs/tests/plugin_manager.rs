use serde_json::{Value, json};
use std::{
    sync::{
        Arc, Mutex,
        atomic::{AtomicUsize, Ordering},
    },
    time::Duration,
};
use workspacer_hub::{
    Hub, Options,
    plugins::{
        Manager,
        manifest::Manifest,
        settings,
        supervisor::{Factory, Process, Spec, State, Supervisor, Timing},
    },
};
fn fixture(dir: &std::path::Path) -> Manifest {
    std::fs::create_dir_all(dir.join("ui")).unwrap();
    std::fs::write(dir.join("ui/index.html"), "hello").unwrap();
    std::fs::write(dir.join("plugin.json"),serde_json::to_vec(&json!({"id":"fixture","apiVersion":"1","ui":"ui","provides":["fixture.*"],"settings":[{"key":"secret","type":"string","secret":true},{"key":"size","type":"number","default":10}]})).unwrap()).unwrap();
    Manifest::load(&dir.join("plugin.json")).unwrap()
}
#[test]
fn typed_settings_are_atomic_secret_redacted_and_persistent() {
    let dir = tempfile::tempdir().unwrap();
    let m = fixture(dir.path());
    assert_eq!(
        settings::update(
            &m,
            json!({"secret":"sensitive","size":20}).as_object().unwrap()
        )
        .unwrap(),
        json!({"secret":"__WKS_SECRET__","size":20})
    );
    assert!(
        settings::update(
            &m,
            json!({"secret":"replacement","size":"wrong"})
                .as_object()
                .unwrap()
        )
        .is_err()
    );
    assert_eq!(settings::merged(&m)["secret"], "sensitive");
    settings::update(
        &m,
        json!({"secret":"__WKS_SECRET__","size":null})
            .as_object()
            .unwrap(),
    )
    .unwrap();
    assert_eq!(settings::merged(&m)["secret"], "sensitive");
    assert_eq!(settings::merged(&m)["size"], 10);
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        assert_eq!(
            std::fs::metadata(dir.path().join(".settings.json"))
                .unwrap()
                .permissions()
                .mode()
                & 0o777,
            0o600
        );
    }
}
#[test]
fn manifests_reject_namespace_secrets_and_ui_escapes() {
    for patch in [
        json!({"provides":["agents.spawn"]}),
        json!({"ui":"../"}),
        json!({"ui":"C:\\secret"}),
        json!({"ui":"."}),
        json!({"settings":[{"key":"k","type":"string","secret":true,"scope":"project"}]}),
        json!({"settings":[{"key":"k","type":"string","secret":true,"default":"plaintext"}]}),
        json!({"tools":[{"name":"x","description":"x","method":"agents.spawn"}]}),
    ] {
        let mut value = json!({"id":"fixture","apiVersion":"1"});
        value
            .as_object_mut()
            .unwrap()
            .extend(patch.as_object().unwrap().clone());
        assert!(
            serde_json::from_value::<Manifest>(value)
                .unwrap()
                .validate()
                .is_err()
        );
    }
}
#[tokio::test]
async fn lifecycle_revokes_panes_and_disabled_identities_and_preserves_token() {
    let dir = tempfile::tempdir().unwrap();
    let m = fixture(dir.path());
    let hub = Hub::start(Options::default()).unwrap();
    hub.ready().await.unwrap();
    let mut manager = Manager::new(dir.path().into(), hub.handle(), String::new());
    manager.add(m).await.unwrap();
    let stable = std::fs::read_to_string(dir.path().join(".bus-token")).unwrap();
    let pane = manager.pane_token("fixture").await.unwrap();
    let mut conn = hub
        .handle()
        .connect_authenticated(pane.clone(), false)
        .await
        .unwrap();
    conn.recv().await.unwrap();
    manager.set_enabled("fixture", false).await.unwrap();
    assert!(conn.recv().await.is_none());
    assert!(
        hub.handle()
            .connect_authenticated(stable.clone(), false)
            .await
            .is_err()
    );
    manager.set_enabled("fixture", true).await.unwrap();
    assert_eq!(
        std::fs::read_to_string(dir.path().join(".bus-token")).unwrap(),
        stable
    );
    assert!(
        hub.handle()
            .connect_authenticated(stable.clone(), false)
            .await
            .is_ok()
    );
    manager.stop().await.unwrap();
    assert!(
        hub.handle()
            .connect_authenticated(stable, false)
            .await
            .is_err()
    );
    hub.shutdown().unwrap();
}
struct FakeFactory {
    starts: AtomicUsize,
    alive: Arc<AtomicUsize>,
    env: Mutex<Vec<Value>>,
}
struct FakeChild {
    alive: Arc<AtomicUsize>,
    polls: usize,
    stopped: bool,
}
impl Process for FakeChild {
    fn pid(&self) -> u32 {
        42
    }
    fn exit_error(&self) -> String {
        "exit status 7".into()
    }
    fn exited(&mut self) -> anyhow::Result<bool> {
        self.polls += 1;
        Ok(self.polls >= 2)
    }
    fn stop(&mut self) -> anyhow::Result<()> {
        if !self.stopped {
            self.alive.fetch_sub(1, Ordering::SeqCst);
            self.stopped = true;
        }
        Ok(())
    }
}
impl Factory for FakeFactory {
    fn spawn(&self, spec: &Spec) -> anyhow::Result<Box<dyn Process>> {
        self.starts.fetch_add(1, Ordering::SeqCst);
        assert_eq!(
            self.alive.fetch_add(1, Ordering::SeqCst),
            0,
            "overlapping child processes"
        );
        self.env.lock().unwrap().push(json!(spec.env));
        Ok(Box::new(FakeChild {
            alive: self.alive.clone(),
            polls: 0,
            stopped: false,
        }))
    }
}
#[test]
fn fake_process_restarts_and_stop_joins_owner() {
    let f = Arc::new(FakeFactory {
        starts: AtomicUsize::new(0),
        alive: Arc::new(AtomicUsize::new(0)),
        env: Mutex::new(vec![]),
    });
    let (tx, rx) = std::sync::mpsc::channel();
    let mut sup = Supervisor::start(
        Spec {
            command: "fake".into(),
            health_url: None,
            log: None,
            args: vec![],
            directory: ".".into(),
            env: Default::default(),
        },
        f.clone(),
        Timing {
            initial: Duration::from_millis(1),
            maximum: Duration::from_millis(4),
            reset_after: Duration::from_secs(1),
            poll: Duration::from_millis(1),
            health_period: Duration::from_millis(1),
        },
        Arc::new(move |s| {
            tx.send(s).unwrap();
        }),
    )
    .unwrap();
    let mut running = 0;
    while running < 2 {
        if rx.recv_timeout(Duration::from_secs(2)).unwrap() == State::Running {
            running += 1;
        }
    }
    sup.stop();
    assert_eq!(sup.state(), State::Stopped);
    assert_eq!(f.alive.load(Ordering::SeqCst), 0);
    assert!(f.starts.load(Ordering::SeqCst) >= 2);
}
#[test]
fn install_excludes_credentials_and_rejects_existing_destination() {
    let src = tempfile::tempdir().unwrap();
    fixture(src.path());
    std::fs::write(src.path().join(".bus-token"), "must-not-copy").unwrap();
    let dest = tempfile::tempdir().unwrap();
    let installed =
        workspacer_hub::plugins::install::install_local(src.path(), dest.path()).unwrap();
    assert!(!installed.dir.join(".bus-token").exists());
    assert!(installed.dir.join("ui/index.html").exists());
    assert!(workspacer_hub::plugins::install::install_local(src.path(), dest.path()).is_err());
    workspacer_hub::plugins::install::uninstall_directory(dest.path(), &installed.dir).unwrap();
    assert!(!installed.dir.exists());
}
#[cfg(unix)]
#[tokio::test]
async fn ui_symlink_cannot_expose_host_credential() {
    let dir = tempfile::tempdir().unwrap();
    let m = fixture(dir.path());
    std::fs::write(dir.path().join(".settings.json"), "secret").unwrap();
    std::os::unix::fs::symlink("../.settings.json", dir.path().join("ui/leak")).unwrap();
    let hub = Hub::start(Options::default()).unwrap();
    hub.ready().await.unwrap();
    let mut manager = Manager::new(dir.path().into(), hub.handle(), String::new());
    manager.add(m).await.unwrap();
    assert!(manager.ui_file("fixture", "leak").is_err());
    assert!(manager.ui_file("fixture", "index.html").is_ok());
    manager.stop().await.unwrap();
    hub.shutdown().unwrap();
}

fn archive(entries: Vec<(&str, &[u8])>) -> Vec<u8> {
    let encoder = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::default());
    let mut builder = tar::Builder::new(encoder);
    for (path, bytes) in entries {
        let mut h = tar::Header::new_gnu();
        h.set_size(bytes.len() as u64);
        h.set_mode(0o644);
        h.set_cksum();
        builder.append_data(&mut h, path, bytes).unwrap();
    }
    builder.into_inner().unwrap().finish().unwrap()
}
fn serve_archive(bytes: Vec<u8>) -> String {
    use std::io::{Read, Write};
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let address = listener.local_addr().unwrap();
    std::thread::spawn(move || {
        let (mut socket, _) = listener.accept().unwrap();
        let mut request = [0; 4096];
        socket.read(&mut request).unwrap();
        write!(
            socket,
            "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
            bytes.len()
        )
        .unwrap();
        socket.write_all(&bytes).unwrap();
    });
    format!("http://{address}/plugin.tar.gz")
}
#[test]
fn remote_install_consent_and_atomic_update_preserve_host_state() {
    use workspacer_hub::plugins::install::{Consent, ConsentRequired, prepare};
    let root = tempfile::tempdir().unwrap();
    let original = fixture(&root.path().join("legacy-fixture-dir"));
    std::fs::write(original.dir.join(".bus-token"), "old-identity").unwrap();
    std::fs::write(
        original.dir.join(".settings.json"),
        r#"{"secret":"old-secret"}"#,
    )
    .unwrap();
    let manifest=serde_json::to_vec(&json!({"id":"fixture","apiVersion":"1","ui":"ui","install":["definitely-nonexistent-program"]})).unwrap();
    let url = serve_archive(archive(vec![("repo/plugin.json", &manifest)]));
    let err = match prepare(root.path(), &url, &Consent::default()) {
        Ok(_) => panic!("consent not required"),
        Err(e) => e,
    };
    assert!(err.downcast_ref::<ConsentRequired>().is_some());
    assert!(original.dir.exists());
    let manifest = serde_json::to_vec(&json!({"id":"fixture","apiVersion":"1","ui":"ui"})).unwrap();
    let url = serve_archive(archive(vec![
        ("repo/plugin.json", &manifest),
        ("repo/.bus-token", b"attacker"),
        ("repo/ui/index.html", b"new"),
    ]));
    let prepared = prepare(root.path(), &url, &Consent::default()).unwrap();
    assert_eq!(
        std::fs::read_to_string(original.dir.join("ui/index.html")).unwrap(),
        "hello"
    );
    let new = prepared.commit(root.path()).unwrap();
    assert_eq!(new.dir, original.dir);
    assert_eq!(
        std::fs::read_to_string(new.dir.join(".bus-token")).unwrap(),
        "old-identity"
    );
    assert_eq!(
        std::fs::read_to_string(new.dir.join(".settings.json")).unwrap(),
        r#"{"secret":"old-secret"}"#
    );
    assert_eq!(
        std::fs::read_to_string(new.dir.join("ui/index.html")).unwrap(),
        "new"
    );
}
#[test]
fn extraction_rejects_links() {
    use workspacer_hub::plugins::install::extract_tar_gz;
    let dir = tempfile::tempdir().unwrap();
    let encoder = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::default());
    let mut builder = tar::Builder::new(encoder);
    let mut header = tar::Header::new_gnu();
    header.set_entry_type(tar::EntryType::Symlink);
    header.set_size(0);
    header.set_link_name("/etc/passwd").unwrap();
    header.set_cksum();
    builder.append_data(&mut header, "leak", &[][..]).unwrap();
    let bytes = builder.into_inner().unwrap().finish().unwrap();
    assert!(extract_tar_gz(&bytes[..], dir.path()).is_err());
    assert!(!dir.path().join("leak").exists());
}
#[tokio::test]
async fn http_public_manifest_omits_private_fields_and_host_mutations_work() {
    let dir = tempfile::tempdir().unwrap();
    let manifest = fixture(dir.path());
    std::fs::write(dir.path().join("ui/app.js"), "export const value = 1;\n").unwrap();
    std::fs::write(dir.path().join("ui/style.css"), "body{color:red}").unwrap();
    let hub = Hub::start(Options::default()).unwrap();
    hub.ready().await.unwrap();
    let mut manager = Manager::new(dir.path().into(), hub.handle(), String::new());
    manager.add(manifest).await.unwrap();
    let manager = Arc::new(tokio::sync::Mutex::new(manager));
    let router = workspacer_hub::plugins::http::router(manager.clone(), "host-key".into());
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let task = tokio::spawn(async move { axum::serve(listener, router).await.unwrap() });
    let client = reqwest::Client::new();
    let base = format!("http://{address}");
    let sdk = client
        .get(format!("{base}/plugins/sdk.js"))
        .send()
        .await
        .unwrap();
    assert_eq!(sdk.status(), 200);
    assert_eq!(
        sdk.headers()["content-type"],
        "application/javascript; charset=utf-8"
    );
    assert_eq!(sdk.headers()["cache-control"], "public, max-age=300");
    assert!(sdk.text().await.unwrap().contains("window.workspacer"));
    let public: Value = client
        .get(format!("{base}/plugins"))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert!(public[0].get("settings").is_none());
    assert!(public[0].get("provides").is_none());
    assert!(public[0].as_object().unwrap().keys().all(|key| {
        [
            "id",
            "name",
            "apiVersion",
            "version",
            "disabled",
            "panes",
            "widgets",
            "hotkeys",
        ]
        .contains(&key.as_str())
    }));
    let private: Value = client
        .get(format!("{base}/plugins"))
        .bearer_auth("host-key")
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(private[0]["provides"], json!(["fixture.*"]));
    assert!(private[0]["settings"].is_array());
    for (file, expected) in [
        ("app.js", "export const value = 1;\n"),
        ("style.css", "body{color:red}"),
    ] {
        let response = client
            .get(format!("{base}/plugins/ui/fixture/{file}"))
            .send()
            .await
            .unwrap();
        assert_eq!(response.status(), 200);
        assert_eq!(response.text().await.unwrap(), expected);
    }
    assert_eq!(
        client
            .get(format!("{base}/plugins/tokens"))
            .send()
            .await
            .unwrap()
            .status(),
        401
    );
    assert_eq!(
        client
            .get(format!(
                "{base}/plugins/settings?pluginId=fixture&token=host-key"
            ))
            .header("Authorization", "Basic invalid")
            .send()
            .await
            .unwrap()
            .status(),
        401
    );
    let res = client
        .post(format!("{base}/plugins/settings"))
        .bearer_auth("host-key")
        .json(&json!({"pluginId":"fixture","values":{"secret":"sensitive"}}))
        .send()
        .await
        .unwrap();
    assert_eq!(res.status(), 200);
    let values: Value = res.json().await.unwrap();
    assert_eq!(values["values"]["secret"], "__WKS_SECRET__");
    let public_ui = client
        .get(format!("{base}/plugins/ui/fixture/"))
        .send()
        .await
        .unwrap()
        .text()
        .await
        .unwrap();
    assert!(public_ui.contains("__WKS_PLUGIN_ID__"));
    assert!(!public_ui.contains("__WKS_SETTINGS__"));
    let pane = manager.lock().await.pane_token("fixture").await.unwrap();
    let own_ui = client
        .get(format!("{base}/plugins/ui/fixture/?busToken={pane}"))
        .send()
        .await
        .unwrap()
        .text()
        .await
        .unwrap();
    assert!(own_ui.contains("__WKS_SETTINGS__"));
    assert!(own_ui.contains("__WKS_SECRET__"));
    assert!(!own_ui.contains("sensitive"));
    // The standalone router also retains Go's explicitly unkeyed local mode.
    let unkeyed = workspacer_hub::plugins::http::router(manager.clone(), String::new());
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let unkeyed_address = listener.local_addr().unwrap();
    let unkeyed_task = tokio::spawn(async move { axum::serve(listener, unkeyed).await.unwrap() });
    assert_eq!(
        client
            .post(format!("http://{unkeyed_address}/plugins/reload"))
            .json(&json!({"dir":dir.path()}))
            .send()
            .await
            .unwrap()
            .status(),
        200
    );
    unkeyed_task.abort();
    let _ = unkeyed_task.await;
    manager.lock().await.remove("fixture").await.unwrap();
    let empty: Value = client
        .get(format!("{base}/plugins"))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(empty, json!([]));
    manager.lock().await.stop().await.unwrap();
    task.abort();
    hub.shutdown().unwrap();
}

#[cfg(target_os = "linux")]
#[tokio::test]
async fn runtime_shutdown_stops_plugin_before_broker_and_reaps_direct_child() {
    let root = tempfile::tempdir().unwrap();
    let directory = root.path().join("fixture");
    std::fs::create_dir(&directory).unwrap();
    std::fs::write(directory.join("plugin.json"),serde_json::to_vec(&json!({"id":"fixture","apiVersion":"1","server":{"command":"/bin/sh","args":["-c","echo $$ > child.pid; exec sleep 60"]},"provides":["fixture.*"]})).unwrap()).unwrap();
    let mut options = Options::default();
    options.listen = Some("127.0.0.1:0".parse().unwrap());
    options.token = "host-key".into();
    options.plugins_dir = Some(root.path().into());
    let hub = Hub::start(options).unwrap();
    hub.ready().await.unwrap();
    let pid = tokio::time::timeout(Duration::from_secs(3), async {
        loop {
            if let Ok(pid) = std::fs::read_to_string(directory.join("child.pid")) {
                break pid.trim().to_string();
            }
            tokio::time::sleep(Duration::from_millis(10)).await
        }
    })
    .await
    .unwrap();
    assert!(std::path::Path::new(&format!("/proc/{pid}")).exists());
    let started = std::time::Instant::now();
    hub.shutdown().unwrap();
    assert!(started.elapsed() < Duration::from_secs(3));
    assert!(!std::path::Path::new(&format!("/proc/{pid}")).exists());
}

#[test]
fn version_comparison_never_offers_downgrades() {
    use workspacer_hub::plugins::install::compare_versions;
    assert!(compare_versions("1.10", "1.9").is_gt());
    assert!(compare_versions("v1.2.0-beta", "1.2").is_eq());
    assert!(compare_versions("1.1", "1.2").is_lt());
}
struct HealthFactory {
    starts: AtomicUsize,
    probes: AtomicUsize,
}
struct LivingChild;
impl Process for LivingChild {
    fn exited(&mut self) -> anyhow::Result<bool> {
        Ok(false)
    }
    fn stop(&mut self) -> anyhow::Result<()> {
        Ok(())
    }
}
impl Factory for HealthFactory {
    fn spawn(&self, _: &Spec) -> anyhow::Result<Box<dyn Process>> {
        self.starts.fetch_add(1, Ordering::SeqCst);
        Ok(Box::new(LivingChild))
    }
    fn healthy(&self, _: &str) -> bool {
        self.probes.fetch_add(1, Ordering::SeqCst) == 0
    }
}
#[test]
fn failed_health_probe_does_not_restart_live_child() {
    let factory = Arc::new(HealthFactory {
        starts: AtomicUsize::new(0),
        probes: AtomicUsize::new(0),
    });
    let (tx, rx) = std::sync::mpsc::channel();
    let mut supervisor = Supervisor::start(
        Spec {
            command: "fake".into(),
            args: vec![],
            directory: ".".into(),
            env: Default::default(),
            health_url: Some("http://fixture/health".into()),
            log: None,
        },
        factory.clone(),
        Timing {
            health_period: Duration::from_millis(1),
            poll: Duration::from_millis(1),
            ..Timing::default()
        },
        Arc::new(move |s| {
            tx.send(s).unwrap();
        }),
    )
    .unwrap();
    loop {
        if rx.recv_timeout(Duration::from_secs(2)).unwrap() == State::Unhealthy {
            break;
        }
    }
    supervisor.stop();
    assert_eq!(factory.starts.load(Ordering::SeqCst), 1);
}

#[cfg(unix)]
#[test]
fn native_supervisor_streams_lines_and_flushes_partial_without_reader_threads() {
    let root = tempfile::tempdir().unwrap();
    let (tx, rx) = std::sync::mpsc::channel();
    let log = Arc::new(move |stream: &str, line: &str| {
        tx.send((stream.to_owned(), line.to_owned())).unwrap();
    });
    let mut supervisor = Supervisor::start(
        Spec {
            command: "/bin/sh".into(),
            args: vec![
                "-c".into(),
                "printf 'hello\\n'; printf 'partial' >&2; exec sleep 60".into(),
            ],
            directory: root.path().into(),
            env: Default::default(),
            health_url: None,
            log: Some(log),
        },
        Arc::new(workspacer_hub::plugins::supervisor::NativeFactory),
        Timing::default(),
        Arc::new(|_| {}),
    )
    .unwrap();
    assert_eq!(
        rx.recv_timeout(Duration::from_secs(2)).unwrap(),
        ("stdout".into(), "hello".into())
    );
    supervisor.stop();
    assert_eq!(
        rx.recv_timeout(Duration::from_secs(2)).unwrap(),
        ("stderr".into(), "partial".into())
    );
    assert_eq!(supervisor.state(), State::Stopped);
}
#[test]
fn extraction_rejects_traversal_and_size_bombs_before_writing() {
    use std::io::Write;
    for oversized in [false, true] {
        let mut header = tar::Header::new_gnu();
        header.set_mode(0o644);
        header.set_entry_type(tar::EntryType::Regular);
        if oversized {
            header.set_path("large").unwrap();
            header.set_size(129 << 20);
        } else {
            header.set_size(0);
            header.as_mut_bytes()[..9].copy_from_slice(b"../escape");
        }
        header.set_cksum();
        let mut encoder = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::default());
        encoder.write_all(header.as_bytes()).unwrap();
        encoder.write_all(&[0u8; 1024]).unwrap();
        let bytes = encoder.finish().unwrap();
        let root = tempfile::tempdir().unwrap();
        assert!(workspacer_hub::plugins::install::extract_tar_gz(&bytes[..], root.path()).is_err());
        assert_eq!(std::fs::read_dir(root.path()).unwrap().count(), 0);
    }
}

#[tokio::test]
async fn http_operator_cannot_install_or_reload_and_bundled_examples_are_host_owned() {
    use workspacer_hub::{
        auth,
        plugins::http::{HttpOptions, router_with_options},
    };
    let root = tempfile::tempdir().unwrap();
    let examples = root.path().join("examples");
    let example = examples.join("fixture");
    std::fs::create_dir_all(&example).unwrap();
    fixture(&example);
    let records = root.path().join("tokens.json");
    let operator = auth::mint(&records, auth::Scope::Operator, "fixture").unwrap();
    let restricted = [
        auth::Scope::View,
        auth::Scope::Triage,
        auth::Scope::Provider,
    ]
    .into_iter()
    .map(|scope| {
        auth::mint(&records, scope, "restricted-fixture")
            .unwrap()
            .token
    })
    .collect::<Vec<_>>();
    let hub = Hub::start(Options::default()).unwrap();
    hub.ready().await.unwrap();
    let manager = Arc::new(tokio::sync::Mutex::new(Manager::new(
        root.path().join("plugins"),
        hub.handle(),
        String::new(),
    )));
    let router = router_with_options(
        manager.clone(),
        HttpOptions {
            host_token: "host-key".into(),
            scoped_tokens: Some(records),
            plugin_origin: "http://localhost:7896/path?q=x".into(),
            examples_dir: Some(examples),
        },
    )
    .unwrap();
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let base = format!("http://{}", listener.local_addr().unwrap());
    let task = tokio::spawn(async move { axum::serve(listener, router).await.unwrap() });
    let client = reqwest::Client::new();
    for route in ["install", "reload", "examples/install"] {
        for token in std::iter::once("")
            .chain(std::iter::once("invalid-fixture"))
            .chain(restricted.iter().map(String::as_str))
        {
            let response = client
                .post(format!("{base}/plugins/{route}"))
                .bearer_auth(token)
                .json(&json!({}))
                .send()
                .await
                .unwrap();
            assert_eq!(
                response.status(),
                401,
                "{route} admitted a non-operator credential"
            );
        }
    }
    let public: Value = client
        .get(format!("{base}/plugins/examples"))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(public[0]["id"], "fixture");
    assert!(public[0].get("settings").is_none());
    let full: Value = client
        .get(format!("{base}/plugins/examples"))
        .bearer_auth(&operator.token)
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert!(full[0].get("settings").is_some());
    for route in ["reload", "install", "examples/install"] {
        let response = client
            .post(format!("{base}/plugins/{route}"))
            .bearer_auth(&operator.token)
            .json(&json!({"id":"fixture","dir":example}))
            .send()
            .await
            .unwrap();
        assert_eq!(response.status(), 403);
        let body = response.text().await.unwrap();
        assert!(body.contains("host authority"));
        assert!(!body.contains(&operator.token));
        assert!(!body.contains("restricted-fixture"));
    }
    let result = client
        .post(format!("{base}/plugins/examples/install"))
        .bearer_auth("host-key")
        .json(&json!({"id":"fixture"}))
        .send()
        .await
        .unwrap();
    assert_eq!(result.status(), 200);
    assert!(root.path().join("plugins/fixture/plugin.json").exists());
    for body in [
        json!({"id":"fixture"}),
        json!({"id":"fixture","enabled":null}),
    ] {
        assert_eq!(
            client
                .post(format!("{base}/plugins/setEnabled"))
                .bearer_auth("host-key")
                .json(&body)
                .send()
                .await
                .unwrap()
                .status(),
            200
        );
        assert!(
            manager
                .lock()
                .await
                .list()
                .into_iter()
                .find(|m| m.id == "fixture")
                .unwrap()
                .disabled
        );
        client
            .post(format!("{base}/plugins/setEnabled"))
            .bearer_auth("host-key")
            .json(&json!({"id":"fixture","enabled":true}))
            .send()
            .await
            .unwrap()
            .error_for_status()
            .unwrap();
    }
    assert_eq!(
        client
            .post(format!("{base}/plugins/setEnabled"))
            .bearer_auth("host-key")
            .json(&json!({"id":"fixture","enabled":"false"}))
            .send()
            .await
            .unwrap()
            .status(),
        400
    );
    assert!(
        !manager
            .lock()
            .await
            .list()
            .into_iter()
            .find(|m| m.id == "fixture")
            .unwrap()
            .disabled
    );
    let origin: Value = client
        .get(format!("{base}/plugins/origin"))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(origin["origin"], "http://localhost:7896");
    assert_eq!(
        client
            .get(format!("{base}/plugins/settings?pluginId=fixture"))
            .bearer_auth(&operator.token)
            .send()
            .await
            .unwrap()
            .status(),
        200
    );
    manager.lock().await.stop().await.unwrap();
    task.abort();
    hub.shutdown().unwrap();
}
#[test]
fn plugin_origin_rejects_script_schemes() {
    assert!(workspacer_hub::plugins::http::normalize_origin("javascript:alert(1)").is_err());
}

#[test]
fn bundled_seed_never_executes_code_or_replaces_existing_plugins() {
    use workspacer_hub::plugins::install::seed_bundled;
    let root = tempfile::tempdir().unwrap();
    let examples = root.path().join("examples");
    let dir = examples.join("editor");
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(
        dir.join("plugin.json"),
        serde_json::to_vec(&json!({"id":"workspacer.editor","apiVersion":"1","ui":"ui"})).unwrap(),
    )
    .unwrap();
    std::fs::create_dir(dir.join("ui")).unwrap();
    let dest = root.path().join("plugins");
    seed_bundled(&dest, &examples).unwrap();
    assert!(dest.join("editor/plugin.json").exists());
    std::fs::write(dir.join("plugin.json"), "invalid").unwrap();
    seed_bundled(&dest, &examples).unwrap();
    assert!(Manifest::load(&dest.join("editor/plugin.json")).is_ok());
    let empty = root.path().join("empty");
    std::fs::write(
        dir.join("plugin.json"),
        serde_json::to_vec(
            &json!({"id":"workspacer.editor","apiVersion":"1","server":{"command":"never-run"}}),
        )
        .unwrap(),
    )
    .unwrap();
    assert!(seed_bundled(&empty, &examples).is_err());
    assert_eq!(std::fs::read_dir(empty).unwrap().count(), 0);
}

#[cfg(target_os = "linux")]
#[test]
fn cancelling_an_install_reaps_its_build_process_before_returning() {
    use workspacer_hub::plugins::install::{Consent, prepare_directory_cancellable};
    let root = tempfile::tempdir().unwrap();
    let source = root.path().join("source");
    std::fs::create_dir(&source).unwrap();
    let pidfile = root.path().join("build.pid");
    let argv = vec![
        "/bin/sh".to_owned(),
        "-c".into(),
        "echo $$ > \"$1\"; exec sleep 60".into(),
        "fixture".into(),
        pidfile.to_string_lossy().into_owned(),
    ];
    std::fs::write(
        source.join("plugin.json"),
        serde_json::to_vec(&json!({"id":"fixture","apiVersion":"1","install":argv})).unwrap(),
    )
    .unwrap();
    let destination = root.path().join("plugins");
    let cancel = Arc::new(std::sync::atomic::AtomicBool::new(false));
    let worker_cancel = cancel.clone();
    let worker = std::thread::spawn(move || {
        prepare_directory_cancellable(
            &destination,
            &source,
            &Consent {
                allow_install_command: true,
                consented_argv: argv,
            },
            &worker_cancel,
        )
        .err()
        .map(|e| e.to_string())
    });
    let deadline = std::time::Instant::now() + Duration::from_secs(3);
    let pid = loop {
        if let Ok(pid) = std::fs::read_to_string(&pidfile) {
            break pid.trim().to_string();
        }
        assert!(std::time::Instant::now() < deadline);
        std::thread::sleep(Duration::from_millis(5));
    };
    cancel.store(true, Ordering::Release);
    assert!(worker.join().unwrap().unwrap().contains("cancelled"));
    assert!(!std::path::Path::new(&format!("/proc/{pid}")).exists());
}

#[tokio::test]
async fn supervisor_events_match_go_topics_source_pid_and_error_fields() {
    use workspacer_hub::{plugins::manifest::Server, protocol::Frame};
    let root = tempfile::tempdir().unwrap();
    let mut manifest = fixture(root.path());
    manifest.server = Some(Server {
        command: "fake".into(),
        ..Server::default()
    });
    let factory = Arc::new(FakeFactory {
        starts: AtomicUsize::new(0),
        alive: Arc::new(AtomicUsize::new(0)),
        env: Mutex::new(vec![]),
    });
    let hub = Hub::start(Options::default()).unwrap();
    hub.ready().await.unwrap();
    let mut watcher = hub.handle().connect().await.unwrap();
    watcher.recv().await.unwrap();
    watcher
        .send(Frame {
            topics: vec!["sidecar.*".into()],
            ..Frame::op("subscribe")
        })
        .unwrap();
    assert_eq!(watcher.recv().await.unwrap().op, "subscribed");
    let mut manager =
        Manager::new(root.path().into(), hub.handle(), String::new()).with_factory(factory);
    manager.add(manifest).await.unwrap();
    let running = tokio::time::timeout(Duration::from_secs(2), watcher.recv())
        .await
        .unwrap()
        .unwrap()
        .event
        .unwrap();
    assert_eq!(running.topic, "sidecar.running");
    assert_eq!(running.source, "supervisor");
    assert_eq!(
        running.data,
        Some(json!({"name":"fixture","state":"running","pid":42}))
    );
    let crashed = tokio::time::timeout(Duration::from_secs(2), watcher.recv())
        .await
        .unwrap()
        .unwrap()
        .event
        .unwrap();
    assert_eq!(crashed.topic, "sidecar.crashed");
    assert_eq!(crashed.source, "supervisor");
    assert_eq!(
        crashed.data,
        Some(json!({"name":"fixture","state":"crashed","err":"exit status 7"}))
    );
    manager.stop().await.unwrap();
    let stopped = tokio::time::timeout(Duration::from_secs(2), watcher.recv())
        .await
        .unwrap()
        .unwrap()
        .event
        .unwrap();
    assert_eq!(stopped.topic, "sidecar.stopped");
    assert_eq!(
        stopped.data,
        Some(json!({"name":"fixture","state":"stopped"}))
    );
    hub.shutdown().unwrap();
}

#[tokio::test]
async fn dev_identity_change_retires_old_directory_owner_and_announces_unload() {
    use workspacer_hub::protocol::Frame;
    let root = tempfile::tempdir().unwrap();
    let manifest = fixture(root.path());
    let hub = Hub::start(Options::default()).unwrap();
    hub.ready().await.unwrap();
    let mut manager = Manager::new(root.path().into(), hub.handle(), String::new());
    manager.add(manifest.clone()).await.unwrap();
    let token = manager.pane_token("fixture").await.unwrap();
    let mut old_pane = hub
        .handle()
        .connect_authenticated(token, false)
        .await
        .unwrap();
    old_pane.recv().await.unwrap();
    let mut watcher = hub.handle().connect().await.unwrap();
    watcher.recv().await.unwrap();
    watcher
        .send(Frame {
            topics: vec!["plugin.*".into()],
            ..Frame::op("subscribe")
        })
        .unwrap();
    watcher.recv().await.unwrap();
    let mut renamed = manifest;
    renamed.id = "renamed".into();
    renamed.provides = vec!["renamed.*".into()];
    manager.add(renamed).await.unwrap();
    assert!(old_pane.recv().await.is_none());
    assert_eq!(manager.list().len(), 1);
    assert_eq!(manager.list()[0].id, "renamed");
    let first = tokio::time::timeout(Duration::from_secs(2), watcher.recv())
        .await
        .unwrap()
        .unwrap()
        .event
        .unwrap();
    assert_eq!(first.topic, "plugin.unloaded");
    assert_eq!(first.data, Some(json!({"id":"fixture"})));
    let next = tokio::time::timeout(Duration::from_secs(2), watcher.recv())
        .await
        .unwrap()
        .unwrap()
        .event
        .unwrap();
    assert_eq!(next.topic, "plugin.loaded");
    manager.stop().await.unwrap();
    hub.shutdown().unwrap();
}

#[tokio::test]
async fn ui_settings_require_a_current_plugin_credential() {
    let dir = tempfile::tempdir().unwrap();
    let manifest = fixture(dir.path());
    let hub = Hub::start(Options::default()).unwrap();
    hub.ready().await.unwrap();
    let mut manager = Manager::new(dir.path().into(), hub.handle(), String::new());
    manager.add(manifest).await.unwrap();
    let pane = manager.pane_token("fixture").await.unwrap();
    let manager = Arc::new(tokio::sync::Mutex::new(manager));
    let router = workspacer_hub::plugins::http::router(manager.clone(), "host-key".into());
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let base = format!("http://{}", listener.local_addr().unwrap());
    let task = tokio::spawn(async move { axum::serve(listener, router).await.unwrap() });
    let client = reqwest::Client::new();
    let tokens: Value = client
        .get(format!("{base}/plugins/tokens"))
        .bearer_auth("host-key")
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    let stable = tokens["fixture"].as_str().unwrap().to_owned();
    hub.handle()
        .register_plugin("other-token".into(), "other".into(), vec![])
        .await
        .unwrap();
    for (token, expected) in [
        ("", false),
        ("wrong", false),
        ("other-token", false),
        (stable.as_str(), true),
        (pane.as_str(), true),
        ("host-key", true),
    ] {
        for form in ["token", "busToken", "bearer"] {
            // busToken is a plugin identity transport, never an owner credential.
            if token == "host-key" && form == "busToken" {
                continue;
            }
            let request = client.get(format!("{base}/plugins/ui/fixture/"));
            let request = if form == "bearer" {
                request.bearer_auth(token)
            } else {
                request.query(&[(form, token)])
            };
            let body = request.send().await.unwrap().text().await.unwrap();
            assert_eq!(
                body.contains("__WKS_SETTINGS__"),
                expected,
                "{form} token={token:?}"
            );
        }
    }
    // Revoke directly in the authority owner, leaving the manager's cache intact.
    for token in [&stable, &pane] {
        hub.handle().revoke_plugin(token.clone()).await.unwrap();
        for form in ["token", "busToken", "bearer"] {
            let request = client.get(format!("{base}/plugins/ui/fixture/"));
            let request = if form == "bearer" {
                request.bearer_auth(token)
            } else {
                request.query(&[(form, token.as_str())])
            };
            let response = request.send().await.unwrap();
            assert_eq!(response.status(), 200, "public UI remains available");
            assert!(
                !response.text().await.unwrap().contains("__WKS_SETTINGS__"),
                "revoked {form}"
            );
        }
    }
    task.abort();
    let _ = task.await;
    manager.lock().await.stop().await.unwrap();
    hub.shutdown().unwrap();
}
