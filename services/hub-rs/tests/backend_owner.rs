use claudemon::daemon::{ServeConfig, embedded::Options as EngineOptions};
use workspacer_hub::{Options, backend::Backend};
static ENGINE_LOCK: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

#[tokio::test]
async fn cancelling_before_readiness_and_failed_initialization_both_release_the_engine() {
    let _guard = ENGINE_LOCK.lock().await;
    let directory = tempfile::tempdir().unwrap();
    let config = || ServeConfig {
        host: "127.0.0.1".into(),
        hook_port: 0,
        api_port: 0,
        db_path: directory.path().join("state.db"),
    };
    let engine = || EngineOptions {
        usage_poll_on_boot: Some(false),
    };
    // The GUI can quit before it has even polled the readiness future.
    let owner = Backend::prepare(config(), engine()).unwrap();
    owner.shutdown().await.unwrap();
    let mut owner = Backend::prepare(config(), engine()).unwrap();
    let mut invalid = Options::default();
    invalid.call_timeout = std::time::Duration::ZERO;
    assert!(owner.initialize(invalid).await.is_err());
    owner.shutdown().await.unwrap();
    // Neither path leaves callback addresses/SQLite ownership leased forever.
    let owner = Backend::start(config(), engine(), Options::default())
        .await
        .unwrap();
    assert!(owner.handle().ready().await.unwrap().is_none());
    owner.shutdown().await.unwrap();
}

#[test]
fn backend_database_lease_child() {
    let Some(path) = std::env::var_os("WKS_BACKEND_OWNER_DB") else {
        return;
    };
    let config = ServeConfig {
        host: "127.0.0.1".into(),
        hook_port: 0,
        api_port: 0,
        db_path: path.into(),
    };
    let options = EngineOptions {
        usage_poll_on_boot: Some(false),
    };
    match std::env::var("WKS_BACKEND_OWNER_CASE").unwrap().as_str() {
        "refuse" => {
            let error = match Backend::prepare(config, options) {
                Ok(owner) => {
                    drop(owner);
                    panic!("a second process acquired the same database")
                }
                Err(error) => error,
            };
            assert!(
                error.to_string().contains("database ownership unavailable"),
                "{error:#}"
            );
        }
        "acquire" => {
            let owner = Backend::prepare(config, options).unwrap();
            tokio::runtime::Runtime::new()
                .unwrap()
                .block_on(owner.shutdown())
                .unwrap();
        }
        "drop" => {
            let owner = Backend::prepare(config, options).unwrap();
            drop(owner);
            std::fs::write(std::env::var_os("WKS_BACKEND_OWNER_READY").unwrap(), "held").unwrap();
            loop {
                std::thread::sleep(std::time::Duration::from_millis(100));
            }
        }
        _ => panic!("unknown lease fixture case"),
    }
}

#[tokio::test]
async fn persistent_database_has_one_cross_process_owner_and_uncertain_drop_keeps_lease() {
    use std::{
        process::{Command, Stdio},
        time::Duration,
    };
    let _guard = ENGINE_LOCK.lock().await;
    let root = tempfile::tempdir().unwrap();
    let database = root.path().join("state.db");
    let config = || ServeConfig {
        host: "127.0.0.1".into(),
        hook_port: 0,
        api_port: 0,
        db_path: database.clone(),
    };
    let child = |case: &str| {
        let mut command = Command::new(std::env::current_exe().unwrap());
        command
            .args(["--exact", "backend_database_lease_child", "--nocapture"])
            .env("WKS_BACKEND_OWNER_DB", &database)
            .env("WKS_BACKEND_OWNER_CASE", case);
        command
    };
    let owner = Backend::prepare(
        config(),
        EngineOptions {
            usage_poll_on_boot: Some(false),
        },
    )
    .unwrap();
    let refused = child("refuse").output().unwrap();
    assert!(
        refused.status.success(),
        "{}",
        String::from_utf8_lossy(&refused.stderr)
    );
    owner.shutdown().await.unwrap();
    assert!(
        root.path().join("state.db.rust-owner.lock").exists(),
        "dropping an owner must not unlink the shared lock inode"
    );
    let acquired = child("acquire").output().unwrap();
    assert!(
        acquired.status.success(),
        "{}",
        String::from_utf8_lossy(&acquired.stderr)
    );
    struct Guard(std::process::Child);
    impl Drop for Guard {
        fn drop(&mut self) {
            let _ = self.0.kill();
            let _ = self.0.wait();
        }
    }
    let ready = root.path().join("dropped-owner");
    let mut held = Guard(
        child("drop")
            .env("WKS_BACKEND_OWNER_READY", &ready)
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .unwrap(),
    );
    tokio::time::timeout(Duration::from_secs(5), async {
        while !ready.exists() {
            assert!(
                held.0.try_wait().unwrap().is_none(),
                "lease holder exited before readiness"
            );
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .unwrap();
    let refused = child("refuse").output().unwrap();
    assert!(
        refused.status.success(),
        "unconfirmed Drop released its lock: {}",
        String::from_utf8_lossy(&refused.stderr)
    );
    held.0.kill().unwrap();
    held.0.wait().unwrap();
    let acquired = child("acquire").output().unwrap();
    assert!(
        acquired.status.success(),
        "crashed owner lock was not released: {}",
        String::from_utf8_lossy(&acquired.stderr)
    );
}
