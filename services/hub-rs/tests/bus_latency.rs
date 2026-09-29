//! Retained mature-snapshot budget, explicitly invoked in an optimized build.
//! Run: cargo test --locked --release --manifest-path services/hub-rs/Cargo.toml
//!      --test bus_latency -- --ignored --nocapture
use futures_util::{SinkExt, StreamExt};
use serde_json::{Value, json};
use std::time::{Duration, Instant};
use tokio_tungstenite::{MaybeTlsStream, WebSocketStream, tungstenite::Message};
use workspacer_hub::{
    Hub, Options,
    protocol::{Event, Frame},
};

const HUB_SAMPLES: usize = 2_000;
const FLOOR_SAMPLES: usize = 500;
const TURNS: usize = 200;
const BUDGET: Duration = Duration::from_millis(5);
const IO_TIMEOUT: Duration = Duration::from_secs(3);
type Socket = WebSocketStream<MaybeTlsStream<tokio::net::TcpStream>>;

fn assert_nodelay(socket: &Socket) {
    let MaybeTlsStream::Plain(stream) = socket.get_ref() else {
        panic!("the isolated latency fixture must use plaintext loopback TCP");
    };
    assert!(
        stream.nodelay().unwrap(),
        "benchmark must match Go and production outbound clients"
    );
}

fn snapshot(turns: usize) -> Value {
    let conversation: Vec<_> = (0..turns).map(|i| json!({
        "role":"assistant",
        "content":"This is a representative conversation turn with some tool output and prose so the payload size is realistic for a mature session.",
        "timestamp":1_730_000_000_000_u64 + i as u64,
    })).collect();
    json!({"sessionId":"sess-123", "cwd":"/home/u/proj", "ambientState":"streaming",
        "totalToolCalls":turns,"conversation":conversation,
        "usage":{"model":"claude-opus-4-8","contextTokens":87000,"contextLimit":200000,"costUSD":1.23}})
}
fn quantile(samples: &mut [Duration], percentile: usize) -> Duration {
    assert!(!samples.is_empty());
    assert!(percentile < 100);
    samples.sort_unstable();
    samples[samples.len() * percentile / 100]
}
#[derive(Debug, PartialEq)]
enum Verdict {
    Pass,
    OverBudget,
    Unmeasurable,
}
fn verdict(hub: Duration, floor: Duration) -> Verdict {
    // Same order and strict boundaries as retained Go bench_test.go: a noisy
    // environment is explicitly unmeasurable, not a hub regression or a pass.
    if floor > BUDGET / 2 {
        Verdict::Unmeasurable
    } else if hub > floor + BUDGET {
        Verdict::OverBudget
    } else {
        Verdict::Pass
    }
}
async fn receive(socket: &mut Socket) -> Message {
    tokio::time::timeout(IO_TIMEOUT, async {
        loop {
            match socket
                .next()
                .await
                .expect("WebSocket closed before receipt")
                .unwrap()
            {
                Message::Ping(data) => socket.send(Message::Pong(data)).await.unwrap(),
                message => return message,
            }
        }
    })
    .await
    .expect("WebSocket sample timed out; dropped data is not a latency sample")
}
async fn write(socket: &mut Socket, message: Message) {
    tokio::time::timeout(IO_TIMEOUT, socket.send(message))
        .await
        .expect("WebSocket send timed out")
        .unwrap();
}
async fn connect(address: std::net::SocketAddr) -> Socket {
    let (mut socket, _) = tokio::time::timeout(
        IO_TIMEOUT,
        tokio_tungstenite::connect_async_with_config(
            format!("ws://{address}/bus?token=latency-fixture-only"),
            None,
            true,
        ),
    )
    .await
    .unwrap()
    .unwrap();
    assert_nodelay(&socket);
    let Message::Text(hello) = receive(&mut socket).await else {
        panic!("missing hello")
    };
    assert_eq!(
        serde_json::from_str::<Value>(&hello).unwrap()["op"],
        "hello"
    );
    socket
}
async fn hub_samples(payload: &Value, count: usize) -> Vec<Duration> {
    let mut options = Options::default();
    options.listen = Some("127.0.0.1:0".parse().unwrap());
    options.token = "latency-fixture-only".into();
    let hub = Hub::start(options).unwrap();
    let address = hub.ready().await.unwrap().unwrap();
    let mut subscriber = connect(address).await;
    write(
        &mut subscriber,
        Message::Text(json!({"op":"subscribe","topics":["agent.*"]}).to_string()),
    )
    .await;
    let Message::Text(ack) = receive(&mut subscriber).await else {
        panic!("missing subscription receipt")
    };
    assert_eq!(
        serde_json::from_str::<Value>(&ack).unwrap()["op"],
        "subscribed"
    );
    let mut publisher = connect(address).await;
    let frame = serde_json::to_string(&Frame {
        event: Some(Event::new("agent.snapshot", "workspacer", payload.clone())),
        ..Frame::op("publish")
    })
    .unwrap();
    let mut elapsed = Vec::with_capacity(count);
    for _ in 0..count {
        let started = Instant::now();
        write(&mut publisher, Message::Text(frame.clone())).await;
        let received = receive(&mut subscriber).await;
        elapsed.push(started.elapsed());
        // Payload validation sits outside the measured interval, but refuses
        // a fast error/control frame masquerading as the expected snapshot.
        let Message::Text(received) = received else {
            panic!("snapshot was not a text frame")
        };
        let received: Frame = serde_json::from_str(&received).unwrap();
        assert_eq!(received.op, "event");
        let event = received.event.unwrap();
        assert_eq!(event.topic, "agent.snapshot");
        assert_eq!(event.data.as_ref(), Some(payload));
    }
    publisher.close(None).await.unwrap();
    subscriber.close(None).await.unwrap();
    hub.shutdown().unwrap();
    elapsed
}
async fn floor_samples(payload_bytes: usize, count: usize) -> Vec<Duration> {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let echo = tokio::spawn(async move {
        let (stream, _) = listener.accept().await.unwrap();
        stream.set_nodelay(true).unwrap();
        assert!(stream.nodelay().unwrap());
        let mut socket = tokio_tungstenite::accept_async(stream).await.unwrap();
        for _ in 0..count {
            let message = socket.next().await.expect("echo client closed").unwrap();
            socket.send(message).await.unwrap();
        }
        // Dropping the echo after its final write flushes the queued response
        // without leaving a server task alive past the measurement.
    });
    let (mut socket, _) =
        tokio_tungstenite::connect_async_with_config(format!("ws://{address}"), None, true)
            .await
            .unwrap();
    assert_nodelay(&socket);
    let payload = "\0".repeat(payload_bytes);
    let mut elapsed = Vec::with_capacity(count);
    for _ in 0..count {
        let started = Instant::now();
        write(&mut socket, Message::Text(payload.clone())).await;
        let reply = receive(&mut socket).await;
        elapsed.push(started.elapsed());
        assert_eq!(reply, Message::Text(payload.clone()));
    }
    tokio::time::timeout(IO_TIMEOUT, echo)
        .await
        .unwrap()
        .unwrap();
    elapsed
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[ignore = "optimized latency guard; make test-hub-latency runs this explicitly in Linux CI"]
async fn mature_snapshot_p99_share_stays_within_retained_five_millisecond_budget() {
    assert!(
        !cfg!(debug_assertions),
        "run this budget with --release; Rust debug timings are not comparable to optimized Go"
    );
    if cfg!(windows) || std::env::var("WORKSPACER_BUS_LATENCY_INSTRUMENTED").as_deref() == Ok("1") {
        println!(
            "{}",
            json!({"status":"unmeasurable","reason":if cfg!(windows){"Windows scheduler jitter"}else{"explicit instrumentation"},"budgetMicros":5000})
        );
        return;
    }
    let payload = snapshot(TURNS);
    let bytes = serde_json::to_vec(&payload).unwrap().len();
    let (mut hub, mut floor) = tokio::time::timeout(Duration::from_secs(60), async {
        let hub = hub_samples(&payload, HUB_SAMPLES).await;
        let floor = floor_samples(bytes, FLOOR_SAMPLES).await;
        (hub, floor)
    })
    .await
    .expect("latency measurement exceeded its overall one-minute budget");
    let p50 = quantile(&mut hub, 50);
    let p99 = quantile(&mut hub, 99);
    let floor = quantile(&mut floor, 99);
    let decision = verdict(p99, floor);
    let report = json!({"status":match decision{Verdict::Pass=>"pass",Verdict::OverBudget=>"fail",Verdict::Unmeasurable=>"unmeasurable"},
        "profile":"release","turns":TURNS,"payloadBytes":bytes,"hubSamples":HUB_SAMPLES,"floorSamples":FLOOR_SAMPLES,
        "hubP50Micros":p50.as_micros(),"hubP99Micros":p99.as_micros(),"floorP99Micros":floor.as_micros(),
        "hubShareP99Micros":p99.as_micros() as i128-floor.as_micros() as i128,"budgetMicros":BUDGET.as_micros(),
        "reason":if decision==Verdict::Unmeasurable{"bare WebSocket floor exceeds half the budget"}else{"retained baseline-adjusted budget"}});
    println!("{report}");
    assert_ne!(decision, Verdict::OverBudget, "{report}");
}

#[test]
fn retained_budget_boundaries_never_convert_an_unmeasurable_run_into_a_pass() {
    let us = Duration::from_micros;
    assert_eq!(verdict(us(7000), us(2000)), Verdict::Pass);
    assert_eq!(verdict(us(7001), us(2000)), Verdict::OverBudget);
    assert_eq!(verdict(us(7500), us(2500)), Verdict::Pass);
    assert_eq!(verdict(us(8000), us(2501)), Verdict::Unmeasurable);
    assert_eq!(verdict(us(1000), us(2000)), Verdict::Pass);
    let mut samples = [us(50), us(10), us(20), us(40), us(30)];
    assert_eq!(quantile(&mut samples, 50), us(30));
    assert_eq!(quantile(&mut samples, 99), us(50));
}
#[test]
fn mature_snapshot_workload_retains_original_fields_and_two_hundred_turns() {
    let payload = snapshot(TURNS);
    let rows = payload["conversation"].as_array().unwrap();
    assert_eq!(rows.len(), 200);
    assert_eq!(rows[0]["timestamp"], 1_730_000_000_000_u64);
    assert_eq!(rows[199]["timestamp"], 1_730_000_000_199_u64);
    assert_eq!(payload["totalToolCalls"], 200);
    assert_eq!(payload["usage"]["contextTokens"], 87000);
    assert_eq!(payload["usage"]["contextLimit"], 200000);
    assert!(serde_json::to_vec(&payload).unwrap().len() > 30_000);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn sampling_helpers_exchange_real_frames_and_finish_without_claiming_performance() {
    let payload = snapshot(TURNS);
    let samples = hub_samples(&payload, 3).await;
    assert_eq!(samples.len(), 3);
    let bytes = serde_json::to_vec(&payload).unwrap().len();
    let floor = floor_samples(bytes, 3).await;
    assert_eq!(floor.len(), 3);
    // Deliberately no timing threshold: this is functional transport coverage
    // in debug builds, not evidence about the optimized five-millisecond budget.
}
