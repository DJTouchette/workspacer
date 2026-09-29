use super::{CommandLine, TokenCommand, print_json};
use crate::auth::{self, Scope};
use anyhow::{Result, bail};
use serde_json::json;
use std::{
    io::Write,
    path::{Path, PathBuf},
};
pub(super) fn home_directory() -> Result<PathBuf> {
    std::env::var_os(if cfg!(windows) { "USERPROFILE" } else { "HOME" })
        .filter(|value| !value.is_empty())
        .map(PathBuf::from)
        .ok_or_else(|| anyhow::anyhow!("home directory unavailable"))
}
pub fn config_directory() -> Result<PathBuf> {
    #[cfg(windows)]
    if let Some(root) = std::env::var_os("APPDATA").filter(|v| !v.is_empty()) {
        return Ok(PathBuf::from(root).join("workspacer"));
    }
    #[cfg(not(windows))]
    if let Some(root) = std::env::var_os("XDG_CONFIG_HOME").filter(|v| !v.is_empty()) {
        return Ok(PathBuf::from(root).join("workspacer"));
    }
    let home = home_directory()?;
    Ok(PathBuf::from(home).join(if cfg!(windows) {
        "AppData/Roaming/workspacer"
    } else {
        ".config/workspacer"
    }))
}
pub fn load_or_create_host_token(directory: &Path, allow_new: bool) -> Result<String> {
    std::fs::create_dir_all(directory)?;
    let _lock = auth::StoreLock::take(&directory.join(".remote-token"))?;
    let path = directory.join("remote-token");
    match std::fs::read_to_string(&path) {
        Ok(token) if !token.trim().is_empty() => return Ok(token.trim().into()),
        Ok(_) => {}
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
        Err(e) => return Err(e.into()),
    }
    // Preserve the CLI's read-error refusal; the diagnostic helper itself
    // follows legacy Suspected's missing/unreadable-root -> false contract.
    std::fs::read_dir(directory)?;
    let previous = crate::state_loss::suspected_ignoring(
        directory,
        std::ffi::OsStr::new("remote-token"),
        &[std::ffi::OsStr::new(".remote-token.lock")],
    );
    if previous && !allow_new {
        bail!(
            "STATE LOSS: remote-token is missing but this configuration directory contains existing state; restore the pairing credential, supply --token, or explicitly use --allow-new-token"
        )
    }
    use base64::Engine;
    use rand::RngCore;
    let mut bytes = [0u8; 24];
    rand::rngs::OsRng
        .try_fill_bytes(&mut bytes)
        .map_err(|_| anyhow::anyhow!("credential random source unavailable"))?;
    let token = base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(bytes);
    let mut file = tempfile::NamedTempFile::new_in(directory)?;
    writeln!(file, "{token}")?;
    file.as_file().sync_all()?;
    file.persist(path).map_err(|e| e.error)?;
    Ok(token)
}
pub(super) fn token(
    args: &CommandLine,
    command: &TokenCommand,
    out: &mut dyn Write,
) -> Result<i32> {
    let path = args.tokens_path()?;
    match command {
        TokenCommand::InitHost { allow_new_token } => {
            let directory = args.directory()?;
            let token = load_or_create_host_token(&directory, *allow_new_token)?;
            print_json(
                out,
                &json!({"path":directory.join("remote-token"),"prefix":token.chars().take(8).collect::<String>()}),
            )?;
        }
        TokenCommand::Create { scope, label, .. } => {
            let record = auth::mint(&path, Scope::parse(scope)?, label)?;
            if args.json {
                print_json(out, &serde_json::to_value(record)?)?
            } else {
                writeln!(out, "{}", record.token)?
            }
        }
        TokenCommand::List => {
            let rows:Vec<_>=auth::load(&path)?.into_iter().map(|record|json!({"prefix":record.token.chars().take(8).collect::<String>(),"scope":record.scope,"label":record.label,"created":record.created,"facadeAuthority":record.facade_authority})).collect();
            if args.json {
                print_json(out, &json!(rows))?
            } else {
                for row in rows {
                    writeln!(
                        out,
                        "{}  {}  {}",
                        row["prefix"].as_str().unwrap(),
                        row["scope"].as_str().unwrap(),
                        row["label"].as_str().unwrap()
                    )?
                }
            }
        }
        TokenCommand::Revoke { reference } => {
            let record = auth::revoke(&path, reference)?;
            print_json(
                out,
                &json!({"revoked":true,"scope":record.scope,"label":record.label}),
            )?
        }
        TokenCommand::FacadeAuthority { label, enabled } => {
            auth::update_records(&path, |records| {
                let indices: Vec<_> = records
                    .iter()
                    .enumerate()
                    .filter(|(_, r)| r.label == *label)
                    .map(|(i, _)| i)
                    .collect();
                if label.is_empty() || indices.len() != 1 {
                    bail!("service label must identify exactly one token")
                };
                let record = &mut records[indices[0]];
                if record.scope() != Some(Scope::Operator)
                    || record.label.starts_with("session:")
                    || record.label.starts_with("Remote Control: ")
                    || record
                        .metadata
                        .get("role")
                        .is_some_and(|role| !role.is_null() && role != "")
                {
                    bail!(
                        "facade authority requires a dedicated operator service token, not a session or pairing"
                    )
                };
                record.facade_authority = *enabled;
                Ok(())
            })?;
            print_json(out, &json!({"label":label,"facadeAuthority":enabled}))?
        }
    }
    Ok(0)
}
