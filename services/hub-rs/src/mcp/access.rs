//! Explicit facade access policy. No absent-credential path creates a host or
//! persisted session token, and malformed/unreadable policy fails closed.
use crate::auth::{Identity, Kind, Record, Scope};
use anyhow::{Result, bail};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{
    net::IpAddr,
    path::PathBuf,
    sync::{Arc, Mutex},
};

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize, clap::ValueEnum)]
#[serde(rename_all = "lowercase")]
pub enum UntokenedAccess {
    #[default]
    Deny,
    View,
    Operator,
}
#[derive(Deserialize)]
struct ConfigPolicy {
    facade: Option<FacadePolicy>,
}
#[derive(Deserialize)]
struct FacadePolicy {
    #[serde(rename = "untokenedAccess")]
    mode: Option<UntokenedAccess>,
}
impl std::str::FromStr for UntokenedAccess {
    type Err = anyhow::Error;
    fn from_str(value: &str) -> Result<Self> {
        match value {
            "deny" => Ok(Self::Deny),
            "view" => Ok(Self::View),
            "operator" => Ok(Self::Operator),
            _ => bail!("invalid untokened access: want deny, view, or operator"),
        }
    }
}
pub(crate) struct Policy {
    config: Option<PathBuf>,
    override_mode: Option<UntokenedAccess>,
    static_token: String,
    loopback: bool,
    cached: Mutex<Option<([u8; 32], UntokenedAccess)>>,
}
#[derive(Clone)]
pub(crate) struct Lease {
    policy: Arc<Policy>,
    scope: Scope,
    guest: bool,
    facade: bool,
    plugin_method: Option<String>,
}
impl Policy {
    pub(crate) fn new(
        config: Option<PathBuf>,
        override_mode: Option<UntokenedAccess>,
        static_token: String,
        bind: IpAddr,
    ) -> Result<Arc<Self>> {
        let policy = Arc::new(Self {
            config,
            override_mode,
            static_token,
            loopback: bind.is_loopback(),
            cached: Mutex::new(None),
        });
        if policy.static_token.is_empty()
            && policy.mode() != UntokenedAccess::Deny
            && !policy.loopback
        {
            bail!(
                "credential-less MCP access requires a loopback listener; use untokened deny or configure a separate MCP static token"
            );
        }
        Ok(policy)
    }
    fn mode(&self) -> UntokenedAccess {
        if let Some(mode) = self.override_mode {
            return mode;
        }
        let Some(path) = &self.config else {
            return UntokenedAccess::Deny;
        };
        // Read the owner's current file rather than Config's last-known-good
        // projection: an unreadable or malformed lockdown cannot retain access.
        let Ok(bytes) = std::fs::read(path) else {
            return UntokenedAccess::Deny;
        };
        let hash: [u8; 32] = Sha256::digest(&bytes).into();
        let mut cached = self.cached.lock().unwrap();
        if let Some((previous, mode)) = cached.as_ref() {
            if *previous == hash {
                return *mode;
            }
        }
        let mode = serde_yaml::from_slice::<ConfigPolicy>(&bytes)
            .ok()
            .and_then(|value| value.facade.and_then(|facade| facade.mode))
            .unwrap_or_default();
        *cached = Some((hash, mode));
        mode
    }
    fn guest_scope(&self) -> Option<Scope> {
        if !self.static_token.is_empty() || !self.loopback {
            return None;
        }
        match self.mode() {
            UntokenedAccess::Deny => None,
            UntokenedAccess::View => Some(Scope::View),
            UntokenedAccess::Operator => Some(Scope::Operator),
        }
    }
    pub(super) fn guest(self: &Arc<Self>) -> Option<Lease> {
        Some(Lease {
            policy: self.clone(),
            scope: self.guest_scope()?,
            guest: true,
            facade: false,
            plugin_method: None,
        })
    }
    pub(super) fn static_credential(self: &Arc<Self>, token: &str) -> Option<Lease> {
        (!self.static_token.is_empty() && crate::auth::credential_eq(&self.static_token, token))
            .then(|| Lease {
                policy: self.clone(),
                scope: Scope::Operator,
                guest: false,
                facade: false,
                plugin_method: None,
            })
    }
}
impl Lease {
    pub(crate) fn valid(&self) -> bool {
        if self.guest {
            self.policy.guest_scope() == Some(self.scope)
        } else {
            !self.policy.static_token.is_empty()
        }
    }
    pub(super) fn scope(&self) -> Scope {
        self.scope
    }
    pub(super) fn is_guest(&self) -> bool {
        self.guest
    }
    pub(super) fn for_facade(&self) -> Self {
        let mut lease = self.clone();
        lease.facade = lease.scope == Scope::Operator;
        lease
    }
    pub(super) fn for_plugin(&self, method: &str) -> Self {
        let mut lease = self.clone();
        lease.plugin_method = Some(method.into());
        lease
    }
    pub(crate) fn allows_plugin(&self, method: &str) -> bool {
        self.valid()
            && !method.starts_with("desktop.")
            && method != "files.receiveUpload"
            && self.plugin_method.as_deref() == Some(method)
    }
    pub(crate) fn identity(&self) -> Result<Identity> {
        if !self.valid() {
            bail!("facade access policy changed");
        }
        Ok(Identity {
            kind: Kind::Scoped(Record {
                scope: self.scope.name().into(),
                facade_authority: self.facade,
                ..Default::default()
            }),
            token_id: if self.guest {
                String::new()
            } else {
                crate::auth::fingerprint(&self.policy.static_token)
            },
            federated: false,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[tokio::test]
    async fn worker_guest_and_static_facades_keep_tool_tiers_and_nonhost_upstream_proof() {
        use serde_json::{Value, json};
        use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
        let root = tempfile::tempdir().unwrap();
        let tokens = root.path().join("tokens.json");
        let record = crate::auth::mint(&tokens, Scope::Operator, "facade-service").unwrap();
        crate::auth::update_records(&tokens, |rows| {
            rows.iter_mut()
                .find(|r| r.token == record.token)
                .unwrap()
                .facade_authority = true;
            Ok(())
        })
        .unwrap();
        let writes = Arc::new(AtomicUsize::new(0));
        let count = writes.clone();
        let mut central_options=crate::Options::default().handler("app.getCwd",|caller,_|async move{Ok(json!({"scope":caller.scope,"host":caller.authenticated_host,"tokenId":caller.token_id}))})
            .handler("fs.write",move|caller,_|{let count=count.clone();async move{count.fetch_add(1,Ordering::SeqCst);Ok(json!({"host":caller.authenticated_host}))}});
        central_options.listen = Some("127.0.0.1:0".parse().unwrap());
        central_options.token = "central-owner".into();
        central_options.scoped_tokens = Some(tokens);
        let central = crate::Hub::start(central_options).unwrap();
        let central_address = central.ready().await.unwrap().unwrap();
        let upstream = crate::provider_relay::UpstreamCaller::new(
            format!("ws://{central_address}/bus"),
            record.token.clone(),
        );
        let caller_task = tokio::spawn(upstream.clone().run());
        tokio::time::timeout(std::time::Duration::from_secs(3), async {
            while !upstream.connected() {
                tokio::time::sleep(std::time::Duration::from_millis(10)).await;
            }
        })
        .await
        .unwrap();
        let local = crate::Hub::start(crate::Options::default()).unwrap();
        local.ready().await.unwrap();
        let http = reqwest::Client::new();
        for static_token in ["", "static-facade"] {
            let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
            let address = listener.local_addr().unwrap();
            let access = Policy::new(
                None,
                Some(UntokenedAccess::View),
                static_token.into(),
                address.ip(),
            )
            .unwrap();
            let running = tokio::spawn(super::super::serve(
                listener,
                local.handle(),
                "local-owner".into(),
                None,
                Arc::new(AtomicBool::new(false)),
                vec![],
                Some(upstream.clone()),
                access,
            ));
            let request = |tool: &str, args: Value| {
                let mut request = http
                    .post(format!("http://{address}/mcp"))
                    .header("accept", "application/json, text/event-stream")
                    .header("MCP-Protocol-Version", "2025-03-26");
                if !static_token.is_empty() {
                    request = request.bearer_auth(static_token);
                }
                request.json(&json!({"jsonrpc":"2.0","id":1,"method":"tools/call","params":{"name":tool,"arguments":args}}))
            };
            let read: Value = request("get_host_cwd", json!({}))
                .send()
                .await
                .unwrap()
                .json()
                .await
                .unwrap();
            let value: Value =
                serde_json::from_str(read["result"]["content"][0]["text"].as_str().unwrap())
                    .unwrap();
            assert_eq!(value["host"], false);
            assert_eq!(value["tokenId"], crate::auth::fingerprint(&record.token));
            let write: Value = request(
                "write_file",
                json!({"path":"/fixture","contents":"fixture"}),
            )
            .send()
            .await
            .unwrap()
            .json()
            .await
            .unwrap();
            if static_token.is_empty() {
                assert!(write.get("error").is_some());
                assert_eq!(writes.load(Ordering::SeqCst), 0);
            } else {
                let value: Value =
                    serde_json::from_str(write["result"]["content"][0]["text"].as_str().unwrap())
                        .unwrap();
                assert_eq!(value["host"], false);
                assert_eq!(writes.load(Ordering::SeqCst), 1);
            }
            running.abort();
            let _ = running.await;
        }
        upstream.close();
        caller_task.await.unwrap().unwrap();
        local.shutdown().unwrap();
        central.shutdown().unwrap();
    }
    #[tokio::test]
    async fn ephemeral_plugin_intent_cannot_become_general_or_host_administration() {
        let hub = crate::Hub::start(
            crate::Options::default()
                .handler("fixture.echo", |caller, _| async move {
                    Ok(serde_json::json!({"scope":caller.scope,"host":caller.authenticated_host}))
                })
                .handler("fixture.other", |_, _| async {
                    Ok(serde_json::Value::Null)
                })
                .handler("desktop.fixture", |_, _| async {
                    panic!("guest reached host administration")
                }),
        )
        .unwrap();
        hub.ready().await.unwrap();
        let policy = Policy::new(
            None,
            Some(UntokenedAccess::View),
            String::new(),
            "127.0.0.1".parse().unwrap(),
        )
        .unwrap();
        let lease = policy.guest().unwrap().for_plugin("fixture.echo");
        let client = crate::client::Client::from_connection(
            hub.handle().connect_ephemeral_facade(lease).await.unwrap(),
        );
        assert_eq!(
            client
                .call("fixture.echo", serde_json::Value::Null)
                .await
                .unwrap(),
            serde_json::json!({"scope":"view","host":false})
        );
        assert!(
            client
                .call("fixture.other", serde_json::Value::Null)
                .await
                .is_err()
        );
        assert!(
            client
                .call("desktop.fixture", serde_json::Value::Null)
                .await
                .is_err()
        );
        client.close();
        for credential in [
            policy.guest().unwrap().for_plugin("desktop.fixture"),
            Policy::new(
                None,
                Some(UntokenedAccess::Operator),
                "static".into(),
                "127.0.0.1".parse().unwrap(),
            )
            .unwrap()
            .static_credential("static")
            .unwrap()
            .for_facade(),
        ] {
            let client = crate::client::Client::from_connection(
                hub.handle()
                    .connect_ephemeral_facade(credential)
                    .await
                    .unwrap(),
            );
            assert!(
                client
                    .call("desktop.fixture", serde_json::Value::Null)
                    .await
                    .is_err()
            );
            client.close();
        }
        hub.shutdown().unwrap();
    }
    #[test]
    fn opt_in_is_live_fail_closed_nonhost_and_distinct_from_static_credentials() {
        let root = tempfile::tempdir().unwrap();
        let path = root.path().join("config.yaml");
        let policy = Policy::new(
            Some(path.clone()),
            None,
            String::new(),
            "127.0.0.1".parse().unwrap(),
        )
        .unwrap();
        assert!(policy.guest().is_none());
        std::fs::write(&path, "facade: { untokenedAccess: view }").unwrap();
        let view = policy.guest().unwrap();
        assert!(!view.identity().unwrap().authenticated_host());
        assert!(!view.identity().unwrap().may_assert_session());
        assert!(!view.identity().unwrap().may_call("fs.write"));
        std::fs::write(&path, "facade: { untokenedAccess: operator }").unwrap();
        assert!(!view.valid());
        let operator = policy.guest().unwrap();
        assert!(!operator.identity().unwrap().authenticated_host());
        assert!(!operator.identity().unwrap().may_call("desktop.tokens"));
        assert!(!operator.identity().unwrap().may_assert_session());
        std::fs::write(&path, "facade: [ broken").unwrap();
        assert!(!operator.valid());
        assert!(policy.guest().is_none());
        std::fs::write(
            &path,
            "facade: { untokenedAccess: deny, untokenedAccess: operator }",
        )
        .unwrap();
        assert!(policy.guest().is_none());
        assert!(
            Policy::new(
                None,
                Some(UntokenedAccess::View),
                String::new(),
                "0.0.0.0".parse().unwrap()
            )
            .is_err()
        );
        let private = Policy::new(
            None,
            Some(UntokenedAccess::Operator),
            "separate-mcp-key".into(),
            "0.0.0.0".parse().unwrap(),
        )
        .unwrap();
        assert!(private.guest().is_none());
        assert!(private.static_credential("wrong").is_none());
        let credential = private.static_credential("separate-mcp-key").unwrap();
        assert!(!credential.identity().unwrap().authenticated_host());
    }
}
