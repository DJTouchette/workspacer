//! `workspacer cleanup-agents`: inspect, and with `--apply` reclaim, generated
//! artifacts in stopped agent worktrees. Dry-run is the default. The core
//! takes the same per-worktree maintenance file as daemon spawn admission, so
//! running this beside a live backend is safe; it still needs that backend's
//! claudemon API for session state, and skips everything without it.
use super::CommandLine;
use crate::services::worktrees::{
    artifact_cleanup::{Options, cleanup_worktree_artifacts, load_worktree_cleanup_state},
    artifact_cleanup_scheduler::{min_age, root},
};
use anyhow::{Context, Result, bail};
use serde_json::{Value, json};
use std::{io::Write, path::Path};

#[derive(clap::Args, Debug)]
pub struct CleanupArgs {
    /// Remove the eligible artifacts; without it nothing is deleted.
    #[arg(long)]
    pub apply: bool,
    /// Worktree root; default agents.worktreeRoot, else ~/.workspacer/worktrees.
    #[arg(long,value_parser=clap::builder::OsStringValueParser::new())]
    pub root: Option<std::ffi::OsString>,
    /// Minimum age; default agents.artifactCleanup.minAgeHours, else 1.
    #[arg(long)]
    pub min_age_hours: Option<f64>,
    /// claudemon API of the backend that owns the agents; default
    /// http://127.0.0.1:7891 (plus WORKSPACER_PORT_OFFSET).
    #[arg(long)]
    pub daemon_url: Option<String>,
}

/// The file as written, without seeding defaults: a dry run changes nothing.
fn read_config(path: &Path) -> Result<Value> {
    let config = match std::fs::read_to_string(path) {
        Ok(text) => serde_yaml::from_str::<Value>(&text)
            .with_context(|| format!("{} is not valid YAML", path.display()))?,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => json!({}),
        Err(error) => return Err(error.into()),
    };
    match config {
        Value::Null => Ok(json!({})),
        Value::Object(_) => Ok(config),
        _ => bail!("config.yaml must contain a configuration object"),
    }
}

fn default_daemon_url(offset: Option<&str>) -> String {
    let offset = offset
        .and_then(|offset| offset.trim().parse::<i64>().ok())
        .unwrap_or(0);
    format!("http://127.0.0.1:{}", 7891 + offset)
}

pub(super) async fn run(
    args: &CommandLine,
    cleanup: &CleanupArgs,
    home: Option<&Path>,
    out: &mut dyn Write,
) -> Result<i32> {
    let config = read_config(&args.directory()?.join("config.yaml"))?;
    let root = match &cleanup.root {
        Some(root) if !root.is_empty() => std::path::absolute(root)?,
        _ => root(&config, home)?,
    };
    let daemon_url = cleanup.daemon_url.clone().unwrap_or_else(|| {
        default_daemon_url(std::env::var("WORKSPACER_PORT_OFFSET").ok().as_deref())
    });
    let client = reqwest::Client::builder()
        .no_proxy()
        .redirect(reqwest::redirect::Policy::none())
        .build()?;
    let report = cleanup_worktree_artifacts(
        Options {
            root: &root,
            apply: cleanup.apply,
            min_age: min_age(&config, cleanup.min_age_hours)?,
            stop: None,
        },
        || load_worktree_cleanup_state(&client, &daemon_url),
    )
    .await?;
    writeln!(out, "{}", serde_json::to_string_pretty(&report)?)?;
    Ok(if report.errors.is_empty() { 0 } else { 1 })
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;
    use clap::Parser;

    fn parse(extra: &[&str]) -> Result<CommandLine, clap::Error> {
        CommandLine::try_parse_compatible_from(
            ["workspacer", "cleanup-agents"]
                .iter()
                .chain(extra)
                .copied(),
        )
    }
    fn arguments(line: &CommandLine) -> &CleanupArgs {
        match &line.command {
            super::super::Command::CleanupAgents(cleanup) => cleanup,
            other => panic!("unexpected command {other:?}"),
        }
    }

    #[test]
    fn dry_run_is_the_default_and_arguments_are_validated() {
        let line = parse(&[]).unwrap();
        let cleanup = arguments(&line);
        assert!(!cleanup.apply);
        assert!(cleanup.root.is_none() && cleanup.min_age_hours.is_none());
        let line = parse(&[
            "--apply",
            "--root",
            "/tmp/worktrees",
            "--min-age-hours",
            "24",
            "--daemon-url",
            "http://127.0.0.1:7991",
        ])
        .unwrap();
        let cleanup = arguments(&line);
        assert!(cleanup.apply);
        assert_eq!(cleanup.root.as_deref(), Some("/tmp/worktrees".as_ref()));
        assert_eq!(cleanup.min_age_hours, Some(24.0));
        assert_eq!(cleanup.daemon_url.as_deref(), Some("http://127.0.0.1:7991"));
        for invalid in [
            &["--root"][..],
            &["--min-age-hours"],
            &["--min-age-hours", "soon"],
            &["--delete"],
        ] {
            assert!(parse(invalid).is_err(), "{invalid:?}");
        }
        assert!(CommandLine::try_parse_from(["workspacer", "cleanup-agents", "--help"]).is_err());
    }

    #[test]
    fn the_default_daemon_honors_the_port_offset() {
        assert_eq!(default_daemon_url(None), "http://127.0.0.1:7891");
        assert_eq!(default_daemon_url(Some("100")), "http://127.0.0.1:7991");
        assert_eq!(
            default_daemon_url(Some("nonsense")),
            "http://127.0.0.1:7891"
        );
    }

    #[tokio::test]
    async fn prints_the_report_and_never_creates_configuration() {
        let temp = tempfile::tempdir().unwrap();
        let home = temp.path().join("home");
        let config = temp.path().join("config");
        // Nothing listens here; an empty root needs no daemon state at all.
        let run = |extra: Vec<String>| {
            let (home, config) = (home.clone(), config.clone());
            async move {
                let mut line = vec![
                    "workspacer".to_owned(),
                    "--config-dir".into(),
                    config.to_str().unwrap().into(),
                    "cleanup-agents".into(),
                    "--daemon-url".into(),
                    "http://127.0.0.1:9".into(),
                ];
                line.extend(extra);
                let line = CommandLine::try_parse_compatible_from(line).unwrap();
                let mut out = Vec::new();
                let code = run(&line, arguments(&line), Some(&home), &mut out).await;
                (code, String::from_utf8(out).unwrap())
            }
        };
        let (code, out) = run(vec![]).await;
        assert_eq!(code.unwrap(), 0);
        let report: Value = serde_json::from_str(&out).unwrap();
        assert_eq!(report["apply"], false);
        assert_eq!(report["root"], json!(home.join(".workspacer/worktrees")));
        assert_eq!(report["scannedWorktrees"], 0);
        for list in ["artifacts", "skipped", "errors"] {
            assert_eq!(report[list], json!([]), "{list}");
        }
        assert!(!config.exists(), "a dry run must not seed config.yaml");
        // Configured values are honored; flags win over them.
        std::fs::create_dir_all(&config).unwrap();
        let configured = temp.path().join("configured");
        std::fs::write(
            config.join("config.yaml"),
            format!(
                "agents:\n  worktreeRoot: {}\n  artifactCleanup:\n    enabled: false\n    minAgeHours: 3\n",
                configured.display()
            ),
        )
        .unwrap();
        let (code, out) = run(vec!["--apply".into()]).await;
        assert_eq!(code.unwrap(), 0);
        let report: Value = serde_json::from_str(&out).unwrap();
        assert_eq!(report["apply"], true);
        assert_eq!(report["root"], json!(configured));
        let elsewhere = temp.path().join("elsewhere");
        let (_, out) = run(vec!["--root".into(), elsewhere.to_str().unwrap().into()]).await;
        assert_eq!(
            serde_json::from_str::<Value>(&out).unwrap()["root"],
            json!(elsewhere)
        );
        let (code, _) = run(vec!["--min-age-hours".into(), "-1".into()]).await;
        assert!(code.unwrap_err().to_string().contains("nonnegative"));
        std::fs::write(
            config.join("config.yaml"),
            "agents:\n  artifactCleanup:\n    minAgeHours: soon\n",
        )
        .unwrap();
        assert!(run(vec![]).await.0.is_err());
        std::fs::write(config.join("config.yaml"), "- not\n- an object\n").unwrap();
        assert!(run(vec![]).await.0.is_err());
    }
}
