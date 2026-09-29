use rmcp::model::Tool;
use serde::Deserialize;
use std::{
    collections::{BTreeMap, BTreeSet},
    sync::OnceLock,
};

#[derive(Deserialize)]
struct Group {
    name: String,
    tools: Vec<String>,
}
#[derive(Deserialize)]
struct Reference {
    guidance: BTreeMap<String, String>,
    groups: BTreeMap<String, Vec<Group>>,
}
pub(super) fn render(scope: &str, tools: &[Tool], topic: &str) -> String {
    static REFERENCE: OnceLock<Reference> = OnceLock::new();
    let reference = REFERENCE.get_or_init(|| {
        serde_json::from_str(include_str!("../../assets/mcp-effective-help.json"))
            .expect("generated Rust help catalog")
    });
    let available: BTreeMap<_, _> = tools
        .iter()
        .map(|tool| (tool.name.as_ref(), tool))
        .collect();
    let mut grouped = Vec::<(String, Vec<&Tool>)>::new();
    let mut assigned = BTreeSet::new();
    for group in reference.groups.get(scope).into_iter().flatten() {
        let entries: Vec<_> = group
            .tools
            .iter()
            .filter_map(|name| available.get(name.as_str()).copied())
            .collect();
        if entries.is_empty() {
            continue;
        }
        assigned.extend(entries.iter().map(|tool| tool.name.as_ref()));
        grouped.push((group.name.clone(), entries));
    }
    let plugins: Vec<_> = available
        .values()
        .copied()
        .filter(|tool| tool.name != "help" && !assigned.contains(tool.name.as_ref()))
        .collect();
    if !plugins.is_empty() {
        grouped.push(("plugins".into(), plugins));
    }
    let topic = topic.trim().to_lowercase();
    if topic.is_empty() {
        let mut text = format!(
            "Workspacer tools — tier: {scope}. Call help with a topic for usage guidance.\n"
        );
        for (name, entries) in &grouped {
            text.push_str(&format!(
                "- {name}: {}\n",
                entries
                    .iter()
                    .map(|tool| tool.name.as_ref())
                    .collect::<Vec<_>>()
                    .join(", ")
            ));
        }
        if grouped.iter().map(|(_, tools)| tools.len()).sum::<usize>() >= 2 {
            text.push_str("\nIf your runtime defers schemas, batch-fetch only the tools needed for this turn using its exposed names. Do not preload the whole catalog.");
        }
        return text.trim_end_matches('\n').into();
    }
    let Some((_, entries)) = grouped.iter().find(|(name, _)| name == &topic) else {
        let mut names: Vec<_> = grouped.iter().map(|(name, _)| name.as_str()).collect();
        names.sort();
        return format!(
            "unknown topic {topic:?} — this tier's topics: {}",
            names.join(", ")
        );
    };
    let mut text = format!("{topic} tools (tier: {scope})\n");
    for tool in entries {
        text.push_str(&format!(
            "- {}: {}\n",
            tool.name,
            tool.description.as_deref().unwrap_or("")
        ));
    }
    if let Some(guidance) = reference
        .guidance
        .get(&topic)
        .filter(|text| !text.is_empty())
    {
        text.push('\n');
        text.push_str(guidance);
    }
    text
}
