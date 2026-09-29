//! Read-only inventory probe of an isolated complete Rust backend graph.
#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let report = workspacer_hub::services::capability_inventory::probe().await?;
    println!("{}", serde_json::to_string_pretty(&report)?);
    Ok(())
}
