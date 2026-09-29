use crate::client::Client;
use anyhow::{Context, Result, bail};
use futures_util::{StreamExt, stream};
use serde_json::{Value, json};
fn row(dir: &str, value: Result<Value>) -> Value {
    let mut row = json!({"dir":dir,"dirty":false,"changedFiles":0});
    match value {
        Err(error) => row["error"] = error.to_string().into(),
        Ok(value) => {
            if !value.is_object()
                || value
                    .get("files")
                    .is_some_and(|value| !value.is_array() && !value.is_null())
                || value
                    .get("branch")
                    .is_some_and(|value| !value.is_string() && !value.is_null())
            {
                row["error"] = "git.status returned a shape this tool could not read".into();
                return row;
            }
            let count = value["files"].as_array().map(Vec::len).unwrap_or(0);
            row["changedFiles"] = json!(count);
            row["dirty"] = (count > 0).into();
            if let Some(branch) = value["branch"].as_str() {
                row["branch"] = branch.into();
            }
            if let Some(upstream) = value["upstream"]
                .as_str()
                .filter(|value| !value.trim().is_empty())
            {
                row["upstream"] = upstream.into();
                row["unpushed"] = json!(value["ahead"].as_i64().unwrap_or(0));
                row["behind"] = json!(value["behind"].as_i64().unwrap_or(0));
            }
        }
    }
    row
}
pub(super) async fn call(client: &Client, params: Value) -> Result<Value> {
    let peer = params["hub"].as_str().unwrap_or("");
    let route = |method: &str| {
        if peer.is_empty() {
            method.into()
        } else {
            format!("hub:{peer}/{method}")
        }
    };
    let mut dirs = Vec::new();
    if let Some(entries) = params.get("dirs").filter(|value| !value.is_null()) {
        let entries = entries.as_array().context("dirs must be an array")?;
        for dir in entries {
            let dir = dir.as_str().context("dirs must contain paths")?;
            if !dir.trim().is_empty() {
                dirs.push(dir.to_owned());
            }
        }
    }
    if dirs.is_empty() {
        let config = client
            .call(&route("config.get"), Value::Null)
            .await
            .context("project_status: could not read the config")?;
        dirs = config["projects"]
            .as_object()
            .into_iter()
            .flat_map(|projects| projects.keys())
            .filter(|dir| !dir.trim().is_empty())
            .cloned()
            .collect();
        dirs.sort();
    }
    if dirs.is_empty() {
        bail!(
            "project_status: no projects are configured (config `projects` is empty) and no dirs were given. Pass dirs explicitly, or add the repos in Settings."
        );
    }
    let method = route("git.status");
    let results: Vec<_> = stream::iter(dirs.into_iter().map(|dir| {
        let method = method.clone();
        async move {
            let value = client.call(&method, json!({"cwd":dir})).await;
            row(&dir, value)
        }
    }))
    .buffered(8)
    .collect()
    .await;
    Ok(json!({"projects":results}))
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn missing_upstream_and_partial_errors_are_not_zero_or_clean_claims() {
        assert_eq!(
            row("/repo", Ok(json!({"branch":null,"files":[{"path":"one"}]}))),
            json!({"dir":"/repo","dirty":true,"changedFiles":1})
        );
        let remote = row(
            "/repo",
            Ok(json!({"branch":"main","upstream":"origin/main","ahead":2,"behind":1,"files":[]})),
        );
        assert_eq!(remote["unpushed"], 2);
        assert_eq!(remote["behind"], 1);
        assert_eq!(remote["dirty"], false);
        assert_eq!(
            row("/broken", Err(anyhow::anyhow!("not a repository")))["error"],
            "not a repository"
        );
        assert!(row("/broken", Ok(json!({"files":"unknown"})))["error"].is_string());
    }
}
