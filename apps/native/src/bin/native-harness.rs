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
    /// Prove UI bus requests remain display-only without a visible workspace.
    UiIntentProbe,
    /// Own the pure Rust backend using isolated state; no model is launched.
    #[cfg(feature = "rust-hub")]
    #[command(alias = "embedded-probe")]
    RustProbe {
        #[arg(long)]
        directory: std::path::PathBuf,
    },
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
        /// Include image, tool/diff, skill, and response-card examples.
        #[arg(long)]
        rich_transcript: bool,
    },
    /// Measure the real turn-footer summary path, excluding GPUI layout/GPU work.
    BenchTurnSummary {
        #[arg(long, default_value_t = 200)]
        tools: usize,
        #[arg(long, default_value_t = 80)]
        lines: usize,
        #[arg(long, default_value_t = 200)]
        iterations: usize,
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
        Command::UiIntentProbe => println!(
            "{}",
            serde_json::to_string_pretty(&wks_native::harness::ui_intent_probe().await?)?
        ),
        #[cfg(feature = "rust-hub")]
        Command::RustProbe { directory } => rust_probe(directory).await?,
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
                        Event::PowerPaused => anyhow::bail!(
                            "Server requested a reconnect pause; probe did not wake it"
                        ),
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
            rich_transcript,
        } => {
            let listener = tokio::net::TcpListener::bind(&bind).await?;
            println!("Fixture hub: ws://{}/bus", listener.local_addr()?);
            wks_native::harness::serve_with_transcript(
                listener,
                sessions.min(10000),
                turns.min(5000),
                rich_transcript,
            )
            .await?;
        }
        Command::BenchTurnSummary {
            tools,
            lines,
            iterations,
        } => {
            anyhow::ensure!(
                (1..=2000).contains(&tools)
                    && (1..=500).contains(&lines)
                    && (1..=10000).contains(&iterations),
                "tools/lines/iterations outside bounded benchmark range"
            );
            let rows: Vec<_> = (0..tools)
                .map(|index| {
                    let mut tool = wks_native::transcript::Tool::from_item(&Item {
                        name: "Edit".into(),
                        input: json!({"file_path":format!("/fixture/file-{}.rs",index%20),
                        "old_string":"old source line\n".repeat(lines),
                        "new_string":"new source line\n".repeat(lines)}),
                        ..Default::default()
                    });
                    tool.complete = true;
                    wks_native::model::Row {
                        tool: Some(tool),
                        ..Default::default()
                    }
                })
                .collect();
            let run = || wks_native::transcript::turn_changes(black_box(rows.iter()));
            for _ in 0..5 {
                black_box(run());
            }
            let mut timings = Vec::with_capacity(iterations);
            let started = Instant::now();
            for _ in 0..iterations {
                let tick = Instant::now();
                black_box(run());
                timings.push(tick.elapsed().as_nanos());
            }
            let elapsed = started.elapsed().as_secs_f64();
            timings.sort_unstable();
            let percentile = |p: usize| timings[(timings.len() - 1) * p / 100] as f64 / 1000.;
            let changes = run();
            assert_eq!(
                changes.iter().map(|change| change.added).sum::<usize>(),
                tools * lines
            );
            assert_eq!(
                changes.iter().map(|change| change.removed).sum::<usize>(),
                tools * lines
            );
            assert!(changes.iter().all(|change| change.diff.is_empty()));
            println!(
                "{}",
                serde_json::to_string_pretty(&json!({"tools":tools,"lines_per_edit_side":lines,
                "iterations":iterations,"input_bytes":rows.iter().map(|row|row.bytes()).sum::<usize>(),
                "debug_assertions":cfg!(debug_assertions),"elapsed_seconds":elapsed,
                "p50_us":percentile(50),"p95_us":percentile(95),"p99_us":percentile(99),
                "scope":"real turn summary parsing/aggregation only; excludes GPUI layout, GPU, network and Electron; five warmup iterations"}))?
            );
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

#[cfg(feature = "rust-hub")]
async fn rust_probe(directory: std::path::PathBuf) -> Result<()> {
    use anyhow::Context;
    use std::time::Duration;
    use wks_native::{
        controller::Command as BackendCommand,
        host::{Mode, NativeHost, RustOptions},
        launch::CatalogKey,
    };
    let mut options = RustOptions::isolated(directory)?;
    options.home_dir = options.config_dir.parent().unwrap().join("probe-home");
    std::fs::create_dir_all(&options.home_dir)?;
    options.usage_poll_on_boot = Some(false);
    let host = NativeHost::start(Mode::Rust(options))?;
    let controller = host.controller();
    let mut views = controller.views.clone();
    let mut owned_listeners = Vec::new();
    let probe=tokio::time::timeout(Duration::from_secs(100),async {
        let ready=host.ready().await?;
        owned_listeners=ready.owned_listeners.clone();
        anyhow::ensure!(owned_listeners.len()==4,"Missing actual owned listener receipts");
        anyhow::ensure!(ready.bus_url=="in-process","Rust preview must own its in-process hub");
        while !views.borrow_and_update().connected {
            views.changed().await.context("Controller closed before connection")?;
        }
        let key=CatalogKey{provider:"claude".into(),cwd:String::new()};
        controller.command(BackendCommand::LoadModels{key:key.clone(),refresh:true})?;
        loop {
            {let view=views.borrow_and_update();if view.catalog.key==key && !view.catalog.loading {
                anyhow::ensure!(view.catalog.error.is_none() && !view.catalog.models.is_empty(),"Rust catalog failed");
                return anyhow::Ok(json!({"backend":"rust","connected":true,"bus":"in-process","models":view.catalog.models.len(),"sessions":view.sessions.len(),"agents_launched":0}));
            }}
            views.changed().await.context("Controller closed during catalog query")?;
        }
    }).await.context("Rust owned backend probe timed out").and_then(|r|r);
    drop(controller);
    drop(views);
    host.shutdown()
        .await
        .context("Rust owned backend failed to join shutdown")?;
    wks_native::host::verify_owned_listeners_released(&owned_listeners).await?;
    let mut report = probe?;
    report["ports_released"] = true.into();
    report["checked_ports"] = owned_listeners.len().into();
    report["shutdown_joined"] = true.into();
    println!("{}", serde_json::to_string_pretty(&report)?);
    Ok(())
}
