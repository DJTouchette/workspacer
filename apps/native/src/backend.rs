//! Shared native backend boundary. Hub projections and launch setup stay intact;
//! an owned local engine can execute session controls through in-process channels.
use crate::{
    bus::{Client, Config, Event},
    controller::Action,
    launch::CatalogKey,
};
use anyhow::Result;
use serde_json::{Value, json};
use std::{collections::BTreeSet, time::Duration};

#[derive(Clone)]
pub struct Backend {
    hub: Client,
    #[cfg(feature = "embedded")]
    local: Option<claudemon::daemon::embedded::EmbeddedClient>,
}

impl Backend {
    pub async fn call(&self, method: &str, params: Value) -> Result<Value> {
        let value = self
            .hub
            .call_with_timeout(method, params, Duration::from_secs(90))
            .await?;
        anyhow::ensure!(
            value.get("ok") != Some(&Value::Bool(false)),
            "{}",
            value["error"].as_str().unwrap_or("Request was refused")
        );
        Ok(value)
    }
    pub fn connect(config: Config) -> (Self, async_channel::Receiver<Event>) {
        let (hub, events) = Client::start(config);
        (
            Self {
                hub,
                #[cfg(feature = "embedded")]
                local: None,
            },
            events,
        )
    }

    #[cfg(feature = "embedded")]
    pub fn with_local(mut self, client: claudemon::daemon::embedded::EmbeddedClient) -> Self {
        self.local = Some(client);
        self
    }

    pub async fn snapshots(&self) -> Result<Value> {
        self.hub.call("sessions.snapshots", json!({})).await
    }

    pub async fn conversation(&self, id: &str) -> Result<Value> {
        self.hub
            .call("sessions.conversation", json!({"sessionId":id}))
            .await
    }

    pub async fn models(&self, key: &CatalogKey) -> Result<Value> {
        let (method, params) = if key.provider == "claude" {
            ("claude.listModels", json!({}))
        } else {
            (
                "providers.listModels",
                json!({"provider":key.provider,"cwd":key.cwd}),
            )
        };
        self.hub
            .call_with_timeout(method, params, Duration::from_secs(60))
            .await
    }

    pub async fn spawn(&self, params: Value) -> Result<Value> {
        // Never bypass facade injection, worktrees, skills, or owner registration.
        self.hub
            .call_with_timeout("agents.spawn", params, Duration::from_secs(360))
            .await
    }

    pub async fn topics(&self, topics: BTreeSet<String>) -> Result<()> {
        self.hub.topics(topics).await
    }

    pub async fn action(&self, id: &str, action: &Action, _stream: bool) -> Result<Value> {
        #[cfg(feature = "embedded")]
        if let Some(local) = &self.local {
            use claudemon::daemon::embedded::Command;
            let command = match action {
                Action::Send(text) => Some(Command::Message {
                    id: id.into(),
                    text: text.clone(),
                }),
                Action::Approve(yes) => Some(Command::Approve {
                    id: id.into(),
                    decision: if *yes { "yes" } else { "no" }.into(),
                }),
                Action::Stop => Some(Command::Interrupt { id: id.into() }),
                Action::Answer(text) if _stream => Some(Command::Answer {
                    id: id.into(),
                    answer: json!({"answers":[text],"answerKinds":["text"]}),
                }),
                Action::Answers(answers) if _stream => Some(Command::Answer {
                    id: id.into(),
                    answer: json!({"answers":answers,"answerKinds":vec!["text"; answers.len()]}),
                }),
                // PTY answers require the existing provider's keystroke path.
                Action::Answer(_) => None,
                _ => None,
            };
            if let Some(command) = command {
                return local.request(command).await;
            }
        }
        let (method, params) = action.wire(id);
        self.call(method, params).await
    }
}
