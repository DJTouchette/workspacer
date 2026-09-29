//! Host-owned previous-run evidence; never infer a clean exit from bad data.
use crate::services::{files::bounded_bytes, nodes::ExitRecord};
use serde_json::Value;
use std::{
    ffi::OsString,
    path::{Path, PathBuf},
};

pub(crate) fn path(explicit: Option<OsString>, volume: Option<OsString>) -> Option<PathBuf> {
    if let Some(path) = explicit.filter(|path| !path.is_empty()) {
        return Some(path.into());
    }
    let volume = volume?;
    let volume = if let Some(text) = volume.to_str() {
        let text = text.trim();
        if text.is_empty() {
            return None;
        }
        PathBuf::from(text)
    } else {
        PathBuf::from(volume)
    };
    Some(volume.join("state/last-exit.json"))
}

pub(super) fn read(path: &Path) -> Option<ExitRecord> {
    // This is a trusted host-selected path, so retain ordinary relative and
    // symlink spellings before invoking the canonical-only bounded reader.
    let selected = std::fs::canonicalize(path).ok()?;
    let bytes = bounded_bytes(&selected, 64 * 1024).ok()?;
    let value: Value = serde_json::from_slice(&bytes).ok()?;
    if !value.is_object() {
        return None;
    }
    let reason = value["reason"].as_str()?.to_owned();
    if reason.is_empty() {
        return None;
    }
    let exit_code = match value.get("exitCode") {
        None | Some(Value::Null) => None,
        Some(value) => Some(value.as_i64()?),
    };
    let at = match value.get("at") {
        None | Some(Value::Null) => String::new(),
        Some(Value::String(at)) => at.clone(),
        _ => return None,
    };
    Some(ExitRecord {
        reason,
        exit_code,
        at,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn exit_path_is_inert_without_a_volume_and_preserves_explicit_override() {
        for volume in [None, Some("".into()), Some(" \t\n ".into())] {
            assert_eq!(path(None, volume), None);
        }
        assert_eq!(
            path(None, Some(" /data \n".into())),
            Some(PathBuf::from("/data").join("state/last-exit.json"))
        );
        assert_eq!(
            path(Some("chosen-exit.json".into()), Some(" /data ".into())),
            Some(PathBuf::from("chosen-exit.json"))
        );
        assert_eq!(
            path(Some("".into()), Some("/data".into())),
            path(None, Some("/data".into()))
        );
    }
    #[test]
    fn exact_entrypoint_records_and_nullable_optional_fields_remain_evidence() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("exit.json");
        std::fs::write(&file,"{\"bootId\":\"20260824T210000Z-abc\",\"reason\":\"claudemon-died\",\"exitCode\":1,\"at\":\"2026-08-24T21:00:00Z\",\"machine\":\"17811944b12345\"}\n").unwrap();
        let record = read(&file).unwrap();
        assert_eq!(record.reason, "claudemon-died");
        assert_eq!(record.exit_code, Some(1));
        assert_eq!(record.at, "2026-08-24T21:00:00Z");
        assert_eq!(
            serde_json::to_value(record).unwrap(),
            serde_json::json!({"reason":"claudemon-died","exitCode":1,"at":"2026-08-24T21:00:00Z"})
        );
        std::fs::write(&file, r#"{"reason":"signal-TERM","exitCode":0,"at":null}"#).unwrap();
        assert_eq!(
            serde_json::to_value(read(&file).unwrap()).unwrap(),
            serde_json::json!({"reason":"signal-TERM","exitCode":0})
        );
        for bytes in [
            "{not json",
            "{}",
            "null",
            "[]",
            "[\"brain-died\",1,\"now\"]",
            r#"{"reason":""}"#,
            r#"{"reason":"brain-died","exitCode":"1"}"#,
            r#"{"reason":"brain-died","at":17}"#,
        ] {
            std::fs::write(&file, bytes).unwrap();
            assert!(read(&file).is_none(), "{bytes}");
        }
        std::fs::write(
            &file,
            serde_json::json!({"reason":"x".repeat(64*1024)}).to_string(),
        )
        .unwrap();
        assert!(read(&file).is_none());
        assert!(read(&dir.path().join("missing")).is_none());
        assert!(read(dir.path()).is_none());
    }
    #[tokio::test]
    async fn relay_caches_the_previous_run_once_per_owned_lifetime() {
        use super::super::{Config, Relay, Scope};
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("exit.json");
        std::fs::write(&file, r#"{"reason":"brain-died","exitCode":1}"#).unwrap();
        let hub = crate::Hub::start(crate::Options::default()).unwrap();
        hub.ready().await.unwrap();
        let config = Config {
            url: "ws://127.0.0.1:1/bus".into(),
            token: "unused-fixture-token".into(),
            scope: Scope::Catalog,
            node_id: String::new(),
            last_exit_file: Some(file.clone()),
            caller_token: None,
        };
        // Construction reads metadata but does not start an upstream connection.
        let relay = Relay::new(config.clone(), hub.handle(), None).unwrap();
        std::fs::write(&file, r#"{"reason":"signal-TERM","exitCode":0}"#).unwrap();
        assert_eq!(relay.last_exit.as_ref().unwrap().reason, "brain-died");
        let replacement = Relay::new(config, hub.handle(), None).unwrap();
        assert_eq!(
            replacement.last_exit.as_ref().unwrap().reason,
            "signal-TERM"
        );
        hub.shutdown().unwrap();
    }
    #[test]
    fn relative_exit_overrides_and_file_symlinks_resolve_before_bounded_read() {
        if std::env::var_os("WKS_LAST_EXIT_RELATIVE_FIXTURE").is_some() {
            let chosen = path(Some("state/last-exit.json".into()), None).unwrap();
            assert_eq!(read(&chosen).unwrap().reason, "brain-died");
            println!("relative-last-exit-read-proved");
            return;
        }
        let dir = tempfile::tempdir().unwrap();
        std::fs::create_dir(dir.path().join("state")).unwrap();
        let file = dir.path().join("state/last-exit.json");
        std::fs::write(&file, r#"{"reason":"brain-died","exitCode":1}"#).unwrap();
        let output=std::process::Command::new(std::env::current_exe().unwrap())
            .args(["--exact","provider_relay::last_exit::tests::relative_exit_overrides_and_file_symlinks_resolve_before_bounded_read","--nocapture"])
            .current_dir(dir.path()).env("WKS_LAST_EXIT_RELATIVE_FIXTURE","1").output().unwrap();
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        assert!(
            String::from_utf8_lossy(&output.stdout).contains("relative-last-exit-read-proved"),
            "child test did not run"
        );
        #[cfg(any(unix, windows))]
        {
            let alias = dir.path().join("exit-alias.json");
            #[cfg(unix)]
            std::os::unix::fs::symlink(&file, &alias).unwrap();
            #[cfg(windows)]
            std::os::windows::fs::symlink_file(&file, &alias)
                .expect("Windows contract CI must provide symlink privilege");
            assert_eq!(read(&alias).unwrap().reason, "brain-died");
        }
    }
}
