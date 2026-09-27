use anyhow::Result;
use clap::{Parser, Subcommand};
use serde_json::json;
use std::{hint::black_box, time::Instant};
use wks_native::model::{
    ConversationSnapshot, Delta, Item, MAX_ROWS, MAX_TRANSCRIPT_BYTES, Transcript,
};

#[derive(Parser)]
struct Args {
    #[command(subcommand)]
    command: Command,
}
#[derive(Subcommand)]
enum Command {
    /// Launch one real disposable agent, test controller send/reseed, terminate it.
    Live {
        #[arg(long, default_value = "ws://127.0.0.1:7895/bus")]
        bus: String,
        #[arg(long)]
        token_file: std::path::PathBuf,
        #[arg(long, default_value = "claude")]
        provider: String,
        #[arg(long)]
        cwd: std::path::PathBuf,
        /// Leave the test session running for manual native-window inspection.
        #[arg(long)]
        keep_open: bool,
    },
    /// Read-only check against an existing hub; prints counts, never transcripts.
    Probe {
        #[arg(long, default_value = "ws://127.0.0.1:7895/bus")]
        bus: String,
        #[arg(long)]
        token_file: Option<std::path::PathBuf>,
    },
    /// Isolated WebSocket fixture; no models, agents, or production data.
    Serve {
        #[arg(long, default_value = "127.0.0.1:7896")]
        bind: String,
        #[arg(long, default_value_t = 100)]
        sessions: usize,
        #[arg(long, default_value_t = 1000)]
        turns: usize,
    },
    /// Repeatable reducer workload. Reports measured values, not GUI frame time.
    Bench {
        #[arg(long, default_value_t = 20000)]
        events: u64,
    },
}

#[tokio::main(worker_threads = 2)]
async fn main() -> Result<()> {
    match Args::parse().command {
        Command::Live {
            bus,
            token_file,
            provider,
            cwd,
            keep_open,
        } => {
            let token = std::fs::read_to_string(token_file)?.trim().to_owned();
            let config = wks_native::bus::Config::new(bus, Some(token))?;
            let report = wks_native::live::run(config, provider, cwd, keep_open).await?;
            println!("{}", serde_json::to_string_pretty(&report)?);
        }
        Command::Probe { bus, token_file } => {
            use wks_native::{
                bus::{Client, Config, Event},
                model::Session,
            };
            let token = match token_file {
                Some(path) => Some(std::fs::read_to_string(path)?.trim().to_owned()),
                None => std::env::var("HUB_TOKEN").ok(),
            };
            let (client, events) = Client::start(Config::new(bus, token)?);
            tokio::time::timeout(std::time::Duration::from_secs(15), async {
                loop {
                    match events.recv().await? {
                        Event::Connected => break,
                        Event::Disconnected(reason) => anyhow::bail!(reason),
                        Event::Data { .. } => {}
                    }
                }
                anyhow::Ok(())
            })
            .await??;
            let rows = client.call("sessions.snapshots", json!({})).await?;
            let rows = rows
                .as_array()
                .ok_or_else(|| anyhow::anyhow!("Expected session array"))?;
            let mut report = json!({"connected":true,"sessions":rows.len(),"read_only":true});
            if let Some(id) = rows
                .iter()
                .filter(|row| {
                    row.get("hub")
                        .and_then(serde_json::Value::as_str)
                        .is_none_or(str::is_empty)
                })
                .find_map(Session::id_of)
            {
                let wire = client
                    .call("sessions.conversation", json!({"sessionId":id}))
                    .await?;
                let conversation: ConversationSnapshot = serde_json::from_value(wire)?;
                let mut transcript = Transcript::default();
                transcript.snapshot(conversation);
                report["sample_retained_rows"] = json!(transcript.rows.len());
                report["sample_text_bytes"] = json!(transcript.bytes);
                report["conversation_contract"] = json!("decoded");
            }
            println!("{}", serde_json::to_string_pretty(&report)?);
        }
        Command::Serve {
            bind,
            sessions,
            turns,
        } => {
            let listener = tokio::net::TcpListener::bind(&bind).await?;
            println!("Fixture hub: ws://{}/bus", listener.local_addr()?);
            wks_native::harness::serve(listener, sessions.min(10000), turns.min(5000)).await?;
        }
        Command::Bench { events } => {
            let mut transcript = Transcript::default();
            let start = Instant::now();
            transcript.snapshot(ConversationSnapshot {
                seq: 5000,
                first_seq: 1,
                items: (0..5000)
                    .map(|_| Item {
                        kind: "assistant_text".into(),
                        text: "A realistic retained transcript row. ".repeat(100),
                        ..Default::default()
                    })
                    .collect(),
            });
            let snapshot_ms = start.elapsed().as_secs_f64() * 1000.;
            let mut timings = Vec::new();
            let mut published = transcript.clone();
            let start = Instant::now();
            for index in 0..events {
                let delta = Delta {
                    seq: 5001 + index,
                    items: vec![Item {
                        kind: if index % 100 == 0 {
                            "user_message".into()
                        } else {
                            "assistant_text".into()
                        },
                        text: "a fragment of streaming text ".into(),
                        ..Default::default()
                    }],
                    ..Default::default()
                };
                let tick = Instant::now();
                black_box(transcript.delta(delta, true));
                // Model the immutable handoff at a 30 Hz UI cadence, rather
                // than assuming the reducer never shares its strings.
                if index % 10 == 0 {
                    published = transcript.clone();
                }
                black_box(&published);
                timings.push(tick.elapsed().as_nanos());
            }
            let elapsed = start.elapsed().as_secs_f64();
            timings.sort_unstable();
            let percentile = |p: usize| {
                timings
                    .get(timings.len().saturating_sub(1) * p / 100)
                    .copied()
                    .unwrap_or(0) as f64
                    / 1000.
            };
            assert!(transcript.rows.len() <= MAX_ROWS && transcript.bytes <= MAX_TRANSCRIPT_BYTES);
            println!(
                "{}",
                serde_json::to_string_pretty(
                    &json!({"events":events, "snapshot_5000_rows_ms":snapshot_ms,
                "elapsed_seconds":elapsed, "events_per_second":events as f64 / elapsed, "p50_us":percentile(50), "p95_us":percentile(95), "p99_us":percentile(99),
                "retained_rows":transcript.rows.len(), "retained_text_bytes":transcript.bytes,
                "scope":"reducer and immutable snapshot handoff; excludes GPU, layout, network and process RSS"})
                )?
            );
        }
    }
    Ok(())
}
