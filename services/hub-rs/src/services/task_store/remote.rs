//! Projection of verified paired lifecycle receipts into local task history.
//! Admission and task/result authority stay local; this API cannot create work.
use super::*;
impl TaskStore {
    pub fn observe_remote(
        &self,
        session: &str,
        lifecycle: &str,
        final_observation: bool,
    ) -> Result<()> {
        if session.is_empty()
            || session.len() > 256
            || !matches!(
                lifecycle,
                "starting" | "running" | "idle" | "needs-decision" | "ended"
            )
        {
            bail!("invalid paired lifecycle observation");
        }
        let key = format!("paired:{session}");
        let reading = json!({"lifecycle":lifecycle,"final":final_observation});
        let mut observed = self.observations.lock().unwrap();
        if observed.get(&key) == Some(&reading) {
            return Ok(());
        }
        let at = self.transaction(|history| {
            let at = now();
            let mut touched = false;
            for task in &mut history.tasks {
                let Some(attempt) = task["attempts"]
                    .as_array_mut()
                    .into_iter()
                    .flatten()
                    .find(|a| a["sessionId"] == session && a["executionTarget"] == "paired")
                else {
                    continue;
                };
                attempt["observedAt"] = at.clone().into();
                attempt["stale"] = false.into();
                attempt["live"] = (lifecycle != "ended").into();
                attempt["lifecycle"] = lifecycle.into();
                if lifecycle == "ended" {
                    attempt["endedAt"] = at.clone().into();
                }
                if let Ok(start) =
                    chrono::DateTime::parse_from_rfc3339(text(&attempt["acceptedAt"]))
                {
                    if !attempt["metrics"].is_object() {
                        attempt["metrics"] = json!({});
                    }
                    attempt["metrics"]["wallMs"] = chrono::Utc::now()
                        .signed_duration_since(start)
                        .num_milliseconds()
                        .max(0)
                        .into();
                }
                let absent = attempt["resultContract"] == "absent";
                if !final_observation && absent {
                    for step in task
                        .get_mut("workflow")
                        .and_then(|w| w.get_mut("steps"))
                        .and_then(Value::as_array_mut)
                        .into_iter()
                        .flatten()
                        .filter(|s| s["sessionId"] == session && s["state"] != "waived")
                    {
                        step["state"] = match lifecycle {
                            "needs-decision" => "blocked",
                            "ended" => "failed",
                            _ => "dispatched",
                        }
                        .into();
                    }
                }
                touched = true;
            }
            anyhow::ensure!(touched, "No admitted paired attempt for this observation");
            Ok(at)
        })?;
        self.fresh.lock().unwrap().insert(session.into(), at);
        observed.insert(key, reading);
        Ok(())
    }
}
