use claudemon::daemon::{
    embedded::{Command, EmbeddedDaemon, Options, Status},
    ServeConfig,
};
use std::time::Duration;

fn config() -> ServeConfig {
    ServeConfig {
        host: "127.0.0.1".into(),
        hook_port: 0,
        api_port: 0,
        db_path: claudemon::testtmp::db_path("embedded"),
    }
}

fn start(config: ServeConfig) -> anyhow::Result<EmbeddedDaemon> {
    EmbeddedDaemon::start_with_options(
        config,
        Options {
            usage_poll_on_boot: Some(false),
        },
    )
}

// Keep lifecycle checks together: only one daemon can own provider callbacks
// within a process. This integration binary isolates the callback slot from
// unit tests that set it while testing provider config serialization.
#[tokio::test]
async fn lifecycle_commands_failure_cleanup_and_restart() {
    tokio::time::timeout(Duration::from_secs(30), async {
        let mut daemon = start(config()).unwrap();
        assert!(start(config()).is_err());
        let ready = daemon.ready().await.unwrap();
        assert_ne!(ready.api_addr.port(), 0);
        assert_ne!(ready.hook_addr.port(), 0);
        assert_eq!(
            claudemon::daemon::API_BASE.get().unwrap(),
            format!("http://{}", ready.api_addr)
        );
        let client = daemon.client();
        assert!(matches!(*client.status().borrow(), Status::Ready(_)));
        assert!(client.request(Command::Sessions).await.unwrap().is_array());
        let conversation = client
            .request(Command::Conversation {
                id: "unknown".into(),
                since: None,
            })
            .await
            .unwrap();
        assert_eq!(conversation["items"], serde_json::json!([]));
        assert!(client
            .request(Command::Message {
                id: "unknown".into(),
                text: "hello".into()
            })
            .await
            .unwrap_err()
            .to_string()
            .contains("404"));
        assert!(client
            .request(Command::Conversation {
                id: "../escape".into(),
                since: None
            })
            .await
            .is_err());
        for addr in [ready.api_addr, ready.hook_addr] {
            assert!(reqwest::get(format!("http://{addr}/health"))
                .await
                .unwrap()
                .status()
                .is_success());
        }
        #[cfg(unix)]
        let child_pid = {
            // Exercise actual process ownership using a fake, token-free CLI.
            let response = reqwest::Client::new().post(format!("http://{}/sessions/spawn", ready.api_addr))
                .json(&serde_json::json!({"argv": ["/bin/sh", "-c", "echo $$; exec sleep 120"], "cwd": "/tmp"}))
                .send().await.unwrap();
            assert!(response.status().is_success());
            let spawned: serde_json::Value = response.json().await.unwrap();
            let id = spawned["session_id"].as_str().unwrap();
            loop {
                let output: serde_json::Value = reqwest::get(format!("http://{}/sessions/{id}/output", ready.api_addr))
                    .await.unwrap().json().await.unwrap();
                if let Some(text) = output["text"].as_str() {
                    if let Ok(pid) = text.trim().parse::<i32>() { break pid; }
                }
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
        };
        daemon.shutdown().await.unwrap();
        #[cfg(unix)]
        assert!(matches!(nix::sys::signal::kill(nix::unistd::Pid::from_raw(child_pid), None), Err(nix::errno::Errno::ESRCH)), "PTY child should be killed and reaped");
        assert!(client.request(Command::Sessions).await.is_err());
        assert!(matches!(*client.status().borrow(), Status::Stopped));
        assert!(claudemon::daemon::API_BASE.get().is_none());
        let _api_rebound = std::net::TcpListener::bind(ready.api_addr).unwrap();
        let _hook_rebound = std::net::TcpListener::bind(ready.hook_addr).unwrap();

        // Failure of the second listener must release the first and the lease.
        let reserved = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let vacant = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let hook_port = vacant.local_addr().unwrap().port();
        drop(vacant);
        let mut failed_cfg = config();
        failed_cfg.api_port = reserved.local_addr().unwrap().port();
        failed_cfg.hook_port = hook_port;
        let mut failed = start(failed_cfg).unwrap();
        assert!(failed.ready().await.is_err());
        assert!(failed.shutdown().await.is_err());
        let _released_hook = std::net::TcpListener::bind(("127.0.0.1", hook_port)).unwrap();
        let mut restarted = start(config()).unwrap();
        restarted.ready().await.unwrap();
        restarted.shutdown().await.unwrap();

        // A client that never finishes its spawn body must not hold the
        // shutdown admission guard forever. The runtime cancels that request
        // and releases the process lease even when graceful draining fails.
        let mut stalled = start(config()).unwrap();
        let ready = stalled.ready().await.unwrap();
        let mut stream = tokio::net::TcpStream::connect(ready.api_addr).await.unwrap();
        use tokio::io::AsyncWriteExt;
        stream.write_all(b"POST /sessions/spawn HTTP/1.1\r\nHost: localhost\r\nContent-Type: application/json\r\nContent-Length: 10000\r\n\r\n{").await.unwrap();
        tokio::time::sleep(Duration::from_millis(100)).await;
        let shutdown = tokio::time::timeout(Duration::from_secs(10), stalled.shutdown())
            .await.expect("incomplete request must not block shutdown");
        if let Err(error) = shutdown {
            assert!(error.to_string().contains("draining active spawn requests"));
        }
        drop(stream);
        let mut after_stall = start(config()).unwrap();
        after_stall.ready().await.unwrap();
        after_stall.shutdown().await.unwrap();
    })
    .await
    .expect("embedded lifecycle must complete without blocking host executor");
}
