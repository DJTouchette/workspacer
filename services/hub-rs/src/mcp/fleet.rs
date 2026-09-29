use crate::client::Client;
use serde_json::Value;
use std::time::Duration;

pub(super) async fn merge(client: &Client, method: &str, params: Value, local: Value) -> Value {
    if !local.is_array() && !local.is_null() {
        return local;
    }
    let Ok(peers) = client
        .call_with_timeout("federation.peers", Value::Null, Duration::from_secs(10))
        .await
    else {
        return local;
    };
    let Some(peers) = peers.as_array() else {
        return local;
    };
    let names: Vec<_> = peers
        .iter()
        .filter(|peer| peer["connected"] == true)
        .filter_map(|peer| peer["name"].as_str().filter(|name| !name.is_empty()))
        .collect();
    if names.is_empty() {
        return local;
    }
    let results = futures_util::future::join_all(names.iter().map(|name| {
        let params = params.clone();
        async move {
            client
                .call_with_timeout(
                    &format!("hub:{name}/{method}"),
                    params,
                    Duration::from_secs(10),
                )
                .await
        }
    }))
    .await;
    let mut merged = local.as_array().cloned().unwrap_or_default();
    for (name, result) in names.into_iter().zip(results) {
        let Ok(Value::Array(rows)) = result else {
            continue;
        };
        if !rows.iter().all(Value::is_object) {
            continue;
        }
        merged.extend(rows.into_iter().map(|mut row| {
            row["hub"] = name.into();
            row
        }));
    }
    Value::Array(merged)
}
