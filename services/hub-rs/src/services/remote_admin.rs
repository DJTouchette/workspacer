//! Owner-controlled network sharing and ordinary remote-device pairing.
use crate::{
    Caller, Options,
    auth::{self, Record, Scope},
};
use anyhow::{Context, Result, anyhow, bail};
use futures_util::future::BoxFuture;
use serde_json::{Value, json};
use std::{path::PathBuf, sync::Arc, time::Duration};
fn trusted(caller: &Caller) -> bool {
    caller.authenticated_host && caller.trusted && caller.scope == "operator"
}
fn guard(caller: &Caller, method: &str, pairing: bool) -> Result<()> {
    if !trusted(caller) {
        bail!(
            "{method} requires the server owner{}",
            if pairing { "'s pairing token" } else { "" }
        );
    }
    Ok(())
}
fn empty_list(record: &Record, key: &str) -> bool {
    record
        .metadata
        .get(key)
        .is_none_or(|v| v.is_null() || v.as_array().is_some_and(Vec::is_empty))
}
pub fn is_pairing(record: &Record) -> bool {
    matches!(
        record.scope(),
        Some(Scope::View | Scope::Triage | Scope::Operator)
    ) && record.label.starts_with("Remote Control: ")
        && !record.facade_authority
        && record.provides.as_ref().is_none_or(Vec::is_empty)
        && record
            .metadata
            .get("role")
            .is_none_or(|v| v.is_null() || v == "")
        && record
            .metadata
            .get("yoloAllowed")
            .is_none_or(|v| v.is_null() || v == false)
        && empty_list(record, "profilesAllowed")
        && empty_list(record, "plugins")
}
pub struct Pairings {
    path: Option<PathBuf>,
}
impl Pairings {
    pub fn new(path: Option<PathBuf>) -> Self {
        Self { path }
    }
    pub fn call(&self, caller: &Caller, method: &str, params: &Value) -> Result<Value> {
        if method == "remote.pairingInfo" {
            return Ok(
                json!({"scope":caller.scope,"canManageTokens":self.path.is_some()&&trusted(caller)}),
            );
        }
        guard(caller, method, true)?;
        let path = self
            .path
            .as_ref()
            .context("pairing token store is disabled")?;
        match method {
            "remote.tokensList" => Ok(serde_json::to_value(
                auth::load(path)?
                    .into_iter()
                    .filter(is_pairing)
                    .collect::<Vec<_>>(),
            )?),
            "remote.tokenGetOrCreate" => {
                let scope = match params["scope"].as_str() {
                    Some("view") => Scope::View,
                    Some("triage") => Scope::Triage,
                    Some("operator") => Scope::Operator,
                    _ => bail!("pairing scope must be view, triage or operator"),
                };
                let label = format!("Remote Control: {}", scope.name());
                auth::update_records(path, |records| {
                    if let Some(record) = records
                        .iter()
                        .find(|r| r.scope() == Some(scope) && r.label == label && is_pairing(r))
                    {
                        return Ok(serde_json::to_value(record)?);
                    }
                    let record = auth::new_record(scope, &label)?;
                    let result = serde_json::to_value(&record)?;
                    records.push(record);
                    Ok(result)
                })
            }
            "remote.tokenRevoke" => {
                let token = params["token"]
                    .as_str()
                    .ok_or_else(|| anyhow!("token must be text"))?;
                auth::update_records(path, |records| {
                    let index=records.iter().position(|r|r.token==token&&is_pairing(r)).context("pairing token not found; infrastructure credentials cannot be revoked here")?;
                    Ok(serde_json::to_value(records.remove(index))?)
                })
            }
            _ => bail!("unknown remote pairing method"),
        }
    }
}
trait Command: Send + Sync {
    fn run<'a>(&'a self, args: Vec<String>, timeout: Duration) -> BoxFuture<'a, Result<Vec<u8>>>;
}
#[derive(serde::Deserialize, Default)]
struct TailscaleSelf {
    #[serde(rename = "DNSName", default)]
    dns_name: Option<String>,
}
#[derive(serde::Deserialize)]
struct TailscaleStatus {
    #[serde(rename = "BackendState", default)]
    state: Option<String>,
    #[serde(rename = "Self", default)]
    self_info: Option<TailscaleSelf>,
}
struct NativeCommand;
impl Command for NativeCommand {
    fn run<'a>(&'a self, args: Vec<String>, timeout: Duration) -> BoxFuture<'a, Result<Vec<u8>>> {
        Box::pin(async move {
            let mut command = tokio::process::Command::new("tailscale");
            command.args(args);
            let output = super::owned_process::capture(&mut command, 1024 * 1024, timeout).await?;
            if !output.status.success() {
                bail!(
                    "Tailscale command failed: {}",
                    String::from_utf8_lossy(&output.stderr).trim()
                );
            }
            Ok(output.stdout)
        })
    }
}
/// Names the owner trusted by enabling Tailscale Serve. Launcher-configured
/// names are never removed; toggled names persist in `file` when provided.
struct Proxy {
    hosts: crate::server::policy::TrustedHosts,
    configured: Vec<String>,
    file: Option<PathBuf>,
}
impl Proxy {
    fn saved(&self) -> Vec<String> {
        let Some(file) = &self.file else {
            return Vec::new();
        };
        std::fs::read_to_string(file)
            .unwrap_or_default()
            .lines()
            .map(str::trim)
            .filter(|line| !line.is_empty())
            .map(str::to_owned)
            .collect()
    }
    fn load(&self) {
        for name in self.saved() {
            if let Err(error) = self.hosts.insert(&name) {
                eprintln!("ignoring saved trusted host {name:?}: {error:#}");
            }
        }
    }
    fn save(&self, names: &[String]) -> Result<()> {
        let Some(file) = &self.file else {
            return Ok(());
        };
        if names.is_empty() {
            return match std::fs::remove_file(file) {
                Err(error) if error.kind() != std::io::ErrorKind::NotFound => Err(error.into()),
                _ => Ok(()),
            };
        }
        let mut text = names.join("\n");
        text.push('\n');
        super::config::atomic_bytes(file, text.as_bytes())
    }
    /// Serve terminates TLS for `name` and forwards to loopback, which the
    /// Host/Origin pins otherwise refuse as DNS rebinding.
    fn trust(&self, name: &str) -> Result<()> {
        self.hosts.insert(name)?;
        let mut names = self.saved();
        if !names.iter().any(|saved| saved.eq_ignore_ascii_case(name)) {
            names.push(name.to_owned());
        }
        self.save(&names)
    }
    /// `tailscale serve reset` clears every handler, so every toggled name goes.
    fn forget(&self, current: Option<&str>) -> Result<()> {
        for name in self.saved().iter().map(String::as_str).chain(current) {
            if !self
                .configured
                .iter()
                .any(|configured| configured.eq_ignore_ascii_case(name))
            {
                self.hosts.remove(name);
            }
        }
        self.save(&[])
    }
}
/// Turn Serve's stderr into the one-time fix. Only the classification and a
/// Tailscale login link are returned, never the raw command output.
fn serve_failure(stderr: &str) -> String {
    let lower = stderr.to_ascii_lowercase();
    if lower.contains("operator") || lower.contains("access denied") || lower.contains("permission")
    {
        return "Tailscale Serve needs permission. Run once on this machine: sudo tailscale set --operator=$USER".into();
    }
    // The opt-in message embeds an https:// link, so match it before the
    // certificate branch below.
    if lower.contains("not enabled") {
        let link = stderr
            .split_whitespace()
            .find(|word| word.starts_with("https://login.tailscale.com/"))
            .map(|word| word.trim_end_matches(['.', ',', ')', '"', '\'']));
        return match link {
            Some(link) => format!("Enable Tailscale Serve for your tailnet once: {link}"),
            None => "Enable HTTPS certificates and Serve for your tailnet in the Tailscale admin console.".into(),
        };
    }
    if lower.contains("https") || lower.contains("cert") {
        return "Enable HTTPS certificates for your tailnet in the Tailscale admin console (DNS → HTTPS Certificates).".into();
    }
    "Tailscale Serve failed; check the server's Tailscale permissions and HTTPS configuration"
        .into()
}
struct Network {
    proxy: Option<Proxy>,
    socket: Option<PathBuf>,
    token_file: Option<PathBuf>,
    port: Option<u16>,
    command: Arc<dyn Command>,
    linux: bool,
    identity: Option<(bool, String, String)>,
}
fn user_identity() -> (bool, String, String) {
    #[cfg(unix)]
    {
        let uid = unsafe { libc::geteuid() };
        let mut user = String::new();
        let mut buffer = vec![0u8; 16384];
        let mut entry = std::mem::MaybeUninit::<libc::passwd>::zeroed();
        let mut result = std::ptr::null_mut();
        if unsafe {
            libc::getpwuid_r(
                uid,
                entry.as_mut_ptr(),
                buffer.as_mut_ptr().cast(),
                buffer.len(),
                &mut result,
            )
        } == 0
            && !result.is_null()
        {
            let entry = unsafe { entry.assume_init() };
            if !entry.pw_name.is_null() {
                user = unsafe { std::ffi::CStr::from_ptr(entry.pw_name) }
                    .to_string_lossy()
                    .into();
            }
        }
        return (uid == 0, user, uid.to_string());
    }
    #[cfg(not(unix))]
    {
        (false, String::new(), String::new())
    }
}
impl Network {
    async fn broker(
        &self,
        method: reqwest::Method,
        path: &str,
        body: Option<Value>,
    ) -> Result<Value> {
        #[cfg(unix)]
        {
            let socket = self
                .socket
                .as_ref()
                .context("server network control unavailable")?;
            let token_file = self
                .token_file
                .as_ref()
                .context("private network credential unavailable")?;
            let token = std::fs::canonicalize(token_file)
                .ok()
                .and_then(|path| super::files::bounded_bytes(&path, 4096).ok())
                .and_then(|v| String::from_utf8(v).ok())
                .filter(|v| !v.trim().is_empty())
                .context("private network credential unavailable")?;
            let client = reqwest::Client::builder()
                .unix_socket(socket.as_path())
                .redirect(reqwest::redirect::Policy::none())
                .timeout(Duration::from_secs(40))
                .build()?;
            let mut request = client
                .request(method, format!("http://network-admin{path}"))
                .bearer_auth(token.trim());
            if let Some(body) = body {
                request = request.json(&body);
            }
            let mut response = request
                .send()
                .await
                .map_err(|_| anyhow!("server network control unavailable"))?;
            if response.status() != reqwest::StatusCode::OK {
                bail!(
                    "server network change was not confirmed (HTTP {})",
                    response.status().as_u16()
                );
            }
            let mut bytes = Vec::new();
            while let Some(chunk) = response
                .chunk()
                .await
                .map_err(|_| anyhow!("server network response unavailable"))?
            {
                if bytes.len() + chunk.len() > 32 * 1024 {
                    bail!("server network response exceeded size limit");
                }
                bytes.extend_from_slice(&chunk);
            }
            Ok(serde_json::from_slice(&bytes)?)
        }
        #[cfg(not(unix))]
        {
            let _ = (method, path, body);
            bail!(
                "server network control unavailable: Unix broker transport is unsupported on this platform"
            )
        }
    }
    async fn command(&self, args: &[&str], timeout: Duration) -> Result<Vec<u8>> {
        self.command
            .run(args.iter().map(|s| (*s).into()).collect(), timeout)
            .await
    }
    async fn info(&self) -> Result<Value> {
        if self.socket.is_some() {
            let value = self.broker(reqwest::Method::GET, "/status", None).await?;
            if !value.is_object() {
                bail!("invalid server network information");
            }
            return Ok(value);
        }
        let Ok(bytes) = self
            .command(&["status", "--json"], Duration::from_secs(10))
            .await
        else {
            return Ok(
                json!({"available":false,"magicName":null,"serveActive":false,"canServe":false}),
            );
        };
        let status: TailscaleStatus = serde_json::from_slice(&bytes)
            .map_err(|_| anyhow!("invalid Tailscale status response"))?;
        let available = status.state.as_deref() == Some("Running");
        let magic_name = status
            .self_info
            .and_then(|info| info.dns_name)
            .unwrap_or_default();
        let serving = self
            .command(&["serve", "status", "--json"], Duration::from_secs(10))
            .await
            .unwrap_or_default();
        let serving = String::from_utf8_lossy(&serving);
        let active = self.port.is_some_and(|port| {
            serving.contains(&format!("127.0.0.1:{port}"))
                || serving.contains(&format!("localhost:{port}"))
        });
        let (root, user_name, uid) = if self.linux {
            match &self.identity {
                Some(identity) => identity.clone(),
                None => tokio::task::spawn_blocking(user_identity).await?,
            }
        } else {
            (false, String::new(), String::new())
        };
        let mut permitted = !self.linux || root;
        if !permitted {
            let bytes = self
                .command(&["debug", "prefs"], Duration::from_secs(10))
                .await
                .unwrap_or_default();
            let prefs = serde_json::from_slice::<Value>(&bytes).unwrap_or(Value::Null);
            permitted = prefs["OperatorUser"].as_str().is_some_and(|user| {
                (!user_name.is_empty() && user == user_name) || (!uid.is_empty() && user == uid)
            });
        }
        let mut result = json!({"available":available,"magicName":magic_name.strip_suffix('.').unwrap_or(&magic_name),"serveActive":active,"canServe":available&&permitted&&self.port.is_some()});
        if !permitted {
            result["hint"] =
                "Configure the server's Tailscale operator to allow HTTPS sharing changes".into();
        }
        Ok(result)
    }
    async fn set_serve(&self, enabled: bool) -> Result<Value> {
        if self.socket.is_some() {
            return self
                .broker(
                    reqwest::Method::POST,
                    "/serve",
                    Some(json!({"enabled":enabled})),
                )
                .await;
        }
        let port = self
            .port
            .context("this server has no network listener to share")?;
        let args = if enabled {
            vec!["serve".into(), "--bg".into(), port.to_string()]
        } else {
            vec!["serve".into(), "reset".into()]
        };
        self.command
            .run(args, Duration::from_secs(120))
            .await
            .map_err(|error| anyhow!(serve_failure(&error.to_string())))?;
        if let Some(proxy) = &self.proxy {
            let name = self.info().await.ok().and_then(|info| {
                info["magicName"]
                    .as_str()
                    .filter(|name| !name.is_empty())
                    .map(str::to_owned)
            });
            if enabled {
                let name = name.context(
                    "Tailscale Serve started, but this node's MagicDNS name is unknown, so the hub cannot trust it",
                )?;
                proxy.trust(&name)?;
            } else {
                proxy.forget(name.as_deref())?;
            }
        }
        Ok(json!({"ok":true}))
    }
    async fn call(&self, caller: &Caller, method: &str, params: &Value) -> Result<Value> {
        if method == "remote.sharingInfo" {
            let mut result =
                json!({"enabled":true,"canToggleSharing":self.socket.is_some()&&trusted(caller)});
            if self.socket.is_some() {
                match tokio::time::timeout(Duration::from_secs(10), self.info()).await {
                    Ok(Ok(info)) => result["enabled"] = info["serveActive"].clone(),
                    error => {
                        result["canToggleSharing"] = false.into();
                        result["error"] = match error {
                            Ok(Err(error)) => error.to_string(),
                            _ => "server network control unavailable".into(),
                        }
                        .into();
                    }
                }
            }
            return Ok(result);
        }
        guard(caller, method, false)?;
        if method == "remote.tailscaleInfo" {
            return tokio::time::timeout(Duration::from_secs(10), self.info())
                .await
                .context("server network information timed out")?;
        }
        if method == "remote.setSharing" && self.socket.is_none() {
            bail!("this server's network listener is managed by its launcher");
        }
        let enabled = params["enabled"]
            .as_bool()
            .context("enabled boolean required")?;
        let result = tokio::time::timeout(Duration::from_secs(120), self.set_serve(enabled))
            .await
            .context("server network change was not confirmed before timeout")??;
        if method == "remote.setSharing" {
            Ok(json!({"enabled":enabled,"canToggleSharing":true}))
        } else {
            Ok(result)
        }
    }
}
pub(crate) fn install(
    mut options: Options,
    port: Option<u16>,
    hosts: crate::server::policy::TrustedHosts,
) -> Options {
    let proxy = port.map(|_| Proxy {
        hosts,
        configured: options
            .trusted_hosts
            .iter()
            .flat_map(|raw| raw.split(','))
            .map(|name| name.trim().to_owned())
            .filter(|name| !name.is_empty())
            .collect(),
        file: options.trusted_hosts_file.clone(),
    });
    if let Some(proxy) = &proxy {
        proxy.load();
    }
    let network = Arc::new(Network {
        proxy,
        socket: options.network_admin_socket.clone(),
        token_file: options.network_admin_token_file.clone(),
        port,
        command: Arc::new(NativeCommand),
        linux: cfg!(target_os = "linux"),
        identity: None,
    });
    for method in [
        "remote.tailscaleInfo",
        "remote.tailscaleServe",
        "remote.sharingInfo",
        "remote.setSharing",
    ] {
        let network = network.clone();
        options = options.handler(method, move |caller, params| {
            let network = network.clone();
            async move { network.call(&caller, method, &params).await }
        });
    }
    let pairings = Arc::new(Pairings::new(options.scoped_tokens.clone()));
    for method in [
        "remote.pairingInfo",
        "remote.tokensList",
        "remote.tokenGetOrCreate",
        "remote.tokenRevoke",
    ] {
        let pairings = pairings.clone();
        options = options.handler(method, move |caller, params| {
            let pairings = pairings.clone();
            async move {
                tokio::task::spawn_blocking(move || pairings.call(&caller, method, &params)).await?
            }
        });
    }
    options
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{collections::VecDeque, sync::Mutex};
    fn owner() -> Caller {
        Caller {
            call_id: 0,
            activity_seq: 0,
            connection_id: 0,
            authenticated_host: true,
            trusted: true,
            scope: "operator".into(),
            plugin_id: String::new(),
            token_id: String::new(),
            federated: false,
        }
    }
    #[test]
    fn pairing_transactions_confine_grants_reuse_identity_and_preserve_infrastructure() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("tokens.json");
        let provider = auth::mint(&path, Scope::Provider, "node").unwrap();
        let service = Arc::new(Pairings::new(Some(path.clone())));
        let request = json!({"scope":"operator","role":"manager","facadeAuthority":true,"yoloAllowed":true,"provides":["*"],"profilesAllowed":["private"]});
        let threads: Vec<_> = (0..8)
            .map(|_| {
                let service = service.clone();
                let request = request.clone();
                std::thread::spawn(move || {
                    service
                        .call(&owner(), "remote.tokenGetOrCreate", &request)
                        .unwrap()
                })
            })
            .collect();
        let rows: Vec<_> = threads.into_iter().map(|t| t.join().unwrap()).collect();
        assert!(rows.iter().all(|r| r["token"] == rows[0]["token"]));
        let minted: Record = serde_json::from_value(rows[0].clone()).unwrap();
        assert!(is_pairing(&minted));
        assert!(minted.metadata.is_empty());
        assert_eq!(
            service
                .call(&owner(), "remote.tokensList", &Value::Null)
                .unwrap()
                .as_array()
                .unwrap()
                .len(),
            1
        );
        for scope in ["provider", " Operator ", "unknown"] {
            assert!(
                service
                    .call(&owner(), "remote.tokenGetOrCreate", &json!({"scope":scope}))
                    .is_err()
            );
        }
        for scope in ["view", "triage", "operator", "provider"] {
            let mut caller = owner();
            caller.authenticated_host = false;
            caller.scope = scope.into();
            for method in [
                "remote.tokensList",
                "remote.tokenGetOrCreate",
                "remote.tokenRevoke",
            ] {
                assert!(service.call(&caller, method, &request).is_err());
            }
            assert_eq!(
                service
                    .call(&caller, "remote.pairingInfo", &Value::Null)
                    .unwrap()["canManageTokens"],
                false
            );
        }
        assert!(
            service
                .call(
                    &owner(),
                    "remote.tokenRevoke",
                    &json!({"token":provider.token})
                )
                .is_err()
        );
        service
            .call(
                &owner(),
                "remote.tokenRevoke",
                &json!({"token":minted.token}),
            )
            .unwrap();
        let records = auth::load(&path).unwrap();
        assert_eq!(records.len(), 1);
        assert!(records[0] == provider);
        for field in ["role", "yoloAllowed", "profilesAllowed", "plugins"] {
            let mut forged = minted.clone();
            forged.metadata.insert(
                field.into(),
                json!({"malformed":"cannot become ordinary pairing"}),
            );
            assert!(!is_pairing(&forged));
        }
    }
    #[tokio::test]
    async fn actual_bus_owner_gate_and_pairing_revocation_are_live() {
        let dir = tempfile::tempdir().unwrap();
        let mut options = Options::default();
        options.scoped_tokens = Some(dir.path().join("tokens.json"));
        options.token = "owner".into();
        let hub = crate::Hub::start(install(options, None, Default::default())).unwrap();
        hub.ready().await.unwrap();
        let host = crate::client::Client::connect(&hub.handle()).await.unwrap();
        let record = host
            .call("remote.tokenGetOrCreate", json!({"scope":"operator"}))
            .await
            .unwrap();
        let scoped = crate::client::Client::from_connection(
            hub.handle()
                .connect_authenticated(record["token"].as_str().unwrap().into(), false)
                .await
                .unwrap(),
        );
        assert_eq!(
            scoped.call("remote.pairingInfo", json!({})).await.unwrap()["canManageTokens"],
            false
        );
        assert!(scoped.call("remote.tokensList", json!({})).await.is_err());
        assert!(
            scoped
                .call("remote.tailscaleInfo", json!({}))
                .await
                .is_err()
        );
        assert!(
            scoped
                .call("remote.tailscaleServe", json!({"enabled":true}))
                .await
                .is_err()
        );
        assert!(
            host.call("remote.setSharing", json!({"enabled":true}))
                .await
                .unwrap_err()
                .to_string()
                .contains("launcher")
        );
        host.call("remote.tokenRevoke", json!({"token":record["token"]}))
            .await
            .unwrap();
        assert!(
            hub.handle()
                .connect_authenticated(record["token"].as_str().unwrap().into(), false)
                .await
                .is_err()
        );
        tokio::time::timeout(Duration::from_secs(7), scoped.disconnected())
            .await
            .unwrap();
        assert!(scoped.call("remote.pairingInfo", json!({})).await.is_err());
        hub.shutdown().unwrap();
    }
    struct Fake {
        calls: Mutex<Vec<Vec<String>>>,
        replies: Mutex<VecDeque<Vec<u8>>>,
    }
    impl Command for Fake {
        fn run<'a>(&'a self, args: Vec<String>, _: Duration) -> BoxFuture<'a, Result<Vec<u8>>> {
            Box::pin(async move {
                self.calls.lock().unwrap().push(args);
                self.replies
                    .lock()
                    .unwrap()
                    .pop_front()
                    .context("fake response unavailable")
            })
        }
    }
    fn fake() -> (Network, Arc<Fake>) {
        let fake = Arc::new(Fake {
            calls: Mutex::new(vec![]),
            replies: Mutex::new(VecDeque::new()),
        });
        (
            Network {
                proxy: None,
                socket: None,
                token_file: None,
                port: Some(4567),
                command: fake.clone(),
                linux: true,
                identity: Some((false, "fixture".into(), "1234".into())),
            },
            fake,
        )
    }
    #[tokio::test]
    async fn local_tailscale_commands_use_fixed_arguments_and_host_operator_identity() {
        let (service, fake) = fake();
        fake.replies.lock().unwrap().extend([
            br#"{"BackendState":"Running","Self":{"DNSName":"fixture.tailnet.ts.net."}}"#.to_vec(),
            br#"{"proxy":"http://127.0.0.1:4567"}"#.to_vec(),
            br#"{"OperatorUser":"1234"}"#.to_vec(),
            vec![],
            vec![],
        ]);
        let info = service
            .call(&owner(), "remote.tailscaleInfo", &Value::Null)
            .await
            .unwrap();
        assert_eq!(
            info,
            json!({"available":true,"magicName":"fixture.tailnet.ts.net","serveActive":true,"canServe":true})
        );
        service
            .call(
                &owner(),
                "remote.tailscaleServe",
                &json!({"enabled":true,"port":"; hostile"}),
            )
            .await
            .unwrap();
        service
            .call(&owner(), "remote.tailscaleServe", &json!({"enabled":false}))
            .await
            .unwrap();
        assert_eq!(
            *fake.calls.lock().unwrap(),
            vec![
                vec!["status", "--json"],
                vec!["serve", "status", "--json"],
                vec!["debug", "prefs"],
                vec!["serve", "--bg", "4567"],
                vec!["serve", "reset"]
            ]
        );
        assert!(
            service
                .call(
                    &owner(),
                    "remote.tailscaleServe",
                    &json!({"enabled":"true"})
                )
                .await
                .is_err()
        );
        assert_eq!(service.info().await.unwrap()["available"], false);
    }
    fn tailscale_info_replies(fake: &Fake) {
        fake.replies.lock().unwrap().extend([
            br#"{"BackendState":"Running","Self":{"DNSName":"Fixture.tailnet.ts.net."}}"#.to_vec(),
            br#"{}"#.to_vec(),
            br#"{"OperatorUser":"1234"}"#.to_vec(),
        ]);
    }
    #[tokio::test]
    async fn serving_trusts_the_magic_name_live_and_reset_forgets_only_toggled_names() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("hub-trusted-hosts");
        let hosts = crate::server::policy::TrustedHosts::parse(&["proxy.example".into()]).unwrap();
        let proxy = || Proxy {
            hosts: hosts.clone(),
            configured: vec!["proxy.example".into()],
            file: Some(file.clone()),
        };
        let (mut service, fake) = fake();
        service.proxy = Some(proxy());
        fake.replies.lock().unwrap().push_back(vec![]);
        tailscale_info_replies(&fake);
        service
            .call(&owner(), "remote.tailscaleServe", &json!({"enabled":true}))
            .await
            .unwrap();
        assert!(hosts.contains("fixture.tailnet.ts.net"));
        assert_eq!(
            std::fs::read_to_string(&file).unwrap(),
            "Fixture.tailnet.ts.net\n"
        );

        // A restarted hub reloads the toggled name before serving a request.
        let restarted = crate::server::policy::TrustedHosts::default();
        Proxy {
            hosts: restarted.clone(),
            configured: vec![],
            file: Some(file.clone()),
        }
        .load();
        assert!(restarted.contains("fixture.tailnet.ts.net"));

        fake.replies.lock().unwrap().push_back(vec![]);
        tailscale_info_replies(&fake);
        service
            .call(&owner(), "remote.tailscaleServe", &json!({"enabled":false}))
            .await
            .unwrap();
        assert!(!hosts.contains("fixture.tailnet.ts.net"));
        assert!(hosts.contains("proxy.example"));
        assert!(!file.exists());
    }
    #[test]
    fn serve_failures_name_the_fix_without_echoing_output() {
        assert!(
            serve_failure("Tailscale command failed: Access denied: serve config denied")
                .contains("--operator")
        );
        assert_eq!(
            serve_failure(
                "Tailscale command failed: Serve is not enabled on your tailnet.\nTo enable, visit:\n\n         https://login.tailscale.com/f/serve?node=abc123\n"
            ),
            "Enable Tailscale Serve for your tailnet once: https://login.tailscale.com/f/serve?node=abc123"
        );
        assert!(
            serve_failure("Tailscale command failed: HTTPS cert unavailable")
                .contains("HTTPS Certificates")
        );
        let generic = serve_failure("Tailscale command failed: secret-ish /home/user detail");
        assert!(!generic.contains("secret-ish"));
    }
    #[tokio::test]
    async fn failed_serve_trusts_nothing() {
        let dir = tempfile::tempdir().unwrap();
        let hosts = crate::server::policy::TrustedHosts::default();
        let (mut service, _fake) = fake();
        service.proxy = Some(Proxy {
            hosts: hosts.clone(),
            configured: vec![],
            file: Some(dir.path().join("hub-trusted-hosts")),
        });
        assert!(
            service
                .call(&owner(), "remote.tailscaleServe", &json!({"enabled":true}))
                .await
                .is_err()
        );
        assert!(!hosts.contains("fixture.tailnet.ts.net"));
        assert!(!dir.path().join("hub-trusted-hosts").exists());
    }
    #[cfg(unix)]
    #[tokio::test]
    async fn unix_broker_authenticates_private_fixed_paths_and_denied_callers_never_connect() {
        use tokio::io::{AsyncReadExt, AsyncWriteExt};
        let dir = tempfile::tempdir().unwrap();
        let socket = dir.path().join("network.sock");
        let token = dir.path().join("network-token");
        std::fs::write(&token, "private-network-fixture\n").unwrap();
        let listener = tokio::net::UnixListener::bind(&socket).unwrap();
        let (mut service, commands) = fake();
        service.socket = Some(socket);
        service.token_file = Some(token);
        let server = tokio::spawn(async move {
            for expected in ["GET /status", "POST /serve"] {
                let (mut stream, _) = listener.accept().await.unwrap();
                let mut bytes = Vec::new();
                loop {
                    let mut buf = [0; 1024];
                    let n = stream.read(&mut buf).await.unwrap();
                    assert!(n > 0);
                    bytes.extend_from_slice(&buf[..n]);
                    if let Some(end) = bytes.windows(4).position(|w| w == b"\r\n\r\n") {
                        let header = String::from_utf8_lossy(&bytes[..end]).to_ascii_lowercase();
                        let len = header
                            .lines()
                            .find_map(|l| {
                                l.strip_prefix("content-length:")
                                    .and_then(|s| s.trim().parse::<usize>().ok())
                            })
                            .unwrap_or(0);
                        if bytes.len() >= end + 4 + len {
                            break;
                        }
                    }
                }
                let request = String::from_utf8(bytes).unwrap();
                assert!(request.starts_with(expected));
                assert!(
                    request
                        .to_ascii_lowercase()
                        .contains("authorization: bearer private-network-fixture")
                );
                if expected.starts_with("POST") {
                    assert_eq!(
                        serde_json::from_str::<Value>(request.split_once("\r\n\r\n").unwrap().1)
                            .unwrap(),
                        json!({"enabled":false})
                    );
                }
                let body = if expected.starts_with("GET") {
                    r#"{"available":true,"magicName":"fixture","serveActive":true,"canServe":true}"#
                } else {
                    r#"{"ok":true}"#
                };
                stream.write_all(format!("HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",body.len()).as_bytes()).await.unwrap();
            }
            listener
        });
        let mut scoped = owner();
        scoped.authenticated_host = false;
        for method in [
            "remote.tailscaleInfo",
            "remote.tailscaleServe",
            "remote.setSharing",
        ] {
            assert!(
                service
                    .call(&scoped, method, &json!({"enabled":false}))
                    .await
                    .is_err()
            );
        }
        assert_eq!(
            service
                .call(&owner(), "remote.tailscaleInfo", &Value::Null)
                .await
                .unwrap()["available"],
            true
        );
        assert_eq!(
            service
                .call(&owner(), "remote.setSharing", &json!({"enabled":false}))
                .await
                .unwrap(),
            json!({"enabled":false,"canToggleSharing":true})
        );
        let listener = server.await.unwrap();
        assert!(commands.calls.lock().unwrap().is_empty());
        assert!(
            tokio::time::timeout(Duration::from_millis(50), listener.accept())
                .await
                .is_err()
        );
    }
}
