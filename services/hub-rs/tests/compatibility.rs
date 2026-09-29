use futures_util::{SinkExt, StreamExt};
use serde_json::{Value, json};
use std::{
    collections::HashMap,
    io::{BufRead, BufReader},
    process::{Child, Command, Stdio},
    time::Duration,
};
use tokio_tungstenite::{MaybeTlsStream, WebSocketStream, tungstenite::Message};
use workspacer_hub::{Connection, Handle, Hub, Options, protocol::Frame};

enum Client {
    Embedded(Connection),
    Socket(Box<WebSocketStream<MaybeTlsStream<tokio::net::TcpStream>>>),
}
impl Client {
    async fn send(&mut self, value: Value) {
        match self {
            Self::Embedded(c) => c.send(serde_json::from_value(value).unwrap()).unwrap(),
            Self::Socket(c) => c.send(Message::Text(value.to_string())).await.unwrap(),
        }
    }
    async fn recv(&mut self) -> Value {
        tokio::time::timeout(Duration::from_secs(3), async {
            match self {
                Self::Embedded(c) => {
                    serde_json::to_value(c.recv().await.expect("embedded connection closed"))
                        .unwrap()
                }
                Self::Socket(c) => loop {
                    match c.next().await.expect("socket closed").unwrap() {
                        Message::Text(t) => break serde_json::from_str(&t).unwrap(),
                        Message::Ping(p) => c.send(Message::Pong(p)).await.unwrap(),
                        other => panic!("unexpected socket message {other:?}"),
                    }
                },
            }
        })
        .await
        .expect("fixture response timed out")
    }
    async fn close(mut self) {
        if let Self::Socket(c) = &mut self {
            c.as_mut().close(None).await.unwrap();
        }
    }
}

enum Host {
    Embedded(Handle),
    Network(String),
}
impl Host {
    async fn connect(&self, token: Option<&str>, scope: &str) -> Client {
        let mut client = match self {
            Self::Embedded(handle) => Client::Embedded(match token {
                None => handle.connect().await.unwrap(),
                Some(token) => handle
                    .connect_authenticated(token.into(), false)
                    .await
                    .unwrap(),
            }),
            Self::Network(address) => Client::Socket(Box::new(
                tokio_tungstenite::connect_async(format!(
                    "ws://{address}/bus?token={}",
                    token.unwrap_or("migration-fixture-only")
                ))
                .await
                .unwrap()
                .0,
            )),
        };
        let hello = client.recv().await;
        if scope == "operator" {
            assert_eq!(
                hello,
                json!({"op":"hello", "scope":"operator", "methods":["*"], "spawnFullAccess":true})
            );
        } else {
            assert_eq!(hello["op"], "hello");
            assert_eq!(hello["scope"], scope);
            assert!(hello.get("spawnFullAccess").is_none());
        }
        client
    }
}

async fn replay(host: Host, case: &Value) {
    let mut clients = HashMap::<String, Client>::new();
    let mut ids = HashMap::<String, String>::new();
    for (index, step) in case["steps"].as_array().unwrap().iter().enumerate() {
        let name = case["name"].as_str().unwrap();
        if let Some(id) = step["connect"].as_str() {
            assert!(
                clients
                    .insert(
                        id.to_owned(),
                        host.connect(
                            step["token"].as_str(),
                            step["scope"].as_str().unwrap_or("operator")
                        )
                        .await
                    )
                    .is_none()
            );
        } else if let Some(id) = step["disconnect"].as_str() {
            clients.remove(id).unwrap().close().await;
        } else {
            let mut frame = step["frame"].clone();
            if let Some(key) = frame["id"].as_str().and_then(|s| s.strip_prefix('$'))
                && let Some(value) = ids.get(key)
            {
                frame["id"] = Value::String(value.clone());
            }
            if let Some(id) = step["send"].as_str() {
                clients.get_mut(id).unwrap().send(frame).await;
            } else if let Some(id) = step["expect"].as_str() {
                let actual = clients.get_mut(id).unwrap().recv().await;
                if let Some(key) = step["captureId"].as_str() {
                    let value = actual["id"].as_str().unwrap().to_owned();
                    assert!(!value.is_empty());
                    ids.insert(key.to_owned(), value.clone());
                    frame["id"] = Value::String(value);
                }
                assert_eq!(actual, frame, "{name}, step {index}");
            } else {
                panic!("invalid fixture step: {step}");
            }
        }
    }
    for (_, client) in clients {
        client.close().await;
    }
}

fn cases() -> Value {
    serde_json::from_str(include_str!("../../../contracts/hub-bus-cases.json")).unwrap()
}
fn options(network: bool, tokens: &std::path::Path) -> Options {
    let mut options = Options::default().handler("test.echo", |_, value| async move { Ok(value) });
    let records =
        ["view", "triage", "operator", "provider"].map(|scope| workspacer_hub::auth::Record {
            token: format!("migration-{scope}-only"),
            scope: scope.into(),
            provides: Some(vec!["*".into()]),
            ..Default::default()
        });
    workspacer_hub::auth::save(tokens, &records).unwrap();
    options.scoped_tokens = Some(tokens.into());
    if network {
        options.listen = Some("127.0.0.1:0".parse().unwrap());
        options.token = "migration-fixture-only".into();
    }
    options
}

#[tokio::test]
async fn shared_contracts_embedded() {
    for case in cases()["cases"].as_array().unwrap() {
        let dir = tempfile::tempdir().unwrap();
        let hub = Hub::start(options(false, &dir.path().join("tokens.json"))).unwrap();
        hub.ready().await.unwrap();
        replay(Host::Embedded(hub.handle()), case).await;
        hub.shutdown().unwrap();
    }
}

#[tokio::test]
async fn shared_contracts_rust_websocket() {
    for case in cases()["cases"].as_array().unwrap() {
        let dir = tempfile::tempdir().unwrap();
        let hub = Hub::start(options(true, &dir.path().join("tokens.json"))).unwrap();
        let address = hub.ready().await.unwrap().unwrap();
        replay(Host::Network(address.to_string()), case).await;
        hub.shutdown().unwrap();
    }
}

struct Reference(Child);
impl Reference {
    fn start() -> (Self, String) {
        let binary = std::env::var_os("WKS_GO_HUB_REFERENCE")
            .expect("set WKS_GO_HUB_REFERENCE to the built hub-reference binary");
        let mut child = Command::new(binary)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::inherit())
            .spawn()
            .unwrap();
        let mut line = String::new();
        BufReader::new(child.stdout.take().unwrap())
            .read_line(&mut line)
            .unwrap();
        let info: Value = serde_json::from_str(&line).unwrap();
        (Self(child), info["address"].as_str().unwrap().into())
    }
}
impl Drop for Reference {
    fn drop(&mut self) {
        drop(self.0.stdin.take());
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

#[tokio::test]
#[ignore = "requires Go reference executable; make test-hub-parity runs this explicitly"]
async fn shared_contracts_go_reference() {
    for case in cases()["cases"].as_array().unwrap() {
        let (_reference, address) = Reference::start();
        replay(Host::Network(address), case).await;
    }
}

#[test]
fn null_payloads_are_not_missing_fields() {
    for value in [
        json!({"op":"result","result":null}),
        json!({"op":"call","params":null}),
    ] {
        assert_eq!(
            serde_json::to_value(serde_json::from_value::<Frame>(value.clone()).unwrap()).unwrap(),
            value
        );
    }
}

#[test]
fn event_timestamps_follow_the_reference_time_contract() {
    let invalid = json!({"op":"publish","event":{"id":"one","type":"fixture","source":"fixture","time":"not-a-date"}});
    assert!(serde_json::from_value::<Frame>(invalid).is_err());
    let event = workspacer_hub::protocol::Event::new("fixture", "fixture", json!({}));
    let wire = serde_json::to_value(event).unwrap();
    assert_eq!(wire["time"], "0001-01-01T00:00:00Z");
    assert!(serde_json::from_value::<workspacer_hub::protocol::Event>(wire).is_ok());
}
