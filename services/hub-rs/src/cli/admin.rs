use super::{CommandLine, FleetCommand, JobsCommand, print_json};
use anyhow::{Result, bail};
use serde_json::{Value, json};
use std::{
    io::{Read, Write},
    path::Path,
    time::Duration,
};
async fn connect(args: &CommandLine) -> Result<crate::client::Client> {
    crate::client::Client::connect_remote(
        &format!("ws://{}/bus", args.authority(args.hub_port)),
        &args.credential()?,
    )
    .await
}
async fn call(args: &CommandLine, method: &str, params: Value) -> Result<Value> {
    let client = connect(args).await?;
    let result = client
        .call_with_timeout(method, params, Duration::from_secs(20))
        .await;
    client.close();
    result
}
pub(super) async fn status(args: &CommandLine, api_port: u16, out: &mut dyn Write) -> Result<i32> {
    async fn probe(address: String, token: String, daemon: bool) -> Value {
        let loopback = address
            .parse::<std::net::SocketAddr>()
            .is_ok_and(|a| a.ip().is_loopback())
            || address.starts_with("localhost:");
        let mut builder = reqwest::Client::builder()
            .redirect(reqwest::redirect::Policy::none())
            .timeout(Duration::from_secs(3));
        if loopback {
            builder = builder.no_proxy();
        }
        let client = match builder.build() {
            Ok(client) => client,
            Err(_) => return json!({"ok":false,"detail":"not running (probe unavailable)"}),
        };
        let response = match client
            .get(format!("http://{address}/health"))
            .bearer_auth(token)
            .send()
            .await
        {
            Ok(response) if response.status() == reqwest::StatusCode::OK => response,
            Ok(response) => {
                return json!({"ok":false,"detail":format!("not running (HTTP {})",response.status())});
            }
            Err(_) => return json!({"ok":false,"detail":"not running (unreachable)"}),
        };
        if daemon {
            let sessions = client
                .get(format!("http://{address}/sessions"))
                .send()
                .await;
            return match sessions {
                Ok(response) if response.status() == reqwest::StatusCode::OK => {
                    match response.json::<Value>().await {
                        Ok(Value::Array(rows)) => {
                            json!({"ok":true,"detail":format!("healthy, {} session(s)",rows.len())})
                        }
                        _ => json!({"ok":true,"detail":"healthy"}),
                    }
                }
                _ => json!({"ok":true,"detail":"healthy (sessions unreadable)"}),
            };
        }
        match response.json::<Value>().await {
            Ok(health) if health["status"] == "ok" => match health["methods"].as_u64() {
                Some(methods) => {
                    json!({"ok":true,"detail":format!("healthy, {methods} capability method(s)")})
                }
                None => {
                    json!({"ok":true,"detail":"healthy (token not accepted — method count hidden)"})
                }
            },
            _ => json!({"ok":false,"detail":"unexpected /health answer"}),
        }
    }
    // Status must still describe an unauthenticated hub; unlike mutating admin
    // commands it does not require a stored pairing credential to probe health.
    let (daemon, hub) = tokio::join!(
        probe(args.authority(api_port), String::new(), true),
        probe(
            args.authority(args.hub_port),
            args.credential().unwrap_or_default(),
            false
        )
    );
    let brain = if hub["ok"] == true {
        match tokio::time::timeout(Duration::from_secs(5), call(args, "brain.info", json!({})))
            .await
        {
            Ok(Ok(_)) => json!({"ok":true,"detail":"registered (brain.info answered)"}),
            Ok(Err(error)) => json!({"ok":false,"detail":format!("probe failed ({error})")}),
            Err(_) => json!({"ok":false,"detail":"probe failed (timed out)"}),
        }
    } else {
        json!({"ok":false,"detail":"not checked (hub is down)"})
    };
    let success = daemon["ok"] == true && hub["ok"] == true;
    let report = json!({"claudemon":daemon,"hub":hub,"brain":brain});
    if args.json {
        print_json(out, &report)?
    } else {
        for (name, endpoint) in [
            ("claudemon", Some(args.authority(api_port))),
            ("hub", Some(args.authority(args.hub_port))),
            ("brain", None),
        ] {
            writeln!(
                out,
                "  {name:<10} {:<5} {}{}",
                if report[name]["ok"] == true {
                    "up"
                } else {
                    "down"
                },
                report[name]["detail"].as_str().unwrap_or(""),
                endpoint
                    .map(|address| format!(" — http://{address}"))
                    .unwrap_or_default()
            )?;
        }
    }
    Ok(if success { 0 } else { 1 })
}
async fn find_job(args: &CommandLine, prefix: &str) -> Result<Value> {
    let result = call(args, "jobs.list", json!({})).await?;
    let jobs = result["jobs"]
        .as_array()
        .ok_or_else(|| anyhow::anyhow!("invalid jobs list"))?;
    let matches: Vec<_> = jobs
        .iter()
        .filter(|job| job["id"].as_str().is_some_and(|id| id.starts_with(prefix)))
        .collect();
    if prefix.is_empty() || matches.len() != 1 {
        bail!("job id must match exactly one job")
    };
    let mut job = matches[0].clone();
    for field in ["nextRunAt", "running", "lastRun"] {
        job.as_object_mut().unwrap().remove(field);
    }
    Ok(job)
}
pub(super) async fn jobs(
    args: &CommandLine,
    command: &JobsCommand,
    out: &mut dyn Write,
) -> Result<i32> {
    let result = match command {
        JobsCommand::List => call(args, "jobs.list", json!({})).await?,
        JobsCommand::Add { file } => {
            let mut bytes = vec![];
            if file == Path::new("-") {
                std::io::stdin().take(64 << 20).read_to_end(&mut bytes)?;
            } else {
                bytes = std::fs::read(file)?;
            }
            let job: Value = serde_json::from_slice(&bytes)?;
            call(args, "jobs.upsert", job).await?
        }
        JobsCommand::Show { id } => find_job(args, id).await?,
        JobsCommand::History { id } => {
            let job = find_job(args, id).await?;
            call(args, "jobs.history", json!({"id":job["id"]})).await?
        }
        JobsCommand::Run { id } => {
            let job = find_job(args, id).await?;
            let result = call(args, "jobs.run", json!({"id":job["id"]})).await?;
            if result["started"] != true {
                print_json(out, &result)?;
                return Ok(1);
            }
            result
        }
        JobsCommand::Remove { id } => {
            let job = find_job(args, id).await?;
            call(args, "jobs.remove", json!({"id":job["id"]})).await?
        }
        JobsCommand::Approve { id, disabled } => {
            let mut job = find_job(args, id).await?;
            if job["proposedBy"].as_str().is_none_or(str::is_empty) {
                job
            } else {
                job["proposedBy"] = "".into();
                job["enabled"] = (!disabled).into();
                call(args, "jobs.upsert", job).await?
            }
        }
        JobsCommand::Enable { id } | JobsCommand::Disable { id } => {
            let mut job = find_job(args, id).await?;
            let enabled = matches!(command, JobsCommand::Enable { .. });
            if enabled && job["proposedBy"].as_str().is_some_and(|p| !p.is_empty()) {
                bail!("job is an unapproved proposal; use jobs approve")
            };
            job["enabled"] = enabled.into();
            call(args, "jobs.upsert", job).await?
        }
    };
    print_json(out, &result)?;
    Ok(0)
}
pub(super) async fn fleet(
    args: &CommandLine,
    command: &FleetCommand,
    out: &mut dyn Write,
) -> Result<i32> {
    let (method, quiet) = match command {
        FleetCommand::Quiescence { quiet } => ("fleet.quiescence", *quiet),
        FleetCommand::Idle { quiet } => ("machine.power", *quiet),
    };
    let raw = call(args, method, json!({})).await?;
    let answer = if method == "machine.power" {
        if raw["idleMode"]
            .as_str()
            .is_none_or(|mode| mode.is_empty() || mode == "off")
        {
            bail!("machine idle detector is disabled or unavailable")
        }
        &raw["idle"]
    } else {
        &raw
    };
    let quiescent = answer["quiescent"]
        .as_bool()
        .ok_or_else(|| anyhow::anyhow!("invalid quiescence response"))?;
    if !quiet {
        if args.json {
            print_json(out, &raw)?
        } else {
            writeln!(out, "{}", if quiescent { "at rest" } else { "not at rest" })?;
            if let Some(blockers) = answer["blockers"].as_array() {
                for blocker in blockers {
                    writeln!(out, "{}", blocker)?;
                }
            }
        }
    }
    Ok(if quiescent { 0 } else { 1 })
}
pub(super) fn install_cli(directory: Option<&Path>, out: &mut dyn Write) -> Result<i32> {
    let source = std::env::current_exe()?.canonicalize()?;
    let directory = if let Some(path) = directory {
        path.to_path_buf()
    } else if cfg!(windows) {
        std::env::var_os("LOCALAPPDATA")
            .map(std::path::PathBuf::from)
            .ok_or_else(|| anyhow::anyhow!("LOCALAPPDATA missing"))?
            .join("workspacer/bin")
    } else {
        std::env::var_os("HOME")
            .map(std::path::PathBuf::from)
            .ok_or_else(|| anyhow::anyhow!("HOME missing"))?
            .join(".local/bin")
    };
    std::fs::create_dir_all(&directory)?;
    let destination = directory.join(if cfg!(windows) {
        "workspacer-rust.exe"
    } else {
        "workspacer-rust"
    });
    if destination.canonicalize().ok().as_ref() == Some(&source) {
        writeln!(out, "already installed: {}", destination.display())?;
        return Ok(0);
    }
    let temporary = directory.join(format!(".workspacer-{}", uuid::Uuid::new_v4()));
    #[cfg(unix)]
    std::os::unix::fs::symlink(&source, &temporary)?;
    #[cfg(windows)]
    {
        std::fs::copy(&source, &temporary)?;
    }
    if let Err(error) = std::fs::rename(&temporary, &destination) {
        let _ = std::fs::remove_file(&temporary);
        return Err(error.into());
    }
    writeln!(
        out,
        "installed {} -> {}",
        destination.display(),
        source.display()
    )?;
    Ok(0)
}
