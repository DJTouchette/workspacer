use super::*;
use rusqlite::{
    Connection, params_from_iter,
    types::{Value as Sql, ValueRef},
};
pub fn query(db: &Connection, sql: &str, params: Vec<Sql>) -> Result<Vec<Value>> {
    let mut statement = db.prepare(sql)?;
    let names: Vec<String> = statement
        .column_names()
        .iter()
        .map(|s| s.to_string())
        .collect();
    let rows = statement.query_map(params_from_iter(params), |row| {
        let mut out = json!({});
        for (i, name) in names.iter().enumerate() {
            out[name] = match row.get_ref(i)? {
                ValueRef::Null => Value::Null,
                ValueRef::Integer(v) => v.into(),
                ValueRef::Real(v) => json!(v),
                ValueRef::Text(v) => String::from_utf8_lossy(v).into_owned().into(),
                ValueRef::Blob(_) => Value::Null,
            };
        }
        Ok(out)
    })?;
    Ok(rows.collect::<rusqlite::Result<Vec<_>>>()?)
}
fn filter(provider: Option<&str>, since: Option<&str>, alias: &str) -> (String, Vec<Sql>) {
    let mut clauses = Vec::new();
    let mut params = Vec::new();
    if let Some(p) = provider.filter(|p| !p.is_empty()) {
        if p == "claude" {
            clauses.push(format!("({alias}provider='' OR {alias}provider='claude')"));
        } else {
            clauses.push(format!("{alias}provider=?"));
            params.push(p.to_string().into());
        }
    }
    if let Some(s) = since.filter(|s| !s.is_empty()) {
        clauses.push(format!("{alias}started_at>=?"));
        params.push(s.to_string().into());
    }
    (
        if clauses.is_empty() {
            String::new()
        } else {
            format!("WHERE {}", clauses.join(" AND "))
        },
        params,
    )
}
pub fn read(db: &Connection, method: &str, input: &Value, now_ms: i64) -> Result<Value> {
    let provider = input["provider"].as_str();
    let since = input["since"]
        .as_str()
        .map(|s| iso(&s.into()))
        .filter(|s| !s.is_empty());
    if input
        .get("since")
        .is_some_and(|v| !v.is_null() && v != &json!("") && v != &json!(false))
        && since.is_none()
    {
        bail!("Invalid analytics time range");
    }
    let (where_, params) = filter(provider, since.as_deref(), "");
    if method == "analytics.recent" {
        let limit = input["limit"]
            .as_f64()
            .map(|n| n.floor().clamp(1., 10000.) as i64)
            .unwrap_or(100);
        let mut params = params;
        params.push(limit.into());
        return Ok(query(db,&format!("SELECT session_id AS sessionId,cwd,agent_name AS agentName,provider,model,git_branch AS gitBranch,started_at AS startedAt,ended_at AS endedAt,duration_ms AS durationMs,input_tokens AS inputTokens,output_tokens AS outputTokens,cost_usd AS costUSD,peak_context AS peakContext,tool_calls AS toolCalls,message_count AS messageCount,subagent_count AS subagentCount,workflow_runs AS workflowRuns,workflow_failed AS workflowFailed,status FROM session_history {where_} ORDER BY (CASE WHEN started_at='' THEN updated_at ELSE started_at END) DESC LIMIT ?"),params)?.into());
    }
    let totals=query(db,&format!("SELECT COUNT(*) AS sessions,COALESCE(SUM(cost_usd),0) AS costUSD,COALESCE(SUM(input_tokens),0) AS inputTokens,COALESCE(SUM(output_tokens),0) AS outputTokens,COALESCE(SUM(tool_calls),0) AS toolCalls,COALESCE(SUM(duration_ms),0) AS durationMs,COALESCE(SUM(workflow_runs),0) AS workflowRuns,COALESCE(SUM(CASE WHEN COALESCE(cost_usd,0)=0 AND COALESCE(input_tokens,0)=0 AND COALESCE(output_tokens,0)=0 THEN 1 ELSE 0 END),0) AS unrecordedSessions FROM session_history {where_}"),params.clone())?.remove(0);
    let day_limit = since
        .as_deref()
        .and_then(|s| chrono::DateTime::parse_from_rfc3339(s).ok())
        .map(|t| {
            ((now_ms - t.timestamp_millis()) as f64 / 86_400_000.)
                .ceil()
                .max(1.) as i64
        })
        .unwrap_or(90);
    let and = if where_.is_empty() {
        "WHERE".into()
    } else {
        format!("{where_} AND")
    };
    let mut day_params = params.clone();
    day_params.push(day_limit.into());
    let mut days = query(
        db,
        &format!(
            "SELECT substr(started_at,1,10) AS key,COUNT(*) AS sessions,COALESCE(SUM(cost_usd),0) AS costUSD,COALESCE(SUM(input_tokens+output_tokens),0) AS tokens FROM session_history {and} started_at!='' GROUP BY key ORDER BY key DESC LIMIT ?"
        ),
        day_params,
    )?;
    days.reverse();
    let projects = query(
        db,
        &format!(
            "SELECT cwd AS key,COUNT(*) AS sessions,COALESCE(SUM(cost_usd),0) AS costUSD,COALESCE(SUM(input_tokens+output_tokens),0) AS tokens FROM session_history {where_} GROUP BY cwd ORDER BY costUSD DESC LIMIT 12"
        ),
        params,
    )?;
    let (aliased, mut model_params) = filter(provider, since.as_deref(), "sh.");
    let and = if aliased.is_empty() {
        "WHERE".into()
    } else {
        format!("{aliased} AND")
    };
    model_params.extend(model_params.clone());
    let models = query(
        db,
        &format!(
            "SELECT CASE WHEN model='' THEN '(unknown)' ELSE model END AS key,COUNT(DISTINCT session_id) AS sessions,COALESCE(SUM(cost_usd),0) AS costUSD,COALESCE(SUM(tokens),0) AS tokens FROM (SELECT smu.session_id,smu.model,smu.cost_usd,smu.input_tokens+smu.output_tokens AS tokens FROM session_model_usage smu JOIN session_history sh ON sh.session_id=smu.session_id {aliased} UNION ALL SELECT sh.session_id,sh.model,sh.cost_usd,sh.input_tokens+sh.output_tokens AS tokens FROM session_history sh {and} NOT EXISTS(SELECT 1 FROM session_model_usage smu WHERE smu.session_id=sh.session_id)) GROUP BY key ORDER BY costUSD DESC LIMIT 12"
        ),
        model_params,
    )?;
    let (time, params) = filter(None, since.as_deref(), "");
    let providers = query(
        db,
        &format!(
            "SELECT CASE WHEN provider='' THEN 'claude' ELSE provider END AS key,COUNT(*) AS sessions,COALESCE(SUM(cost_usd),0) AS costUSD,COALESCE(SUM(input_tokens+output_tokens),0) AS tokens FROM session_history {time} GROUP BY key ORDER BY costUSD DESC"
        ),
        params,
    )?;
    Ok(
        json!({"totals":totals,"byDay":days,"byProject":projects,"byModel":models,"byProvider":providers}),
    )
}
pub fn record(db: &Connection, row: &Value) -> Result<()> {
    const FIELDS: &[(&str, &str)] = &[
        ("session_id", "sessionId"),
        ("cwd", "cwd"),
        ("agent_name", "agentName"),
        ("provider", "provider"),
        ("model", "model"),
        ("git_branch", "gitBranch"),
        ("started_at", "startedAt"),
        ("ended_at", "endedAt"),
        ("duration_ms", "durationMs"),
        ("input_tokens", "inputTokens"),
        ("output_tokens", "outputTokens"),
        ("cost_usd", "costUSD"),
        ("peak_context", "peakContext"),
        ("tool_calls", "toolCalls"),
        ("message_count", "messageCount"),
        ("subagent_count", "subagentCount"),
        ("workflow_runs", "workflowRuns"),
        ("workflow_failed", "workflowFailed"),
        ("status", "status"),
        ("updated_at", "updatedAt"),
    ];
    let columns = FIELDS.iter().map(|f| f.0).collect::<Vec<_>>().join(",");
    let binds = vec!["?"; FIELDS.len()].join(",");
    let updates = FIELDS
        .iter()
        .filter(|f| !matches!(f.0, "session_id" | "started_at"))
        .map(|f| format!("{}=excluded.{}", f.0, f.0))
        .collect::<Vec<_>>()
        .join(",");
    let values = FIELDS.iter().map(|(_, key)| match &row[*key] {
        Value::String(s) => Sql::Text(s.clone()),
        Value::Number(n) => n
            .as_i64()
            .map(Sql::Integer)
            .unwrap_or_else(|| Sql::Real(n.as_f64().unwrap_or(0.))),
        _ => Sql::Text(String::new()),
    });
    db.execute(&format!("INSERT INTO session_history({columns}) VALUES({binds}) ON CONFLICT(session_id) DO UPDATE SET {updates}"),params_from_iter(values))?;
    Ok(())
}
