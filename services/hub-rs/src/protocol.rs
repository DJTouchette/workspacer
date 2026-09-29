//! The existing hub wire vocabulary. Embedded consumers use these types without
//! serializing them or opening sockets.
use serde::{Deserialize, Serialize};
use serde_json::Value;

/// Opt-in broker-to-provider metadata. Ordinary incoming frames never confer
/// this authority; the broker replaces it from its authenticated connection.
#[derive(Clone,Debug,Deserialize,Serialize,PartialEq,Eq,Hash)]
#[serde(rename_all="camelCase",deny_unknown_fields)]
pub struct ProviderCaller {
    pub version:u32,
    pub connection_id:u64,
    pub scope:String,
    pub authenticated_host:bool,
    pub federated:bool,
    pub plugin_id:String,
    pub token_id:String,
    pub may_assert_session:bool,
}
fn zero(value:&u32)->bool{*value==0}

#[derive(Clone, Debug, Default, Deserialize, Serialize, PartialEq)]
#[serde(default)]
pub struct Event {
    pub id: String,
    #[serde(rename = "type")]
    pub topic: String,
    pub source: String,
    #[serde(skip_serializing_if = "String::is_empty")]
    pub hub: String,
    #[serde(
        deserialize_with = "event_time",
        serialize_with = "serialize_event_time"
    )]
    pub time: String,
    #[serde(skip_serializing_if = "Option::is_none", deserialize_with = "present")]
    pub data: Option<Value>,
}

impl Event {
    pub fn new(topic: impl Into<String>, source: impl Into<String>, data: Value) -> Self {
        Self {
            topic: topic.into(),
            source: source.into(),
            data: Some(data),
            ..Self::default()
        }
    }
}

#[derive(Clone, Debug, Default, Deserialize, Serialize, PartialEq)]
#[serde(default, rename_all = "camelCase")]
pub struct Frame {
    pub op: String,
    #[serde(skip_serializing_if="std::ops::Not::not")]
    pub wants_caller_context:bool,
    #[serde(skip_serializing_if="zero")]
    pub caller_context_version:u32,
    #[serde(skip_serializing_if="Option::is_none")]
    pub provider_caller:Option<ProviderCaller>,
    #[serde(skip_serializing_if = "std::ops::Not::not")]
    pub spawn_full_access: bool,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub topics: Vec<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub event: Option<Event>,
    #[serde(skip_serializing_if = "String::is_empty")]
    pub topic: String,
    #[serde(skip_serializing_if = "std::ops::Not::not")]
    pub demand: bool,
    #[serde(skip_serializing_if = "String::is_empty")]
    pub id: String,
    #[serde(skip_serializing_if = "String::is_empty")]
    pub method: String,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub methods: Vec<String>,
    #[serde(skip_serializing_if = "Option::is_none", deserialize_with = "present")]
    pub params: Option<Value>,
    #[serde(skip_serializing_if = "Option::is_none", deserialize_with = "present")]
    pub result: Option<Value>,
    #[serde(skip_serializing_if = "String::is_empty")]
    pub error: String,
    #[serde(skip_serializing_if = "String::is_empty")]
    pub scope: String,
}

impl Frame {
    /// Validate authority-bearing preferences before JSON object decoding can
    /// erase duplicate keys. In-memory Frames cannot represent duplicates.
    pub fn decode(bytes: &[u8]) -> anyhow::Result<Self> {
        #[derive(Deserialize)]
        struct Raw<'a> {
            #[serde(default)]
            op: String,
            #[serde(default)]
            method: String,
            #[serde(default, borrow, deserialize_with = "raw_present")]
            params: Option<&'a serde_json::value::RawValue>,
        }
        let raw: Raw<'_> = serde_json::from_slice(bytes)?;
        let bare = if raw.method.starts_with("hub:") {
            raw.method
                .split_once('/')
                .map(|(_, bare)| bare)
                .unwrap_or(&raw.method)
        } else {
            &raw.method
        };
        if raw.op == "call"
            && matches!(
                bare,
                "routing.preferences.validate"
                    | "routing.preferences.save"
                    | "routing.preferences.reset"
            )
        {
            crate::services::routing::validate_preferences_raw(
                raw.params
                    .map(|params| params.get().as_bytes())
                    .unwrap_or(b"{}"),
            )?;
        }
        Ok(serde_json::from_slice(bytes)?)
    }
    pub fn op(op: &str) -> Self {
        Self {
            op: op.into(),
            ..Self::default()
        }
    }
    pub fn error(id: impl Into<String>, error: impl Into<String>) -> Self {
        Self {
            id: id.into(),
            error: error.into(),
            ..Self::op("error")
        }
    }
}

pub(crate) fn raw_present<'de, D: serde::Deserializer<'de>>(
    deserializer: D,
) -> Result<Option<&'de serde_json::value::RawValue>, D::Error> {
    <&'de serde_json::value::RawValue>::deserialize(deserializer).map(Some)
}

pub fn matches(pattern: &str, topic: &str) -> bool {
    pattern == "*"
        || pattern == topic
        || (pattern.ends_with(".*") && topic.starts_with(&pattern[..pattern.len() - 1]))
}

/// Server-owned budgets; a client cannot extend a method's execution window.
pub fn provider_timeout(method: &str, base: std::time::Duration) -> std::time::Duration {
    use std::time::Duration;
    if base < Duration::from_secs(30) {
        return base;
    }
    let method = if method.starts_with("hub:") {
        method
            .split_once('/')
            .map(|(_, bare)| bare)
            .unwrap_or(method)
    } else {
        method
    };
    let seconds = match method {
        "agents.spawn" | "desktop.worktreeCreate" => 360,
        "claude.handoffAgentBrief" => 180,
        "desktop.managerRequestSend" | "desktop.worktreeRemove" => 60,
        _ => 0,
    };
    base.max(Duration::from_secs(seconds))
}

fn present<'de, D: serde::Deserializer<'de>>(deserializer: D) -> Result<Option<Value>, D::Error> {
    Value::deserialize(deserializer).map(Some)
}

fn event_time<'de, D: serde::Deserializer<'de>>(deserializer: D) -> Result<String, D::Error> {
    let Some(raw) = Option::<String>::deserialize(deserializer)? else {
        return Ok(String::new());
    };
    let parsed = time::OffsetDateTime::parse(&raw, &time::format_description::well_known::Rfc3339)
        .map_err(serde::de::Error::custom)?;
    parsed
        .format(&time::format_description::well_known::Rfc3339)
        .map_err(serde::de::Error::custom)
}
fn serialize_event_time<S: serde::Serializer>(
    value: &str,
    serializer: S,
) -> Result<S::Ok, S::Error> {
    serializer.serialize_str(if value.is_empty() {
        "0001-01-01T00:00:00Z"
    } else {
        value
    })
}

pub(crate) fn now() -> String {
    time::OffsetDateTime::now_utc()
        .format(&time::format_description::well_known::Rfc3339)
        .expect("UTC time formats as RFC3339")
}
