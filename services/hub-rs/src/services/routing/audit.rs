use super::*;
use std::io::Write;
pub(super) struct DecisionLog {
    path: PathBuf,
    state: Mutex<bool>,
}
impl DecisionLog {
    pub fn new(directory: &Path) -> Self {
        Self {
            path: directory.join("routing-decisions.jsonl"),
            state: Mutex::new(false),
        }
    }
    pub fn append(&self, row: Value) {
        let mut failed = self.state.lock().unwrap();
        match self.write(row) {
            Ok(()) => *failed = false,
            Err(e) => {
                if !*failed {
                    eprintln!("routing decision log unavailable: {e}");
                    *failed = true;
                }
            }
        }
    }
    fn write(&self, row: Value) -> Result<()> {
        let directory = self.path.parent().unwrap();
        std::fs::create_dir_all(directory)?;
        let mut file = self.open()?;
        if file.metadata()?.len() >= 8 * 1024 * 1024 {
            drop(file);
            let previous = self.path.with_extension("jsonl.1");
            // Windows rename does not replace an existing destination.
            if previous.exists() {
                std::fs::remove_file(&previous)?;
            }
            std::fs::rename(&self.path, &previous)?;
            file = self.open()?;
        }
        let mut line = serde_json::to_vec(&row)?;
        line.push(b'\n');
        file.write_all(&line)?;
        Ok(())
    }
    #[cfg(windows)]
    fn open(&self) -> Result<std::fs::File> {
        super::windows_audit::open(&self.path)
    }
    #[cfg(not(any(unix, windows)))]
    fn open(&self) -> Result<std::fs::File> {
        bail!("private routing audit unsupported on this platform")
    }
    #[cfg(unix)]
    fn open(&self) -> Result<std::fs::File> {
        use std::os::unix::fs::{OpenOptionsExt, PermissionsExt};
        if std::fs::symlink_metadata(&self.path).is_ok_and(|m| m.file_type().is_symlink()) {
            bail!("audit file must not be a symlink");
        }
        let file = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .mode(0o600)
            .open(&self.path)?;
        file.set_permissions(std::fs::Permissions::from_mode(0o600))?;
        Ok(file)
    }
}
/// One operation may resolve twice around slow worktree/template preparation.
/// Keep the latest effective tuple and accumulated restrictions, then append one
/// allowlisted routing-admission receipt when that operation leaves its scope.
pub(crate) struct SpawnAudit<'a> {
    service: &'a RoutingService,
    scope: String,
    token_id: String,
    row: Option<Value>,
    scrubbed: std::collections::BTreeSet<String>,
    capability_refused: bool,
}
impl SpawnAudit<'_> {
    pub(crate) fn begin_resolution(&mut self) {
        self.scrubbed.clear();
        self.capability_refused = false;
    }
    pub(crate) fn check(&mut self, params: &mut Value) -> Result<Vec<String>> {
        let matrix = self.service.matrix();
        let fresh = (!word(&params["resumeSessionId"]).is_empty())
            .then(|| freshness(&matrix, params))
            .flatten();
        let mut ceiling = json!({"capabilityRefused":false,"resumeRefused":false,"denied":false});
        if let Some((key, row)) = super::ceiling(&matrix, word(&params["cwd"])) {
            let max = word(&row["max_capability"]);
            ceiling["key"] = json!(key);
            ceiling["maxCapability"] = json!(max);
            let capability = word(&params["capability"]).to_lowercase();
            let pin = provider(word(&params["provider"]));
            let limit = rank(&matrix, max);
            if !max.is_empty()
                && limit != i64::MAX
                && ((!capability.is_empty() && rank(&matrix, &capability) > limit)
                    || named_rank(
                        &matrix,
                        &pin,
                        word(&params["model"]),
                        word(&params["effort"]),
                    ) > limit)
            {
                self.capability_refused = true;
            }
        }
        let result = sanitize(&matrix, params);
        if let Ok(fields) = &result {
            self.scrubbed.extend(fields.iter().cloned());
        }
        ceiling["capabilityRefused"] = json!(self.capability_refused);
        ceiling["denied"] = json!(result.is_err());
        if result.is_err() {
            if let Some(capability) = fresh {
                ceiling["resumeRefused"] = json!(true);
                ceiling["freshCapability"] = json!(capability);
            }
        }
        let mut spawn = safe_spawn(params);
        spawn["callerScope"] = json!(self.scope);
        spawn["callerTokenId"] = json!(self.token_id);
        spawn["ceiling"] = ceiling;
        spawn["scrubbed"] = json!(self.scrubbed);
        spawn["outcome"] = json!(if result.is_err() {
            "refused"
        } else if self.scrubbed.is_empty() {
            "allowed"
        } else {
            "clamped"
        });
        self.row = Some(json!({"kind":"spawn","at":chrono::Utc::now().to_rfc3339(),
            "decisionId":params["decisionId"].as_str(),"phase":"routing","spawn":spawn}));
        result
    }
    pub(crate) fn extend_scrubbed(&mut self, fields: &[String]) {
        self.scrubbed.extend(fields.iter().cloned());
        if let Some(row) = &mut self.row {
            row["spawn"]["scrubbed"] = json!(self.scrubbed);
            if row["spawn"]["outcome"] != "refused" && !self.scrubbed.is_empty() {
                row["spawn"]["outcome"] = json!("clamped");
            }
        }
    }
    pub(crate) fn refuse_profile_override(&mut self) {
        if let Some(row) = &mut self.row {
            row["spawn"]["outcome"] = json!("refused");
            row["spawn"]["ceiling"]["denied"] = json!(true);
            row["spawn"]["ceiling"]["profileOverride"] = json!(true);
        }
    }
}
impl Drop for SpawnAudit<'_> {
    fn drop(&mut self) {
        if let Some(row) = self.row.take() {
            self.service.log.append(row);
        }
    }
}
fn safe_spawn(params: &Value) -> Value {
    let mut spawn = json!({});
    for key in [
        "role",
        "capability",
        "provider",
        "model",
        "effort",
        "toolScope",
    ] {
        if let Some(value) = params[key].as_str() {
            spawn[key] = json!(value);
        }
    }
    if let Some(cwd) = canonical(word(&params["cwd"])) {
        spawn["cwd"] = json!(cwd.to_string_lossy());
    }
    spawn
}
impl RoutingService {
    pub(crate) fn begin_spawn_audit(&self, caller: Option<&crate::Caller>) -> SpawnAudit<'_> {
        SpawnAudit {
            service: self,
            scope: caller
                .map(|c| c.scope.clone())
                .unwrap_or_else(|| "internal".into()),
            token_id: caller.map(|c| c.token_id.clone()).unwrap_or_default(),
            row: None,
            scrubbed: Default::default(),
            capability_refused: false,
        }
    }
    pub fn log_decision(&self, decision: &Value) {
        self.log.append(json!({"kind":"decision","at":chrono::Utc::now().to_rfc3339(),"decisionId":decision["decisionId"],"decision":decision}));
    }
    pub fn audit_spawn(&self, caller: &crate::Caller, params: &Value, scrubbed: &[String]) {
        let mut spawn =
            json!({"callerScope":caller.scope,"callerTokenId":caller.token_id,"scrubbed":scrubbed});
        for key in [
            "role",
            "capability",
            "provider",
            "model",
            "effort",
            "toolScope",
        ] {
            if params.get(key).is_some() {
                spawn[key] = params[key].clone();
            }
        }
        if let Some(cwd) = canonical(word(&params["cwd"])) {
            spawn["cwd"] = cwd.to_string_lossy().into_owned().into();
        }
        self.log.append(json!({"kind":"spawn","at":chrono::Utc::now().to_rfc3339(),"decisionId":params["decisionId"],"spawn":spawn}));
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    #[cfg(unix)]
    fn repairs_permissions_and_drops_sensitive_spawn_fields() {
        use std::os::unix::fs::PermissionsExt;
        let dir = tempfile::tempdir().unwrap();
        let service = RoutingService::open(dir.path().into()).unwrap();
        let path = dir.path().join("routing-decisions.jsonl");
        std::fs::write(&path, b"").unwrap();
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o644)).unwrap();
        let caller = crate::Caller {
            call_id: 0,
            activity_seq: 0,
            federated: false,
            connection_id: 0,
            authenticated_host: true,
            trusted: true,
            scope: "operator".into(),
            plugin_id: String::new(),
            token_id: "fingerprint".into(),
        };
        service.audit_spawn(
            &caller,
            &json!({"prompt":"secret prose","apiKey":"secret credential","provider":"codex"}),
            &[],
        );
        let bytes = std::fs::read_to_string(&path).unwrap();
        assert!(!bytes.contains("secret"));
        assert_eq!(
            std::fs::metadata(path).unwrap().permissions().mode() & 0o777,
            0o600
        );
    }
}
