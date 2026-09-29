//! Lifecycle-bound facade credentials and provider injection. Configured facade
//! readiness is verified for every launch; a migration-only listener is refused.
use super::{
    agent_lifecycle::{LaunchPreparation, Operation},
    atomic_json,
    spawn_plan::Plan,
};
use crate::auth;
use anyhow::{Result, bail};
use base64::Engine;
use rand::RngCore;
use serde_json::{Value, json};
use std::{path::PathBuf, time::Duration};

pub enum Readiness {
    /// The owning host deliberately has no Workspacer facade listener.
    Disabled,
    /// Compatibility with a separately owned production facade.
    Legacy,
    /// An owned Rust listener, proven through its in-process actor rather than
    /// trusting an HTTP responder to assert migration/launch capabilities.
    OwnedRust(crate::Handle),
}
pub struct SessionFacade {
    pub endpoint: Option<url::Url>,
    pub expected_hub: String,
    pub readiness: Readiness,
    pub tokens: PathBuf,
    pub directory: PathBuf,
    pub home: PathBuf,
    /// Additional trusted launch contract. Built-in skill pointers and manager
    /// doctrine are composed per session, never inferred from permission grants.
    pub instructions: String,
}
impl SessionFacade {
    async fn ready(&self) -> Result<()> {
        let endpoint = self
            .endpoint
            .as_ref()
            .ok_or_else(|| anyhow::anyhow!("configured facade has no endpoint"))?;
        if !matches!(endpoint.scheme(), "http" | "https")
            || endpoint.host_str().is_none()
            || !endpoint.username().is_empty()
            || endpoint.password().is_some()
        {
            bail!("invalid facade endpoint");
        }
        if let Readiness::OwnedRust(hub) = &self.readiness {
            let status = hub.status().borrow().clone();
            let crate::Status::Ready {
                mcp_address: Some(address),
                ..
            } = status
            else {
                bail!("owned facade runtime is not ready");
            };
            let ip = match endpoint.host() {
                Some(url::Host::Ipv4(ip)) => std::net::IpAddr::V4(ip),
                Some(url::Host::Ipv6(ip)) => std::net::IpAddr::V6(ip),
                _ => bail!("owned facade requires its bound IP address"),
            };
            if endpoint.scheme() != "http"
                || std::net::SocketAddr::new(ip, endpoint.port_or_known_default().unwrap_or(0))
                    != crate::net_address::dial_addr(address)
                || endpoint.path() != "/mcp"
                || endpoint.query().is_some()
                || endpoint.fragment().is_some()
            {
                bail!("owned facade endpoint differs from its bound listener");
            }
            let health = tokio::time::timeout(Duration::from_millis(1200), hub.health())
                .await
                .map_err(|_| anyhow::anyhow!("owned facade readiness timed out"))??;
            if health["status"] != "ok" || health["launchReady"] != true {
                bail!("owned facade launch capabilities are not ready");
            }
            return Ok(());
        }
        let mut url = endpoint.clone();
        url.set_path("/health");
        url.set_query(None);
        url.set_fragment(None);
        let client = reqwest::Client::builder()
            .timeout(Duration::from_millis(1200))
            .redirect(reqwest::redirect::Policy::none())
            .build()?;
        // Readiness is a public, nonsecret endpoint. Never disclose the host
        // bearer to a listener whose identity has not yet been established.
        let response = client.get(url).send().await?.error_for_status()?;
        let health: Value = response.json().await?;
        let listen = &endpoint[url::Position::BeforeHost..url::Position::AfterPort];
        if health["status"] != "ok"
            || health["service"] != "workspacer-mcp-facade"
            || health["hubConnected"] != true
            || health["pluginCatalogReady"] != true
            || health["listenAddr"] != listen
            || (!self.expected_hub.is_empty() && health["hubUrl"] != self.expected_hub)
        {
            bail!("configured facade is not ready or does not match the expected hub");
        }
        Ok(())
    }
    fn mint(&self, plan: &Plan, generation: &str) -> Result<String> {
        let label = format!("session:{}", plan.session_id);
        let mut bytes = [0; 24];
        rand::rngs::OsRng
            .try_fill_bytes(&mut bytes)
            .map_err(|_| anyhow::anyhow!("credential random source unavailable"))?;
        let token = base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(bytes);
        auth::update_records(&self.tokens, |records| {
            records.retain(|r| r.label != label);
            let mut record = auth::Record {
                token: token.clone(),
                scope: "operator".into(),
                label,
                created: crate::protocol::now(),
                ..Default::default()
            };
            record
                .metadata
                .insert("generation".into(), json!(generation));
            record.metadata.insert("plugins".into(), json!(["*"]));
            if plan.metadata["isWakeTarget"] == true {
                record.metadata.insert("role".into(), json!("manager"));
            }
            records.push(record);
            Ok(())
        })?;
        Ok(token)
    }
}
impl LaunchPreparation for SessionFacade {
    fn sweep<'a>(&'a self, live: &'a std::collections::BTreeSet<String>) -> Operation<'a, ()> {
        Box::pin(async move {
            if matches!(self.readiness, Readiness::Disabled) && !self.tokens.try_exists()? {
                return Ok(());
            }
            auth::update_records(&self.tokens, |records| {
                records.retain(|record| {
                    record
                        .label
                        .strip_prefix("session:")
                        .is_none_or(|id| live.contains(id))
                });
                Ok(())
            })
        })
    }
    fn prepare<'a>(&'a self, plan: &'a mut Plan, generation: &'a str) -> Operation<'a, ()> {
        Box::pin(async move {
            if plan.session_id.is_empty()
                || matches!(plan.session_id.as_str(), "." | "..")
                || !plan
                    .session_id
                    .bytes()
                    .all(|b| b.is_ascii_alphanumeric() || b"-_.".contains(&b))
            {
                bail!("invalid session identifier");
            }
            if generation.is_empty() {
                bail!("launch generation is required");
            }
            let enabled = !matches!(self.readiness, Readiness::Disabled);
            if enabled {
                self.ready().await?;
            }
            let mut instructions = if enabled {
                super::launch_instructions::instructions(
                    &plan.session_id,
                    &plan.provider,
                    std::path::Path::new(
                        plan.request["cwd"]
                            .as_str()
                            .ok_or_else(|| anyhow::anyhow!("launch requires cwd"))?,
                    ),
                    &self.home,
                    plan.metadata["isWakeTarget"] == true,
                )
            } else {
                String::new()
            };
            if !self.instructions.trim().is_empty() {
                if !instructions.is_empty() {
                    instructions.push_str("\n\n");
                }
                instructions.push_str(&self.instructions);
            }
            let library = super::library::Library::new(
                self.directory
                    .parent()
                    .ok_or_else(|| anyhow::anyhow!("session MCP directory has no config parent"))?
                    .to_owned(),
            );
            let selected = library.selected_mcp(
                std::path::Path::new(plan.request["cwd"].as_str().unwrap()),
                &plan.mcp_item_ids,
            )?;
            let token = if enabled {
                Some(self.mint(plan, generation)?)
            } else {
                None
            };
            if plan.provider == "claude" {
                let path = self.directory.join(format!("{}.json", plan.session_id));
                let mut servers = json!({});
                let mut allowed = vec![];
                if let Some(token) = &token {
                    servers["workspacer"] = json!({"type":"http","url":self.endpoint.as_ref().unwrap().as_str(),"headers":{"Authorization":format!("Bearer {token}")}});
                    allowed.push("mcp__workspacer".to_string());
                }
                for (id, config) in &selected {
                    servers[id] = config.clone();
                    allowed.push(format!("mcp__{id}"));
                }
                let key = if plan.endpoint == "/sessions/spawn" {
                    "argv"
                } else {
                    "extra_args"
                };
                if plan.request.get(key).is_none() {
                    plan.request[key] = json!([]);
                }
                let argv = plan.request[key]
                    .as_array_mut()
                    .ok_or_else(|| anyhow::anyhow!("invalid provider arguments"))?;
                if !allowed.is_empty() {
                    atomic_json(&path, &json!({"mcpServers":servers}), true)?;
                    argv.extend([
                        json!("--mcp-config"),
                        json!(path),
                        json!("--allowedTools"),
                        json!(allowed.join(",")),
                    ]);
                }
                if !instructions.is_empty() {
                    argv.extend([json!("--append-system-prompt"), json!(instructions)]);
                }
                if !selected.is_empty() {
                    argv.push(json!("--strict-mcp-config"));
                }
                compose_instructions(argv)?;
            } else if let Some(token) = &token {
                let mut endpoint = self.endpoint.as_ref().unwrap().clone();
                let pairs: Vec<_> = endpoint
                    .query_pairs()
                    .filter(|(k, _)| k != "t")
                    .map(|(k, v)| (k.into_owned(), v.into_owned()))
                    .collect();
                endpoint.set_query(None);
                endpoint
                    .query_pairs_mut()
                    .extend_pairs(pairs)
                    .append_pair("t", token);
                plan.request["mcp"] = json!(endpoint.as_str());
            }
            if plan.endpoint == "/sessions/spawn-managed" && !instructions.is_empty() {
                let existing = plan.request["instructions"].as_str().unwrap_or("");
                plan.request["instructions"] = json!(if existing.is_empty() {
                    instructions
                } else {
                    format!("{instructions}\n\n{existing}")
                });
            }
            Ok(())
        })
    }
    fn revoke<'a>(&'a self, session: &'a str, generation: &'a str) -> Operation<'a, ()> {
        Box::pin(async move {
            if matches!(self.readiness, Readiness::Disabled) && !self.tokens.try_exists()? {
                return Ok(());
            }
            let label = format!("session:{session}");
            auth::update_records(&self.tokens, |records| {
                records.retain(|r| {
                    !(r.label == label
                        && r.metadata
                            .get("generation")
                            .is_some_and(|g| g == generation))
                });
                Ok(())
            })?;
            // A config may belong to a successor generation. Credentials are
            // revoked by identity; artifact sweeping is a separate operation.
            Ok(())
        })
    }
}
/// Claude consumes one additive prompt flag. Preserve all existing fragments in
/// order instead of allowing the final facade flag to erase a profile's prompt.
pub fn compose_instructions(argv: &mut Vec<Value>) -> Result<()> {
    let mut output = Vec::new();
    let mut prompts = Vec::new();
    let mut index = 0;
    while index < argv.len() {
        let arg = argv[index]
            .as_str()
            .ok_or_else(|| anyhow::anyhow!("provider argument must be a string"))?;
        if arg == "--append-system-prompt" {
            index += 1;
            prompts.push(
                argv.get(index)
                    .and_then(Value::as_str)
                    .ok_or_else(|| anyhow::anyhow!("append-system-prompt requires text"))?
                    .to_owned(),
            );
        } else if let Some(value) = arg.strip_prefix("--append-system-prompt=") {
            prompts.push(value.into());
        } else {
            output.push(argv[index].clone());
        }
        index += 1;
    }
    if !prompts.is_empty() {
        output.extend([json!("--append-system-prompt"), json!(prompts.join("\n\n"))]);
    }
    *argv = output;
    Ok(())
}
