//! Persisted credential compatibility and provenance-aware authorization.
use crate::protocol::matches;
use anyhow::{Result, anyhow, bail};
use base64::Engine;
use rand::RngCore;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use sha2::{Digest, Sha256};
use std::{
    collections::BTreeMap,
    fs,
    io::Write,
    path::{Path, PathBuf},
    sync::OnceLock,
};

#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum Scope {
    View,
    Triage,
    Operator,
    Provider,
}
impl Scope {
    pub fn parse(raw: &str) -> Result<Self> {
        match raw
            .trim_matches([' ', '\t', '\n', '\r', '\x0b', '\x0c'])
            .to_ascii_lowercase()
            .as_str()
        {
            "view" => Ok(Self::View),
            "triage" => Ok(Self::Triage),
            "operator" => Ok(Self::Operator),
            "provider" => Ok(Self::Provider),
            _ => bail!("unknown scope (want view, triage, operator, or provider)"),
        }
    }
    pub fn name(self) -> &'static str {
        match self {
            Self::View => "view",
            Self::Triage => "triage",
            Self::Operator => "operator",
            Self::Provider => "provider",
        }
    }
    pub fn methods(self) -> &'static [String] {
        &vocabulary().scopes[self.name()]
    }
}

/// Keep unknown/legacy metadata when rewriting a store shared with old clients.
/// No Debug implementation: records contain bearer credentials.
#[derive(Clone, Default, Deserialize, Serialize, PartialEq)]
#[serde(default, rename_all = "camelCase")]
pub struct Record {
    pub token: String,
    pub scope: String,
    #[serde(skip_serializing_if = "String::is_empty")]
    pub label: String,
    pub created: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub provides: Option<Vec<String>>,
    #[serde(skip_serializing_if = "std::ops::Not::not")]
    pub facade_authority: bool,
    #[serde(flatten)]
    pub metadata: BTreeMap<String, Value>,
}
impl Record {
    pub fn scope(&self) -> Option<Scope> {
        // Stored scopes are exact, unlike the user-facing parse helper.
        match self.scope.as_str() {
            "view" => Some(Scope::View),
            "triage" => Some(Scope::Triage),
            "operator" => Some(Scope::Operator),
            "provider" => Some(Scope::Provider),
            _ => None,
        }
    }
    pub fn provides(&self) -> &[String] {
        if self.scope() == Some(Scope::Provider) {
            self.provides.as_deref().unwrap_or(&[])
        } else {
            &[]
        }
    }
}

pub fn load(path: &Path) -> Result<Vec<Record>> {
    match fs::read(path) {
        Ok(bytes) => Ok(serde_json::from_slice::<Option<Vec<Record>>>(&bytes)?.unwrap_or_default()),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(vec![]),
        Err(e) => Err(e.into()),
    }
}

/// Atomically replace the store. For read-modify-write operations use
/// `update_records` so simultaneous credential writers cannot lose records.
pub fn save(path: &Path, records: &[Record]) -> Result<()> {
    prepare_store_parent(path)?;
    let _guard = StoreLock::take(path)?;
    save_unlocked(path, records)
}
fn prepare_store_parent(path: &Path) -> Result<()> {
    if let Some(parent) = path.parent().filter(|p| !p.as_os_str().is_empty()) {
        fs::create_dir_all(parent)?;
    }
    Ok(())
}
/// Serializes cooperating Rust processes through a sidecar lock. Legacy
/// writers that ignore this lock must not share ownership.
pub fn update_records<T>(
    path: &Path,
    update: impl FnOnce(&mut Vec<Record>) -> Result<T>,
) -> Result<T> {
    prepare_store_parent(path)?;
    let _guard = StoreLock::take(path)?;
    let mut records = load(path)?;
    let previous = records.clone();
    let result = update(&mut records)?;
    if records != previous {
        save_unlocked(path, &records)?;
    }
    Ok(result)
}
// Keep a stable lock-file inode: unlinking it after unlock would let a waiter
// hold the old inode while a newcomer acquires a different, unprotected file.
pub(crate) struct StoreLock {
    _file: fs::File,
}
impl StoreLock {
    pub(crate) fn take(path: &Path) -> Result<Self> {
        let lock = path.with_file_name(format!(
            "{}.lock",
            path.file_name().unwrap_or_default().to_string_lossy()
        ));
        let mut options = fs::OpenOptions::new();
        options.read(true).write(true).create(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            options.mode(0o600).custom_flags(libc::O_NOFOLLOW);
        }
        let file = options.open(&lock)?;
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(2);
        loop {
            match fs2::FileExt::try_lock_exclusive(&file) {
                Ok(()) => return Ok(Self { _file: file }),
                Err(error)
                    if error.kind() == std::io::ErrorKind::WouldBlock
                        || error.raw_os_error() == fs2::lock_contended_error().raw_os_error() =>
                {
                    if std::time::Instant::now() >= deadline {
                        bail!("credential store is locked by another writer");
                    }
                    std::thread::sleep(std::time::Duration::from_millis(10));
                }
                Err(error) => return Err(error.into()),
            }
        }
    }
}
fn save_unlocked(path: &Path, records: &[Record]) -> Result<()> {
    let parent = path
        .parent()
        .filter(|p| !p.as_os_str().is_empty())
        .unwrap_or(Path::new("."));
    fs::create_dir_all(parent)?;
    let mut tmp = tempfile::Builder::new()
        .prefix(".tokens-")
        .suffix(".json")
        .tempfile_in(parent)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        tmp.as_file()
            .set_permissions(fs::Permissions::from_mode(0o600))?;
    }
    serde_json::to_writer_pretty(&mut tmp, records)?;
    tmp.write_all(b"\n")?;
    tmp.as_file().sync_all()?;
    tmp.persist(path).map_err(|e| e.error)?;
    Ok(())
}

pub(crate) fn new_record(scope: Scope, label: &str) -> Result<Record> {
    let mut bytes = [0u8; 24];
    rand::rngs::OsRng
        .try_fill_bytes(&mut bytes)
        .map_err(|_| anyhow!("credential random source unavailable"))?;
    let record = Record {
        token: base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(bytes),
        scope: scope.name().into(),
        label: label.into(),
        created: crate::protocol::now(),
        provides: (scope == Scope::Provider).then(|| vec!["*".into()]),
        ..Record::default()
    };
    Ok(record)
}

pub fn mint(path: &Path, scope: Scope, label: &str) -> Result<Record> {
    let record = new_record(scope, label)?;
    update_records(path, |records| {
        records.push(record.clone());
        Ok(record)
    })
}

pub fn revoke(path: &Path, reference: &str) -> Result<Record> {
    let reference = reference.trim();
    if reference.len() < 8 {
        bail!("token reference too short (give the full token or at least 8 leading characters)");
    }
    update_records(path, |records| {
        let indices: Vec<_> = records
            .iter()
            .enumerate()
            .filter(|(_, r)| r.token.starts_with(reference))
            .map(|(i, _)| i)
            .collect();
        let index = match indices.as_slice() {
            [one] => *one,
            [] => bail!("no matching token"),
            _ => bail!("prefix matches more than one token"),
        };
        Ok(records.remove(index))
    })
}

pub fn fingerprint(token: &str) -> String {
    if token.is_empty() {
        return String::new();
    }
    // Same full SHA-256 identity used by the Go bus, never the bearer.
    format!("{:x}", Sha256::digest(token.as_bytes()))
}

/// Match the Go facade's constant-time bearer comparison. Callers retain
/// responsibility for deciding whether an empty credential is permitted.
pub(crate) fn credential_eq(expected: &str, presented: &str) -> bool {
    use subtle::ConstantTimeEq;
    bool::from(expected.as_bytes().ct_eq(presented.as_bytes()))
}

/// Read through each lookup so replacement/deletion/corruption revokes rather
/// than preserving an earlier grant. A cache can be added without changing policy.
#[derive(Clone)]
pub struct Store {
    pub path: PathBuf,
}
impl Store {
    pub fn lookup(&self, token: &str) -> Option<Record> {
        if token.is_empty() {
            return None;
        }
        load(&self.path)
            .ok()?
            .into_iter()
            .rev()
            .find(|r| credential_eq(&r.token, token))
    }
}

/// Operator-facing refusal context only; never serialize a credential Record
/// or return these details to the HTTP caller. Labels remain bounded and JSON
/// encoding at the log site escapes newlines/control characters.
pub(crate) fn scoped_diagnostic(
    host_token: &str,
    path: Option<&Path>,
    token: &str,
) -> Option<Value> {
    if token.is_empty() || (!host_token.is_empty() && credential_eq(host_token, token)) {
        return None;
    }
    let record = Store {
        path: path?.to_owned(),
    }
    .lookup(token)?;
    let scope = record.scope()?;
    let mut label = record.label.chars().take(128).collect::<String>();
    if record.label.chars().count() > 128 {
        label.push('…');
    }
    if label.is_empty() {
        label = "(unlabelled)".into();
    }
    Some(serde_json::json!({"scope":scope.name(),"label":label,"tokenId":fingerprint(token)}))
}

#[cfg(test)]
mod diagnostic_tests {
    use super::*;
    #[test]
    fn scoped_refusal_hint_contains_only_bounded_label_scope_and_fingerprint() {
        let root = tempfile::tempdir().unwrap();
        let path = root.path().join("tokens.json");
        let mut record = Record {
            token: "secret-scoped-fixture".into(),
            scope: "operator".into(),
            label: "fly-node".into(),
            ..Default::default()
        };
        record
            .metadata
            .insert("private".into(), serde_json::json!("must-not-copy"));
        save(&path, &[record.clone()]).unwrap();
        let hint = scoped_diagnostic("host", Some(&path), &record.token).unwrap();
        assert_eq!(
            hint,
            serde_json::json!({"scope":"operator","label":"fly-node","tokenId":fingerprint(&record.token)})
        );
        for token in ["host", "unknown", ""] {
            assert!(scoped_diagnostic("host", Some(&path), token).is_none());
        }
        assert!(scoped_diagnostic("host", None, &record.token).is_none());
        record.label = "\n".repeat(1000);
        save(&path, &[record.clone()]).unwrap();
        let hint = scoped_diagnostic("host", Some(&path), &record.token).unwrap();
        assert_eq!(hint["label"].as_str().unwrap().chars().count(), 129);
        assert!(hint.to_string().len() < 1024);
        assert!(!hint.to_string().contains(&record.token));
        record.label.clear();
        save(&path, &[record.clone()]).unwrap();
        assert_eq!(
            scoped_diagnostic("host", Some(&path), &record.token).unwrap()["label"],
            "(unlabelled)"
        );
    }
}

#[derive(Deserialize)]
struct Vocabulary {
    scopes: BTreeMap<String, Vec<String>>,
    topics: Vec<Topic>,
    #[serde(rename = "spawnKeys")]
    spawn_keys: Vec<String>,
}
#[derive(Deserialize)]
#[serde(rename_all = "PascalCase")]
struct Topic {
    pattern: String,
    disposition: String,
    method: String,
    publisher: String,
}
fn vocabulary() -> &'static Vocabulary {
    static VOCABULARY: OnceLock<Vocabulary> = OnceLock::new();
    VOCABULARY.get_or_init(|| {
        serde_json::from_str(include_str!("../assets/hub-vocabulary.json"))
            .expect("validated hub vocabulary")
    })
}
pub(crate) fn spawn_keys() -> &'static [String] {
    &vocabulary().spawn_keys
}
fn desktop_service(method: &str) -> bool {
    static METHODS: OnceLock<Vec<String>> = OnceLock::new();
    METHODS
        .get_or_init(|| {
            let manifest: Value = serde_json::from_str(include_str!(
                "../../../contracts/desktop-service-methods.json"
            ))
            .expect("validated desktop service manifest");
            manifest["ownerMethods"]
                .as_array()
                .expect("owner methods")
                .iter()
                .map(|method| method.as_str().expect("desktop method name").to_owned())
                .collect()
        })
        .iter()
        .any(|name| name == method)
}
fn topic_spec(topic: &str) -> Option<&'static Topic> {
    let rows = &vocabulary().topics;
    // The Go registry chooses exact names before wildcard patterns.
    rows.iter().find(|r| r.pattern == topic).or_else(|| {
        rows.iter().find(|r| {
            r.pattern
                .strip_suffix('*')
                .is_some_and(|p| topic.starts_with(p))
        })
    })
}

#[derive(Clone)]
pub(crate) enum Kind {
    Host,
    Scoped(Record),
    Plugin { id: String, provides: Vec<String> },
}
#[derive(Clone)]
pub(crate) struct Identity {
    pub kind: Kind,
    pub token_id: String,
    pub federated: bool,
}
impl Identity {
    pub(crate) fn provider_caller(&self, connection_id: u64) -> crate::protocol::ProviderCaller {
        crate::protocol::ProviderCaller {
            version: 1,
            connection_id,
            scope: self.scope().into(),
            authenticated_host: self.authenticated_host(),
            federated: self.federated,
            plugin_id: self.plugin_id().into(),
            token_id: self.token_id.clone(),
            may_assert_session: self.may_assert_session(),
        }
    }
    /// Called only by the host-owned authenticated provider relay. There is no
    /// wire operation which can create this local connection identity.
    pub(crate) fn from_provider_caller(proof: crate::protocol::ProviderCaller) -> Result<Self> {
        anyhow::ensure!(
            proof.version == 1 && proof.connection_id > 0,
            "unsupported provider caller context"
        );
        anyhow::ensure!(
            proof.token_id.is_empty()
                || (proof.token_id.len() == 64
                    && proof
                        .token_id
                        .bytes()
                        .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))),
            "invalid provider caller fingerprint"
        );
        let kind = if !proof.plugin_id.is_empty() {
            anyhow::ensure!(
                proof.plugin_id.len() <= 200
                    && proof.scope.is_empty()
                    && !proof.authenticated_host
                    && !proof.may_assert_session,
                "invalid plugin caller context"
            );
            Kind::Plugin {
                id: proof.plugin_id.clone(),
                provides: Vec::new(),
            }
        } else {
            let scope = Scope::parse(&proof.scope)?;
            anyhow::ensure!(
                scope.name() == proof.scope,
                "noncanonical provider caller scope"
            );
            if proof.authenticated_host {
                anyhow::ensure!(
                    scope == Scope::Operator && !proof.federated && proof.may_assert_session,
                    "invalid owner caller context"
                );
                Kind::Host
            } else {
                anyhow::ensure!(
                    !proof.may_assert_session || (scope == Scope::Operator && !proof.federated),
                    "invalid session attribution authority"
                );
                Kind::Scoped(Record {
                    scope: proof.scope.clone(),
                    facade_authority: proof.may_assert_session,
                    ..Record::default()
                })
            }
        };
        let identity = Self {
            kind,
            token_id: proof.token_id,
            federated: proof.federated,
        };
        anyhow::ensure!(
            identity.authenticated_host() == proof.authenticated_host
                && identity.may_assert_session() == proof.may_assert_session,
            "inconsistent provider caller context"
        );
        Ok(identity)
    }
    pub(crate) fn may_assert_session(&self) -> bool {
        if !self.trusted() || self.federated || !self.plugin_id().is_empty() {
            return false;
        }
        match &self.kind {
            Kind::Host => true,
            Kind::Scoped(record) => {
                record.facade_authority && record.scope() == Some(Scope::Operator)
            }
            _ => false,
        }
    }
    pub fn host(token: &str) -> Self {
        Self {
            kind: Kind::Host,
            token_id: fingerprint(token),
            federated: false,
        }
    }
    pub fn scope(&self) -> &str {
        match &self.kind {
            Kind::Host => "operator",
            Kind::Scoped(r) => &r.scope,
            Kind::Plugin { .. } => "",
        }
    }
    pub fn trusted(&self) -> bool {
        matches!(&self.kind, Kind::Host)
            || matches!(&self.kind, Kind::Scoped(r) if r.scope() == Some(Scope::Operator))
    }
    pub fn authenticated_host(&self) -> bool {
        matches!(self.kind, Kind::Host) && !self.federated
    }
    pub fn plugin_id(&self) -> &str {
        match &self.kind {
            Kind::Plugin { id, .. } => id,
            _ => "",
        }
    }
    pub fn methods(&self) -> Vec<String> {
        match &self.kind {
            Kind::Host => vec!["*".into()],
            Kind::Scoped(r) => r.scope().map(|s| s.methods().to_vec()).unwrap_or_default(),
            Kind::Plugin { .. } => vec![],
        }
    }
    pub fn may_call(&self, method: &str) -> bool {
        if method.starts_with("desktop.") {
            return self.authenticated_host() && desktop_service(method);
        }
        if method == "files.receiveUpload" {
            return self.authenticated_host();
        } // widened only with upload receiver integration
        self.trusted()
            || match &self.kind {
                Kind::Scoped(r) => r
                    .scope()
                    .is_some_and(|s| s.methods().iter().any(|p| matches(p, method))),
                Kind::Plugin { .. } => true,
                _ => false,
            }
    }
    pub fn may_provide(&self, method: &str) -> bool {
        self.trusted()
            || match &self.kind {
                Kind::Scoped(r) => r.provides().iter().any(|p| matches(p, method)),
                Kind::Plugin { id, provides } => {
                    method.starts_with(&format!("{id}."))
                        && provides.iter().any(|p| matches(p, method))
                }
                _ => false,
            }
    }
    pub fn may_publish(&self, topic: &str) -> bool {
        if topic.starts_with("agent.dispatch.") {
            return topic == "agent.dispatch.update"
                && !self.federated
                && (self.authenticated_host()
                    || (self.scope() == "provider" && self.may_provide("agents.spawn")));
        }
        if self.trusted() {
            return true;
        }
        if let Some(spec) = topic_spec(topic) {
            return !spec.publisher.is_empty() && self.may_provide(&spec.publisher);
        }
        matches!(self.kind, Kind::Plugin { .. })
    }
    pub fn may_consume(&self, topic: &str) -> bool {
        if topic.starts_with("agent.dispatch.") && topic != "agent.dispatch.update" {
            return self.authenticated_host();
        }
        if self.trusted() {
            return true;
        }
        if self.scope() == "provider" {
            return false;
        }
        let spec = topic_spec(topic);
        match &self.kind {
            Kind::Scoped(_) => spec.is_some_and(|s| match s.disposition.as_str() {
                "guarded-by-capability" => self.may_call(&s.method),
                "host-only" => false,
                "open-by-decision" => true,
                _ => false,
            }),
            Kind::Plugin { .. } => spec.is_none_or(|s| s.disposition != "host-only"),
            _ => false,
        }
    }
}

#[cfg(test)]
mod desktop_contract_tests {
    use super::*;

    #[test]
    fn desktop_service_manifest_authority_cases() {
        let contract: Value = serde_json::from_str(include_str!(
            "../../../contracts/desktop-service-methods.json"
        ))
        .unwrap();
        for row in contract["authorityCases"].as_array().unwrap() {
            let name = row["identity"].as_str().unwrap();
            let identity = match name {
                "owner" => Identity::host("fixture"),
                "operator" | "facade" | "provider" => Identity {
                    kind: Kind::Scoped(Record {
                        scope: if name == "provider" {
                            "provider"
                        } else {
                            "operator"
                        }
                        .into(),
                        facade_authority: name == "facade",
                        provides: Some(vec!["*".into()]),
                        ..Default::default()
                    }),
                    token_id: String::new(),
                    federated: false,
                },
                _ => panic!("unhandled authority case {name}"),
            };
            for method in contract["ownerMethods"].as_array().unwrap() {
                let method = method.as_str().unwrap();
                assert_eq!(
                    identity.may_call(method),
                    row["owner"].as_bool().unwrap(),
                    "{name}: {method}"
                );
            }
        }
        let mut remote_owner = Identity::host("fixture");
        remote_owner.federated = true;
        for method in contract["ownerMethods"].as_array().unwrap() {
            assert!(!remote_owner.may_call(method.as_str().unwrap()));
        }
    }
}
