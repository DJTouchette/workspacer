use super::{Operation, Plan};
#[cfg(windows)]
#[path = "windows_command.rs"]
mod windows_command;
use anyhow::{Result, anyhow, bail};
use claudemon::child_env::SanitizeChildEnvironment;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::{collections::BTreeMap, path::Path, process::Stdio, time::Duration};
use tokio::io::{AsyncBufReadExt, AsyncReadExt, AsyncWriteExt, BufReader};
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct Provider {
    pub id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub base_url: Option<String>,
}
pub fn routing_config_args(args: &[String]) -> Result<Vec<String>> {
    let mut selected = vec![];
    let mut index = 0;
    while index < args.len() {
        let arg = &args[index];
        if arg == "-p"
            || arg == "--profile"
            || arg.starts_with("--profile=")
            || arg.starts_with("-p") && arg.len() > 2
        {
            bail!("Codex launch integrations require base routing without a native preset")
        }
        if arg == "-c" || arg == "--config" {
            let value = args
                .get(index + 1)
                .filter(|value| !value.is_empty())
                .ok_or_else(|| anyhow!("Missing Codex config override value"))?;
            selected.extend([arg.clone(), value.clone()]);
            index += 1;
        } else if arg.starts_with("--config=") || arg.starts_with("-c") && arg.len() > 2 {
            selected.push(arg.clone())
        }
        index += 1;
    }
    Ok(selected)
}
pub fn provider_from_config(
    config: &Value,
    env: &BTreeMap<String, String>,
    inherited_base: Option<&str>,
) -> Result<Provider> {
    if !config.is_object() {
        bail!("Invalid Codex configuration response")
    }
    if config
        .get("profile")
        .is_some_and(|value| !value.is_null() && value != false && value != 0 && value != "")
    {
        bail!("Launch integrations require base Codex routing without a default named profile")
    }
    let id = config
        .get("model_provider")
        .filter(|value| !value.is_null())
        .map(|value| value.as_str().unwrap_or(""))
        .unwrap_or("openai");
    if id.is_empty() || id.encode_utf16().count() > 200 {
        bail!("Invalid Codex model provider")
    }
    let base = if id == "openai" {
        config
            .get("openai_base_url")
            .filter(|value| !value.is_null())
            .cloned()
            .or_else(|| env.get("OPENAI_BASE_URL").map(|v| json!(v)))
            .or_else(|| inherited_base.map(|v| json!(v)))
            .or_else(|| {
                config["model_providers"]["openai"]
                    .get("base_url")
                    .filter(|v| !v.is_null())
                    .cloned()
            })
    } else {
        config["model_providers"][id]
            .get("base_url")
            .filter(|v| !v.is_null())
            .cloned()
    };
    if id != "openai" && base.as_ref().is_none_or(|value| value == "") {
        bail!("Selected Codex provider needs an explicit base_url")
    }
    let base_url = match base {
        Some(value) => {
            let raw = value
                .as_str()
                .ok_or_else(|| anyhow!("Invalid Codex provider base URL"))?;
            let url =
                url::Url::parse(raw).map_err(|_| anyhow!("Invalid Codex provider base URL"))?;
            if !["http", "https"].contains(&url.scheme())
                || !url.username().is_empty()
                || url.password().is_some()
                || url.query().is_some()
                || url.fragment().is_some()
                || url.host().is_none()
            {
                bail!("Codex provider URL must be HTTP(S), without credentials, query, or fragment")
            };
            Some(
                url.as_str()
                    .strip_suffix('/')
                    .unwrap_or(url.as_str())
                    .to_owned(),
            )
        }
        None => None,
    };
    Ok(Provider {
        id: id.into(),
        base_url,
    })
}
pub(super) trait Probe: Send + Sync {
    fn read<'a>(&'a self, plan: &'a Plan) -> Operation<'a, Provider>;
}
pub(super) struct NativeProbe;
impl Probe for NativeProbe {
    fn read<'a>(&'a self, plan: &'a Plan) -> Operation<'a, Provider> {
        Box::pin(read(plan))
    }
}
fn command(
    binary: &str,
    _cwd: &Path,
    _env: &BTreeMap<String, String>,
) -> Result<(String, Vec<String>)> {
    #[cfg(windows)]
    let resolved = windows_command::resolve(binary, _cwd, _env);
    #[cfg(windows)]
    let binary = resolved.as_str();
    if !binary.to_ascii_lowercase().ends_with(".cmd") {
        return Ok((binary.into(), vec![]));
    }
    let dir = Path::new(binary).parent().unwrap_or(Path::new("."));
    let script = dir.join("node_modules/@openai/codex/bin/codex.js");
    if !script.is_file() {
        bail!("Codex integration requires a native binary or standard npm installation")
    }
    let node = dir.join("node.exe");
    Ok((
        if node.is_file() {
            node.to_string_lossy().into_owned()
        } else {
            "node".into()
        },
        vec![script.to_string_lossy().into_owned()],
    ))
}
async fn read(plan: &Plan) -> Result<Provider> {
    let args_key = if plan.endpoint == "/sessions/spawn" {
        "argv"
    } else {
        "extra_args"
    };
    let all_args: Vec<String> = plan
        .request
        .get(args_key)
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .map(|arg| {
            arg.as_str()
                .map(str::to_owned)
                .ok_or_else(|| anyhow!("launch argument must be string"))
        })
        .collect::<Result<_>>()?;
    let binary = plan.request["bin"]
        .as_str()
        .or_else(|| all_args.first().map(String::as_str))
        .filter(|s| !s.is_empty())
        .ok_or_else(|| anyhow!("Codex executable required"))?;
    let args = if args_key == "argv" {
        all_args
            .get(1..)
            .ok_or_else(|| anyhow!("Codex launch argv is empty"))?
    } else {
        &all_args[..]
    };
    let env: BTreeMap<String, String> = serde_json::from_value(
        plan.request
            .get("env")
            .cloned()
            .unwrap_or_else(|| json!({})),
    )?;
    let cwd = plan.request["cwd"]
        .as_str()
        .ok_or_else(|| anyhow!("Codex probe cwd required"))?;
    let (binary, mut arguments) = command(binary, Path::new(cwd), &env)?;
    arguments.extend(["app-server".into(), "--listen".into(), "stdio://".into()]);
    arguments.extend(routing_config_args(args)?);
    let cwd = plan.request["cwd"]
        .as_str()
        .ok_or_else(|| anyhow!("Codex probe cwd required"))?;
    let mut command = tokio::process::Command::new(binary);
    command
        .args(arguments)
        .current_dir(cwd)
        .envs(&env)
        .env_remove("HUB_TOKEN")
        .env_remove("WKS_MCP_TOKEN")
        .env_remove("WORKSPACER_HUB_TOKEN")
        .env_remove("WORKSPACER_PARENT_PID")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .kill_on_drop(true);
    #[cfg(unix)]
    command.process_group(0);
    command.scrub_host_authority();
    #[cfg(windows)]
    let (mut child, _job) = crate::plugins::supervisor::windows_job::Job::spawn_tokio(
        &mut command,
        windows_sys::Win32::System::Threading::CREATE_NO_WINDOW,
    )
    .map_err(|_| anyhow!("Could not start Codex with owned process supervision"))?;
    #[cfg(not(windows))]
    let mut child = command
        .spawn()
        .map_err(|_| anyhow!("Could not start Codex to read routing configuration"))?;
    #[cfg(unix)]
    let mut group = ProcessGroup(
        child
            .id()
            .ok_or_else(|| anyhow!("Codex probe process id unavailable"))?,
    );
    let mut stdin = child
        .stdin
        .take()
        .ok_or_else(|| anyhow!("Codex stdin unavailable"))?;
    let stdout = child
        .stdout
        .take()
        .ok_or_else(|| anyhow!("Codex stdout unavailable"))?;
    let result=tokio::time::timeout(Duration::from_secs(8),async{
        async fn send(stdin:&mut tokio::process::ChildStdin,value:Value)->Result<()>{let mut bytes=serde_json::to_vec(&value)?;bytes.push(b'\n');stdin.write_all(&bytes).await.map_err(|_|anyhow!("Could not communicate with Codex"))?;Ok(())}
        send(&mut stdin,json!({"id":1,"method":"initialize","params":{"clientInfo":{"name":"workspacer_launch_routing","version":"1.0.0"}}})).await?;
        let mut lines=BufReader::new(stdout.take(16<<20)).lines();
        while let Some(line)=lines.next_line().await?{let Ok(message)=serde_json::from_str::<Value>(&line)else{continue};if message["id"]!=1&&message["id"]!=2{continue}if message.get("error").is_some_and(|error|!error.is_null()){bail!("Codex could not read configuration; check it with codex app-server")}
            if message["id"]==1{send(&mut stdin,json!({"method":"initialized","params":{}})).await?;send(&mut stdin,json!({"id":2,"method":"config/read","params":{"includeLayers":false,"cwd":cwd}})).await?;}
            else{return provider_from_config(&message["result"]["config"],&env,std::env::var("OPENAI_BASE_URL").ok().as_deref())}
        }bail!("Codex exited before returning routing configuration")
    }).await.map_err(|_|anyhow!("Timed out reading Codex routing configuration"));
    drop(stdin);
    #[cfg(unix)]
    group.kill();
    let _ = child.start_kill();
    let _ = child.wait().await;
    result?
}

#[cfg(unix)]
struct ProcessGroup(u32);
#[cfg(unix)]
impl ProcessGroup {
    fn kill(&mut self) {
        if self.0 != 0 {
            unsafe {
                libc::kill(-(self.0 as libc::pid_t), libc::SIGKILL);
            }
            self.0 = 0;
        }
    }
}
#[cfg(unix)]
impl Drop for ProcessGroup {
    fn drop(&mut self) {
        self.kill();
    }
}
