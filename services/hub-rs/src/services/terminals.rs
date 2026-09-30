//! Leased terminal byte forwarding over the bus. The embedded daemon owns every
//! shell/process; this service owns only subscriptions and visible-pane requests.
use crate::{Caller, Handle, Options, protocol::Event};
use anyhow::{Result, anyhow, bail};
use base64::Engine;
use claudemon::daemon::embedded::{Command, EmbeddedClient};
use serde_json::{Value, json};
use std::{
    collections::{BTreeMap, BTreeSet},
    path::PathBuf,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    time::{Duration, Instant},
};
use tokio::sync::{Mutex, broadcast, watch};
const LEASE: Duration = Duration::from_secs(20);
const FRAME: Duration = Duration::from_millis(16);
const FLUSH: usize = 64 * 1024;
struct Forwarder {
    leases: BTreeMap<u64, Instant>,
    stop: watch::Sender<bool>,
    task: tokio::task::JoinHandle<()>,
}
pub struct Terminals {
    engine: EmbeddedClient,
    hub: Handle,
    home: PathBuf,
    forwarders: Mutex<BTreeMap<String, Forwarder>>,
    closing: AtomicBool,
    shells: std::sync::Mutex<BTreeSet<String>>,
    uncertain_shells: std::sync::Mutex<BTreeSet<String>>,
    sweep_stop: watch::Sender<bool>,
    sweeper: Mutex<Option<tokio::task::JoinHandle<()>>>,
}
fn id(params: &Value) -> Result<&str> {
    let id = params["sessionId"].as_str().unwrap_or("");
    if id.is_empty()
        || matches!(id, "." | "..")
        || !id
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b"-_.".contains(&b))
    {
        bail!("a valid sessionId is required");
    }
    Ok(id)
}
fn typed<'a>(params: &'a Value, key: &str) -> Result<&'a str> {
    match params.get(key) {
        None | Some(Value::Null) => Ok(""),
        Some(v) => v.as_str().ok_or_else(|| anyhow!("{key} must be a string")),
    }
}
pub fn normalize_cwd(raw: &str, home: &std::path::Path) -> String {
    let mut raw = raw.trim_matches([' ', '\t', '\n', '\r', '\x0b', '\x0c']);
    while raw.len() > 1 && (raw.ends_with('/') || raw.ends_with('\\')) {
        raw = &raw[..raw.len() - 1];
    }
    if raw.is_empty() {
        home.to_string_lossy().into_owned()
    } else {
        raw.into()
    }
}
fn dimension(params: &Value, key: &str, default: Option<u16>) -> Result<u16> {
    match params.get(key) {
        None | Some(Value::Null) => default.ok_or_else(|| anyhow!("{key} is required")),
        Some(v) => {
            let n = v
                .as_u64()
                .ok_or_else(|| anyhow!("{key} must be a positive integer"))?;
            if n == 0 {
                if let Some(default) = default {
                    return Ok(default);
                }
            }
            if !(1..=65535).contains(&n) {
                bail!("{key} must be between 1 and 65535");
            }
            Ok(n as u16)
        }
    }
}
pub fn resolve_shell(requested: &str) -> Result<String> {
    let shell = std::env::var("SHELL").ok();
    let listed = if requested.trim().is_empty() {
        String::new()
    } else {
        std::fs::read_to_string("/etc/shells").unwrap_or_default()
    };
    resolve_shell_from(requested, shell.as_deref(), &listed)
}
fn resolve_shell_from(requested: &str, host_shell: Option<&str>, listed: &str) -> Result<String> {
    #[cfg(windows)]
    let defaults = ["powershell.exe", "pwsh.exe", "cmd.exe"].as_slice();
    #[cfg(not(windows))]
    let defaults = [
        "/bin/sh",
        "/bin/bash",
        "/bin/zsh",
        "/usr/bin/bash",
        "/usr/bin/zsh",
        "/bin/fish",
        "/usr/bin/fish",
    ]
    .as_slice();
    let default = host_shell.filter(|s| !s.is_empty()).unwrap_or(defaults[0]);
    if requested.trim().is_empty() {
        return Ok(default.into());
    }
    // The default retains its host spelling, but every allowlist entry follows
    // the same trimming/comment rules. Caller-supplied argv stays exact.
    let allowed: BTreeSet<String> = defaults
        .iter()
        .copied()
        .chain(host_shell)
        .chain(listed.split('\n'))
        .map(str::trim)
        .filter(|s| !s.is_empty() && !s.starts_with('#'))
        .map(str::to_owned)
        .collect();
    if allowed.contains(requested) {
        return Ok(requested.into());
    }
    #[cfg(windows)]
    {
        let basename = |s: &str| {
            std::path::Path::new(s)
                .file_name()
                .unwrap_or_default()
                .to_string_lossy()
                .to_ascii_lowercase()
        };
        if allowed.iter().any(|s| basename(s) == basename(requested)) {
            return Ok(requested.into());
        }
    }
    bail!("terminals.create: requested executable is not one of this host's login shells")
}
impl Terminals {
    pub fn new(engine: EmbeddedClient, hub: Handle, home: PathBuf) -> Arc<Self> {
        let (stop, mut stopping) = watch::channel(false);
        let service = Arc::new(Self {
            engine,
            hub,
            home,
            forwarders: Mutex::new(BTreeMap::new()),
            closing: AtomicBool::new(false),
            shells: std::sync::Mutex::new(BTreeSet::new()),
            uncertain_shells: std::sync::Mutex::new(BTreeSet::new()),
            sweep_stop: stop,
            sweeper: Mutex::new(None),
        });
        let weak = Arc::downgrade(&service);
        let task = tokio::spawn(async move {
            let mut tick = tokio::time::interval(Duration::from_secs(5));
            tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
            loop {
                tokio::select! {biased;_=stopping.changed()=>break,_=tick.tick()=>{let Some(service)=weak.upgrade()else{break};service.expire(Instant::now()).await;}}
            }
        });
        *service.sweeper.try_lock().unwrap() = Some(task);
        service
    }
    async fn stop_forwarder(mut entry: Forwarder) {
        entry.stop.send_replace(true);
        if tokio::time::timeout(Duration::from_secs(2), &mut entry.task)
            .await
            .is_err()
        {
            entry.task.abort();
            let _ = entry.task.await;
        }
    }
    async fn expire(&self, now: Instant) {
        let mut forwarders = self.forwarders.lock().await;
        Self::expire_entries(&mut forwarders, now).await;
    }
    async fn expire_entries(forwarders: &mut BTreeMap<String, Forwarder>, now: Instant) {
        let ids: Vec<_> = forwarders
            .iter_mut()
            .filter_map(|(id, row)| {
                row.leases.retain(|_, deadline| *deadline >= now);
                (row.leases.is_empty() || row.task.is_finished()).then(|| id.clone())
            })
            .collect();
        for id in ids {
            if let Some(row) = forwarders.remove(&id) {
                Self::stop_forwarder(row).await;
            }
        }
    }
    /// Host launch attribution only; intersect with authoritative engine liveness.
    pub fn owned_shell_ids(&self) -> BTreeSet<String> {
        self.shells.lock().unwrap().clone()
    }
    pub fn uncertain_shell_ids(&self) -> BTreeSet<String> {
        self.uncertain_shells.lock().unwrap().clone()
    }
    /// Only authoritative local engine observations settle unknown admissions.
    pub fn confirm_shell_observation(&self, id: &str) {
        self.uncertain_shells.lock().unwrap().remove(id);
    }
    pub async fn close(&self) {
        self.closing.store(true, Ordering::Release);
        self.sweep_stop.send_replace(true);
        if let Some(task) = self.sweeper.lock().await.take() {
            let _ = task.await;
        }
        let mut forwarders = self.forwarders.lock().await;
        let old = std::mem::take(&mut *forwarders);
        for (_, row) in old {
            Self::stop_forwarder(row).await;
        }
    }
    async fn attach(&self, session: &str, connection: u64) -> Result<()> {
        if self.closing.load(Ordering::Acquire) {
            bail!("terminal forwarding is closing");
        }
        let mut forwarders = self.forwarders.lock().await;
        if self.closing.load(Ordering::Acquire) {
            bail!("terminal forwarding is closing");
        }
        let (mut replay, rx) = self.engine.subscribe_terminal(session).await?;
        let mut leases = BTreeMap::new();
        if let Some(old) = forwarders.remove(session) {
            leases = old.leases.clone();
            Self::stop_forwarder(old).await;
            let mut reset = b"\x1bc".to_vec();
            reset.append(&mut replay);
            replay = reset;
        }
        leases.insert(connection, Instant::now() + LEASE);
        let (stop, stopping) = watch::channel(false);
        let engine = self.engine.clone();
        let hub = self.hub.clone();
        let session = session.to_owned();
        let task = tokio::spawn(forward(engine, hub, session.clone(), replay, rx, stopping));
        forwarders.insert(session, Forwarder { leases, stop, task });
        Ok(())
    }
    pub async fn call(&self, caller: Caller, method: &str, params: Value) -> Result<Value> {
        if self.closing.load(Ordering::Acquire) {
            bail!("terminal service is closing");
        }
        if !params.is_object() && !params.is_null() {
            bail!("terminal parameters must be an object or null");
        }
        match method {
            "terminals.open" => {
                let cwd = normalize_cwd(typed(&params, "cwd")?, &self.home);
                let payload = json!({"cwd":cwd,"command":typed(&params,"command")?,"label":typed(&params,"label")?,"parentSessionId":typed(&params,"parentSessionId")?});
                self.hub
                    .publish_wait(Event::new("facade.openTerminal", "brain", payload))
                    .await?;
                Ok(json!({"ok":true}))
            }
            "terminals.create" => {
                let shell = resolve_shell(typed(&params, "shell")?)?;
                let cwd = normalize_cwd(typed(&params, "cwd")?, &self.home);
                let cols = dimension(&params, "cols", Some(120))?;
                let rows = dimension(&params, "rows", Some(32))?;
                let session = uuid::Uuid::new_v4().to_string();
                // Attribute before engine admission: losing an acknowledgement
                // cannot make a running shell invisible to quiescence checks.
                self.shells.lock().unwrap().insert(session.clone());
                self.uncertain_shells
                    .lock()
                    .unwrap()
                    .insert(session.clone());
                let result=self.engine.request(Command::Request {
                    method:"POST".into(),path:"/sessions/spawn".into(),
                    payload:Some(json!({"session_id":session,"argv":[shell],"cwd":cwd,"cols":cols,"rows":rows})),
                }).await;
                let result = match result {
                    Ok(result) => result,
                    Err(error) => {
                        if error
                            .downcast_ref::<claudemon::daemon::embedded::CommandRejected>()
                            .is_some()
                        {
                            self.shells.lock().unwrap().remove(&session);
                            self.uncertain_shells.lock().unwrap().remove(&session);
                        }
                        return Err(error);
                    }
                };
                if result["session_id"] != session {
                    bail!(
                        "terminal spawn acknowledgement differs from pinned identity; outcome unknown, do not retry automatically"
                    );
                }
                self.uncertain_shells.lock().unwrap().remove(&session);
                Ok(json!({"sessionId":session}))
            }
            "sessions.attachTerminal" => {
                self.attach(id(&params)?, caller.connection_id).await?;
                Ok(json!({"ok":true}))
            }
            "sessions.terminalKeepalive" => {
                let mut map = self.forwarders.lock().await;
                let ok = map.get_mut(id(&params)?).is_some_and(|row| {
                    if row.task.is_finished() {
                        return false;
                    }
                    if let Some(deadline) = row.leases.get_mut(&caller.connection_id) {
                        *deadline = Instant::now() + LEASE;
                        true
                    } else {
                        false
                    }
                });
                Ok(json!({"ok":ok}))
            }
            "sessions.detachTerminal" => {
                let id = id(&params)?;
                let mut map = self.forwarders.lock().await;
                if let Some(row) = map.get_mut(id) {
                    row.leases.remove(&caller.connection_id);
                    if row.leases.is_empty() {
                        let row = map.remove(id).unwrap();
                        Self::stop_forwarder(row).await;
                    }
                }
                Ok(json!({"ok":true}))
            }
            "sessions.terminalInput" => {
                let id = id(&params)?;
                let bytes = typed(&params, "bytesB64")?;
                let data = typed(&params, "data")?;
                let payload = if !bytes.is_empty() {
                    base64::engine::general_purpose::STANDARD.decode(bytes)?;
                    json!({"bytes_b64":bytes,"newline":false})
                } else {
                    json!({"text":data,"newline":false})
                };
                self.engine
                    .request(Command::Request {
                        method: "POST".into(),
                        path: format!("/sessions/{id}/input"),
                        payload: Some(payload),
                    })
                    .await?;
                Ok(json!({"ok":true}))
            }
            "sessions.terminalResize" => {
                let id = id(&params)?;
                let cols = dimension(&params, "cols", None)?;
                let rows = dimension(&params, "rows", None)?;
                self.engine
                    .request(Command::Request {
                        method: "POST".into(),
                        path: format!("/sessions/{id}/resize"),
                        payload: Some(json!({"cols":cols,"rows":rows})),
                    })
                    .await?;
                Ok(json!({"ok":true}))
            }
            _ => bail!("unknown terminal method"),
        }
    }
}
async fn publish(
    hub: &Handle,
    session: &str,
    bytes: &[u8],
    stop: &mut watch::Receiver<bool>,
) -> Result<()> {
    for chunk in bytes.chunks(FLUSH) {
        if *stop.borrow() {
            bail!("terminal forwarding stopped");
        }
        let event = Event::new(
            format!("pty.bytes.{session}"),
            "brain",
            json!(base64::engine::general_purpose::STANDARD.encode(chunk)),
        );
        tokio::select! {biased;_=stop.changed()=>bail!("terminal forwarding stopped"),result=tokio::time::timeout(Duration::from_secs(2),hub.publish_wait(event))=>{result.map_err(|_|anyhow!("terminal event delivery timed out"))??;}}
    }
    Ok(())
}
async fn publish_exit_if_ended(
    engine: &EmbeddedClient,
    hub: &Handle,
    session: &str,
    stop: &mut watch::Receiver<bool>,
) -> Result<()> {
    let result = tokio::select! {biased;_=stop.changed()=>return Ok(()),result=engine.request(Command::Request{method:"GET".into(),path:format!("/sessions/{session}"),payload:None})=>result};
    let ended = match result {
        Ok(row) => row["mode"] == "stopped",
        Err(error) => error
            .downcast_ref::<claudemon::daemon::embedded::CommandRejected>()
            .is_some_and(|e| e.status == 404),
    };
    if ended {
        tokio::select! {biased;_=stop.changed()=>return Ok(()),result=hub.publish_wait(Event::new("pty.exit","brain",json!({"sessionId":session})))=>{result?;}}
    }
    Ok(())
}
async fn forward(
    engine: EmbeddedClient,
    hub: Handle,
    session: String,
    replay: Vec<u8>,
    mut rx: broadcast::Receiver<Vec<u8>>,
    mut stop: watch::Receiver<bool>,
) {
    let result: Result<()> = async {
        publish(&hub, &session, &replay, &mut stop).await?;
        let mut pending = Vec::new();
        let mut tick = tokio::time::interval(FRAME);
        tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
        loop {
            tokio::select! {
                biased;
                _ = stop.changed() => return Ok(()),
                chunk = rx.recv() => match chunk {
                    Ok(chunk) => {
                        pending.extend_from_slice(&chunk);
                        if pending.len() >= FLUSH {
                            publish(&hub, &session, &pending, &mut stop).await?;
                            pending.clear();
                        }
                    }
                    Err(broadcast::error::RecvError::Lagged(_)) => {
                        pending.clear();
                        let (snapshot, fresh) = engine.subscribe_terminal(&session).await?;
                        rx = fresh;
                        pending.extend_from_slice(b"\x1bc");
                        pending.extend(snapshot);
                        publish(&hub, &session, &pending, &mut stop).await?;
                        pending.clear();
                    }
                    Err(broadcast::error::RecvError::Closed) => {
                        publish(&hub, &session, &pending, &mut stop).await?;
                        publish_exit_if_ended(&engine, &hub, &session, &mut stop).await?;
                        return Ok(());
                    }
                },
                _ = tick.tick(), if !pending.is_empty() => {
                    publish(&hub, &session, &pending, &mut stop).await?;
                    pending.clear();
                }
            }
        }
    }
    .await;
    if let Err(error) = result {
        if !*stop.borrow() {
            eprintln!("terminal stream {session} stopped; reattach required: {error}");
            let _ = hub.publish(Event::new(
                "pty.desync",
                "brain",
                json!({"sessionId":session}),
            ));
        }
    }
}
pub(crate) fn install(mut options: Options, hub: Handle) -> Options {
    let Some(engine) = options.engine.clone() else {
        return options;
    };
    let home = options
        .home_dir
        .clone()
        .or_else(|| {
            std::env::var_os(if cfg!(windows) { "USERPROFILE" } else { "HOME" }).map(PathBuf::from)
        })
        .unwrap_or_else(|| std::env::current_dir().unwrap_or_else(|_| PathBuf::from("/")));
    let service = Terminals::new(engine, hub, home);
    options.terminals = Some(service.clone());
    for method in [
        "terminals.open",
        "terminals.create",
        "sessions.attachTerminal",
        "sessions.detachTerminal",
        "sessions.terminalKeepalive",
        "sessions.terminalInput",
        "sessions.terminalResize",
    ] {
        let service = service.clone();
        options = options.handler(method, move |caller, params| {
            let service = service.clone();
            async move { service.call(caller, method, params).await }
        });
    }
    options
}
#[cfg(test)]
mod tests {
    use super::*;
    #[tokio::test]
    async fn viewer_lease_expiry_keeps_live_viewers_and_joins_last_forwarder() {
        let now = Instant::now();
        let (stop, mut stopping) = watch::channel(false);
        let stopped = Arc::new(AtomicBool::new(false));
        let ended = stopped.clone();
        let task = tokio::spawn(async move {
            stopping.changed().await.unwrap();
            assert!(*stopping.borrow());
            ended.store(true, Ordering::Release);
        });
        let mut rows = BTreeMap::from([(
            "session".into(),
            Forwarder {
                leases: BTreeMap::from([
                    (1, now + LEASE),
                    (2, now + LEASE + Duration::from_secs(1)),
                ]),
                stop,
                task,
            },
        )]);
        Terminals::expire_entries(&mut rows, now + LEASE).await;
        assert_eq!(
            rows["session"].leases.len(),
            2,
            "exact deadline still owns its lease"
        );
        Terminals::expire_entries(&mut rows, now + LEASE + Duration::from_millis(1)).await;
        assert_eq!(
            rows["session"].leases.keys().copied().collect::<Vec<_>>(),
            vec![2]
        );
        assert!(!stopped.load(Ordering::Acquire));
        // Renewal replaces a viewer deadline rather than leaking a reference.
        rows.get_mut("session")
            .unwrap()
            .leases
            .insert(2, now + LEASE + Duration::from_secs(5));
        Terminals::expire_entries(&mut rows, now + LEASE + Duration::from_secs(2)).await;
        assert_eq!(rows.len(), 1);
        Terminals::expire_entries(&mut rows, now + LEASE + Duration::from_secs(6)).await;
        assert!(rows.is_empty());
        assert!(
            stopped.load(Ordering::Acquire),
            "last expired viewer must join the owned stream"
        );
    }
    #[test]
    fn shell_entries_are_normalized_but_default_and_requested_argv_are_preserved() {
        let listed = "# comment\n /host/custom-login \n\n";
        for shell in ["/host/env-login", "/host/custom-login"] {
            assert_eq!(
                resolve_shell_from(shell, Some(" /host/env-login "), listed).unwrap(),
                shell
            );
        }
        assert_eq!(
            resolve_shell_from("", Some(" /host/env-login "), listed).unwrap(),
            " /host/env-login "
        );
        assert_eq!(
            resolve_shell_from(" \t", Some("/host/env-login"), listed).unwrap(),
            "/host/env-login"
        );
        for requested in [
            " /host/env-login ",
            "# comment",
            "/host/unknown-program",
            "/bin/sh -c id",
            "../../bin/sh",
            "sh",
        ] {
            let error = resolve_shell_from(requested, Some("# comment"), listed).unwrap_err();
            assert!(
                error.to_string().contains("login shells"),
                "{requested}: {error}"
            );
        }
        #[cfg(not(windows))]
        for absent in [None, Some("")] {
            assert_eq!(resolve_shell_from("", absent, "").unwrap(), "/bin/sh");
            assert_eq!(
                resolve_shell_from("/bin/sh", absent, "").unwrap(),
                "/bin/sh"
            );
        }
        #[cfg(windows)]
        {
            assert_eq!(resolve_shell_from("", None, "").unwrap(), "powershell.exe");
            assert_eq!(
                resolve_shell_from(r"C:\Host\PWSH.EXE", None, "").unwrap(),
                r"C:\Host\PWSH.EXE"
            );
        }
    }
    #[test]
    fn shell_allowlist_and_cwd_are_host_policy() {
        assert_eq!(
            normalize_cwd(" \t/tmp/project///\r", std::path::Path::new("/home/test")),
            "/tmp/project"
        );
        assert_eq!(
            normalize_cwd("", std::path::Path::new("/home/test")),
            "/home/test"
        );
        assert_eq!(
            normalize_cwd("~/literal", std::path::Path::new("/home/test")),
            "~/literal"
        );
        assert!(resolve_shell("/tmp/caller-created-executable").is_err());
        #[cfg(unix)]
        assert_eq!(resolve_shell("/bin/sh").unwrap(), "/bin/sh");
        assert!(dimension(&json!({"cols":-1}), "cols", None).is_err());
        assert_eq!(dimension(&json!({}), "cols", Some(120)).unwrap(), 120);
    }
}
