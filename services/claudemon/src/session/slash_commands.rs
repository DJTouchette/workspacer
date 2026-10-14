//! The commands a session's composer can offer when the user types `/`:
//! Claude's `system/init.slash_commands` (described from its skill
//! inventory where the names match) and Codex's `skills/list`.
//!
//! Each entry carries the exact text a client inserts. Claude invokes a
//! command or skill as `/name`; Codex mentions a skill as `$name` in the
//! message text (its tool-mention sigil) and has no text slash commands over
//! the app-server, so a Codex entry inserts `$name`.
use super::state::ContextInventory;
use serde::{Deserialize, Serialize};
use serde_json::Value;

/// Most entries kept per session; a menu longer than this is not browsed.
pub const MAX_COMMANDS: usize = 400;
const MAX_NAME: usize = 128;
const MAX_DESCRIPTION: usize = 300;

/// JSON-RPC id of the Codex `skills/list` request. A string, so it never
/// collides with the numeric ids the bootstrap and user turns count up.
pub const CODEX_SKILLS_RPC_ID: &str = "workspacer-skills-list";

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct SlashCommand {
    /// The bare name (`compact`, `code-review`, `plugin:skill`).
    pub name: String,
    /// What the composer inserts on selection (`/compact`, `$review-pr`).
    pub insert: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    /// `"command"` or `"skill"`.
    pub kind: String,
}

fn clip(text: &str, max: usize) -> String {
    let line = text.lines().next().unwrap_or("").trim();
    if line.len() <= max {
        return line.to_owned();
    }
    let mut end = max;
    while !line.is_char_boundary(end) {
        end -= 1;
    }
    format!("{}…", &line[..end])
}

/// A name a composer can insert: one token, no whitespace or control text.
fn usable(name: &str) -> bool {
    !name.is_empty()
        && name.len() <= MAX_NAME
        && !name.starts_with(['/', '$'])
        && !name.chars().any(|c| c.is_whitespace() || c.is_control())
}

fn push(
    out: &mut Vec<SlashCommand>,
    name: &str,
    sigil: char,
    kind: &str,
    description: Option<&str>,
) {
    if out.len() >= MAX_COMMANDS || !usable(name) || out.iter().any(|c| c.name == name) {
        return;
    }
    out.push(SlashCommand {
        name: name.to_owned(),
        insert: format!("{sigil}{name}"),
        description: description
            .map(|d| clip(d, MAX_DESCRIPTION))
            .filter(|d| !d.is_empty()),
        kind: kind.to_owned(),
    });
}

/// Claude: the init frame's command names, then any skill it did not list
/// (a skill is invoked by the same `/name`). Descriptions come from the
/// skill inventory's frontmatter when a name matches.
pub fn from_claude_inventory(inventory: Option<&ContextInventory>) -> Vec<SlashCommand> {
    let Some(inventory) = inventory else {
        return Vec::new();
    };
    let skill = |name: &str| inventory.skills.iter().find(|s| s.name == name);
    let mut out = Vec::new();
    for name in &inventory.slash_commands {
        let found = skill(name);
        push(
            &mut out,
            name,
            '/',
            if found.is_some() { "skill" } else { "command" },
            found.and_then(|s| s.description.as_deref()),
        );
    }
    for item in &inventory.skills {
        push(
            &mut out,
            &item.name,
            '/',
            "skill",
            item.description.as_deref(),
        );
    }
    out
}

/// Codex: the enabled skills of a `skills/list` result
/// (`{data: [{cwd, skills: [{name, description, shortDescription?,
/// interface?: {shortDescription?}, enabled}]}]}`).
pub fn from_codex_skills(result: &Value) -> Vec<SlashCommand> {
    let mut out = Vec::new();
    let entries = result.get("data").and_then(Value::as_array);
    for skill in entries
        .into_iter()
        .flatten()
        .filter_map(|entry| entry.get("skills").and_then(Value::as_array))
        .flatten()
    {
        if skill.get("enabled").and_then(Value::as_bool) == Some(false) {
            continue;
        }
        let Some(name) = skill.get("name").and_then(Value::as_str) else {
            continue;
        };
        let description = skill
            .pointer("/interface/shortDescription")
            .or_else(|| skill.get("shortDescription"))
            .or_else(|| skill.get("description"))
            .and_then(Value::as_str);
        push(&mut out, name, '$', "skill", description);
    }
    out
}

/// The Codex request that lists this session's skills (`cwds` empty: the
/// app-server's own working directory, which is the session's).
pub fn codex_skills_request() -> Value {
    serde_json::json!({
        "jsonrpc": "2.0", "id": CODEX_SKILLS_RPC_ID, "method": "skills/list", "params": {}
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::session::state::ContextItem;
    use serde_json::json;

    #[test]
    fn claude_commands_take_skill_descriptions_and_list_unlisted_skills() {
        let inventory = ContextInventory {
            slash_commands: vec![
                "compact".into(),
                "code-review".into(),
                "compact".into(),
                "bad name".into(),
                "/slashed".into(),
            ],
            skills: vec![
                ContextItem {
                    name: "code-review".into(),
                    description: Some("Review the diff\nsecond line".into()),
                    ..Default::default()
                },
                ContextItem {
                    name: "dataviz".into(),
                    ..Default::default()
                },
            ],
            ..Default::default()
        };
        let got = from_claude_inventory(Some(&inventory));
        assert_eq!(
            serde_json::to_value(&got).unwrap(),
            json!([
                { "name": "compact", "insert": "/compact", "kind": "command" },
                { "name": "code-review", "insert": "/code-review",
                  "description": "Review the diff", "kind": "skill" },
                { "name": "dataviz", "insert": "/dataviz", "kind": "skill" }
            ])
        );
        assert!(from_claude_inventory(None).is_empty());
    }

    #[test]
    fn codex_skills_insert_their_mention_sigil_and_skip_disabled_ones() {
        let result = json!({ "data": [
            { "cwd": "/p", "errors": [], "skills": [
                { "name": "review-pr", "description": "Long", "enabled": true,
                  "interface": { "shortDescription": "Review a PR" } },
                { "name": "off", "description": "x", "enabled": false },
                { "name": "plain", "description": "Plain skill", "enabled": true }
            ] },
            { "cwd": "/q", "errors": [], "skills": [
                { "name": "review-pr", "description": "dup", "enabled": true }
            ] }
        ] });
        let got = from_codex_skills(&result);
        assert_eq!(
            got.iter()
                .map(|c| (c.insert.as_str(), c.description.as_deref()))
                .collect::<Vec<_>>(),
            vec![
                ("$review-pr", Some("Review a PR")),
                ("$plain", Some("Plain skill"))
            ]
        );
        assert!(from_codex_skills(&json!({})).is_empty());
        assert_eq!(codex_skills_request()["method"], "skills/list");
    }
}
