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
impl RoutingService {
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
