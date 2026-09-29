#![cfg(unix)]
use base64::Engine;
use claudemon::daemon::{ServeConfig, embedded::Options as EngineOptions};
use serde_json::{Value, json};
use std::time::Duration;
use workspacer_hub::{Options, backend::Backend, client::Client, protocol::Event};
async fn bytes_until(
    events: &mut tokio::sync::broadcast::Receiver<Event>,
    session: &str,
    needle: &[u8],
) -> Vec<u8> {
    tokio::time::timeout(Duration::from_secs(10), async {
        let mut output = Vec::new();
        loop {
            let event = events.recv().await.unwrap();
            if event.topic != format!("pty.bytes.{session}") {
                continue;
            }
            output.extend(
                base64::engine::general_purpose::STANDARD
                    .decode(event.data.as_ref().and_then(Value::as_str).unwrap())
                    .unwrap(),
            );
            if output.windows(needle.len()).any(|w| w == needle) {
                return output;
            }
        }
    })
    .await
    .unwrap()
}
#[tokio::test]
async fn actual_shell_pty_replay_raw_input_resize_viewer_leases_and_owned_shutdown() {
    let dir = tempfile::tempdir().unwrap();
    let mut options = Options::default();
    options.home_dir = Some(dir.path().into());
    let backend = Backend::start(
        ServeConfig {
            host: "127.0.0.1".into(),
            hook_port: 0,
            api_port: 0,
            db_path: dir.path().join("sessions.db"),
        },
        EngineOptions {
            usage_poll_on_boot: Some(false),
        },
        options,
    )
    .await
    .unwrap();
    let first = Client::connect(&backend.handle()).await.unwrap();
    let second = Client::connect(&backend.handle()).await.unwrap();
    assert!(
        first
            .call(
                "terminals.create",
                json!({"shell":"/tmp/untrusted-program","cwd":dir.path()})
            )
            .await
            .is_err()
    );
    let mut first_events = first.events();
    first
        .topics(["facade.openTerminal".into()].into())
        .await
        .unwrap();
    first
        .call(
            "terminals.open",
            json!({"cwd":dir.path(),"command":"echo visible-only","label":"Visible request"}),
        )
        .await
        .unwrap();
    let visible = tokio::time::timeout(Duration::from_secs(5), first_events.recv())
        .await
        .unwrap()
        .unwrap();
    assert_eq!(visible.topic, "facade.openTerminal");
    assert_eq!(visible.data.unwrap()["command"], "echo visible-only");
    assert!(
        first
            .call("sessions.snapshots", json!({}))
            .await
            .unwrap()
            .as_array()
            .unwrap()
            .is_empty()
    );
    let spawned = first
        .call(
            "terminals.create",
            json!({"shell":"/bin/sh","cwd":dir.path(),"cols":80,"rows":24}),
        )
        .await
        .unwrap();
    let id = spawned["sessionId"].as_str().unwrap().to_owned();
    first
        .topics([format!("pty.bytes.{id}")].into())
        .await
        .unwrap();
    first
        .call("sessions.attachTerminal", json!({"sessionId":id}))
        .await
        .unwrap();
    first
        .call(
            "sessions.terminalInput",
            json!({"sessionId":id,"data":"printf 'pid-proof:%s\\n' \"$$\"\r"}),
        )
        .await
        .unwrap();
    let output = bytes_until(&mut first_events, &id, b"pid-proof:").await;
    // The echoed command itself contains the marker; wait for the numeric result.
    let mut all = output;
    let pid = tokio::time::timeout(Duration::from_secs(10), async {
        loop {
            let text = String::from_utf8_lossy(&all);
            if let Some(pid) = regex::Regex::new(r"pid-proof:([0-9]+)")
                .unwrap()
                .captures(&text)
                .and_then(|m| m[1].parse::<i32>().ok())
            {
                break pid;
            }
            let ev = first_events.recv().await.unwrap();
            if ev.topic == format!("pty.bytes.{id}") {
                all.extend(
                    base64::engine::general_purpose::STANDARD
                        .decode(ev.data.as_ref().unwrap().as_str().unwrap())
                        .unwrap(),
                );
            }
        }
    })
    .await
    .unwrap();
    first
        .call(
            "sessions.terminalResize",
            json!({"sessionId":id,"cols":90,"rows":40}),
        )
        .await
        .unwrap();
    first.call("sessions.terminalInput",json!({"sessionId":id,"bytesB64":base64::engine::general_purpose::STANDARD.encode(b"stty size\r")})).await.unwrap();
    bytes_until(&mut first_events, &id, b"40 90").await;
    let mut second_events = second.events();
    second
        .topics([format!("pty.bytes.{id}")].into())
        .await
        .unwrap();
    second
        .call("sessions.attachTerminal", json!({"sessionId":id}))
        .await
        .unwrap();
    let replay = bytes_until(&mut second_events, &id, b"pid-proof:").await;
    assert!(replay.starts_with(b"\x1bc"));
    first
        .call("sessions.detachTerminal", json!({"sessionId":id}))
        .await
        .unwrap();
    assert_eq!(
        first
            .call("sessions.terminalKeepalive", json!({"sessionId":id}))
            .await
            .unwrap()["ok"],
        false
    );
    assert_eq!(
        second
            .call("sessions.terminalKeepalive", json!({"sessionId":id}))
            .await
            .unwrap()["ok"],
        true
    );
    assert!(
        second
            .call(
                "sessions.terminalInput",
                json!({"sessionId":id,"bytesB64":"invalid!"})
            )
            .await
            .is_err()
    );
    second
        .call(
            "sessions.terminalInput",
            json!({"sessionId":id,"data":"printf '\\377raw-proof\\n'\r"}),
        )
        .await
        .unwrap();
    let output = bytes_until(&mut second_events, &id, b"\xffraw-proof").await;
    assert!(output.contains(&255));
    second
        .call("sessions.detachTerminal", json!({"sessionId":id}))
        .await
        .unwrap();
    assert_eq!(
        second
            .call("sessions.terminalKeepalive", json!({"sessionId":id}))
            .await
            .unwrap()["ok"],
        false
    );
    let natural = first
        .call(
            "terminals.create",
            json!({"shell":"/bin/sh","cwd":dir.path()}),
        )
        .await
        .unwrap();
    let natural_id = natural["sessionId"].as_str().unwrap();
    second
        .topics([format!("pty.bytes.{natural_id}"), "pty.exit".into()].into())
        .await
        .unwrap();
    second
        .call("sessions.attachTerminal", json!({"sessionId":natural_id}))
        .await
        .unwrap();
    second
        .call(
            "sessions.terminalInput",
            json!({"sessionId":natural_id,"data":"exit\r"}),
        )
        .await
        .unwrap();
    tokio::time::timeout(Duration::from_secs(10), async {
        loop {
            let event = second_events.recv().await.unwrap();
            if event.topic == "pty.exit"
                && event
                    .data
                    .as_ref()
                    .is_some_and(|v| v["sessionId"] == natural_id)
            {
                break;
            }
        }
    })
    .await
    .unwrap();
    tokio::time::timeout(Duration::from_secs(5), async {
        loop {
            let rows = first.call("sessions.snapshots", json!({})).await.unwrap();
            if rows
                .as_array()
                .unwrap()
                .iter()
                .all(|r| r["sessionId"] != natural_id)
            {
                break;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .unwrap();
    first.close();
    second.close();
    backend.shutdown().await.unwrap();
    assert_ne!(
        unsafe { libc::kill(pid, 0) },
        0,
        "owned shell child survived backend shutdown"
    );
}
