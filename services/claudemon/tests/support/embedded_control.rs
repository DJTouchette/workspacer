//! Private unit fixture included by embedded.rs only under cfg(test).
use super::*;
use axum::{routing::get, Json};
use std::sync::{
    atomic::{AtomicUsize, Ordering},
    Arc, Mutex,
};
use std::time::Duration;

fn owner(router: Router) -> (EmbeddedClient, tokio::task::JoinHandle<()>) {
    let (commands, receiver) = mpsc::channel(COMMAND_QUEUE);
    let (state, status) = watch::channel(Status::Starting);
    let store = Arc::new(Mutex::new(None));
    let conversations = Arc::new(Mutex::new(None));
    let client = EmbeddedClient {
        commands,
        status,
        store: store.clone(),
        conversations: conversations.clone(),
    };
    let task = serve_commands(
        Control {
            receiver,
            status: state,
            cleanup: store,
            conversations,
            options: Options::default(),
        },
        router,
        "127.0.0.1:0".parse().unwrap(),
        "127.0.0.1:0".parse().unwrap(),
    );
    (client, task)
}

#[derive(Default)]
struct Counts {
    active: AtomicUsize,
    peak: AtomicUsize,
    started: AtomicUsize,
    effects: AtomicUsize,
    dropped: AtomicUsize,
    notice: tokio::sync::Notify,
}
struct Active(Arc<Counts>);
impl Active {
    fn new(counts: Arc<Counts>) -> Self {
        let active = counts.active.fetch_add(1, Ordering::SeqCst) + 1;
        counts.peak.fetch_max(active, Ordering::SeqCst);
        counts.started.fetch_add(1, Ordering::SeqCst);
        counts.notice.notify_one();
        Self(counts)
    }
}
impl Drop for Active {
    fn drop(&mut self) {
        self.0.active.fetch_sub(1, Ordering::SeqCst);
        self.0.dropped.fetch_add(1, Ordering::SeqCst);
        self.0.notice.notify_one();
    }
}
async fn started(counts: &Counts, target: usize) {
    tokio::time::timeout(Duration::from_secs(3), async {
        while counts.started.load(Ordering::SeqCst) < target {
            counts.notice.notified().await;
        }
    })
    .await
    .expect("owned dispatch did not reach its phase barrier");
}
fn held_router(counts: Arc<Counts>, gate: Arc<tokio::sync::Semaphore>) -> Router {
    Router::new().route(
        "/held",
        get(move || {
            let (counts, gate) = (counts.clone(), gate.clone());
            async move {
                let _active = Active::new(counts.clone());
                gate.acquire().await.unwrap().forget();
                counts.effects.fetch_add(1, Ordering::SeqCst);
                Json(json!({"held":true}))
            }
        }),
    )
}

#[tokio::test]
async fn concurrency_queue_and_caller_cancellation_stay_bounded() {
    assert_eq!(COMMAND_QUEUE, 64);
    assert_eq!(COMMAND_CONCURRENCY, 8);
    assert_eq!(COMMAND_TIMEOUT, Duration::from_secs(30));
    let counts = Arc::new(Counts::default());
    let gate = Arc::new(tokio::sync::Semaphore::new(0));
    let fast_effects = Arc::new(AtomicUsize::new(0));
    let recorded = fast_effects.clone();
    let router = held_router(counts.clone(), gate.clone()).route(
        "/fast",
        get(move || {
            let recorded = recorded.clone();
            async move {
                recorded.fetch_add(1, Ordering::SeqCst);
                Json(json!({"fast":true}))
            }
        }),
    );
    let (client, task) = owner(router);
    let mut held = Vec::new();
    for _ in 0..COMMAND_CONCURRENCY {
        let client = client.clone();
        held.push(tokio::spawn(async move {
            client.request(request("/held")).await
        }));
    }
    started(&counts, COMMAND_CONCURRENCY).await;
    let cancelled = held.remove(0);
    cancelled.abort();
    assert!(cancelled.await.unwrap_err().is_cancelled());
    assert_eq!(
        counts.active.load(Ordering::SeqCst),
        COMMAND_CONCURRENCY,
        "dropping a started caller must not pretend its effect was cancelled"
    );
    let mut replies = Vec::new();
    for index in 0..COMMAND_QUEUE {
        let (reply, received) = oneshot::channel();
        let command = request(if index < 2 { "/fast" } else { "/held" });
        assert!(client
            .commands
            .try_send(Envelope { command, reply })
            .is_ok());
        replies.push(received);
    }
    assert_eq!(client.commands.capacity(), 0);
    let rejected = client.request(request("/fast")).await.unwrap_err();
    assert!(rejected
        .to_string()
        .contains("queue is full; command was not submitted"));
    assert_eq!(fast_effects.load(Ordering::SeqCst), 0);
    // The first queued caller leaves before dispatch. One released active slot
    // must skip it, complete the next fast read, then admit exactly one hold.
    drop(replies.remove(0));
    let fast = replies.remove(0);
    gate.add_permits(1);
    assert_eq!(
        tokio::time::timeout(Duration::from_secs(3), fast)
            .await
            .unwrap()
            .unwrap()
            .unwrap(),
        json!({"fast":true})
    );
    started(&counts, COMMAND_CONCURRENCY + 1).await;
    assert_eq!(counts.active.load(Ordering::SeqCst), COMMAND_CONCURRENCY);
    assert_eq!(client.commands.capacity(), 3);
    assert_eq!(
        fast_effects.load(Ordering::SeqCst),
        1,
        "closed queued caller was executed"
    );
    gate.add_permits(COMMAND_CONCURRENCY - 1 + COMMAND_QUEUE - 2);
    for reply in replies {
        assert_eq!(
            tokio::time::timeout(Duration::from_secs(3), reply)
                .await
                .unwrap()
                .unwrap()
                .unwrap(),
            json!({"held":true})
        );
    }
    for caller in held {
        tokio::time::timeout(Duration::from_secs(3), caller)
            .await
            .unwrap()
            .unwrap()
            .unwrap();
    }
    drop(client);
    tokio::time::timeout(Duration::from_secs(3), task)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(counts.active.load(Ordering::SeqCst), 0);
    assert_eq!(
        counts.effects.load(Ordering::SeqCst),
        COMMAND_CONCURRENCY + COMMAND_QUEUE - 2
    );
    assert_eq!(counts.peak.load(Ordering::SeqCst), COMMAND_CONCURRENCY);
}

#[tokio::test]
async fn closing_senders_drains_admitted_futures_without_spinning() {
    let counts = Arc::new(Counts::default());
    let first_gate = Arc::new(tokio::sync::Semaphore::new(0));
    let second_gate = Arc::new(tokio::sync::Semaphore::new(0));
    let router = held_router(counts.clone(), first_gate.clone())
        .nest("/second", held_router(counts.clone(), second_gate.clone()));
    let (client, task) = owner(router);
    let mut replies = Vec::new();
    for path in ["/held", "/second/held"] {
        let (reply, received) = oneshot::channel();
        assert!(client
            .commands
            .try_send(Envelope {
                command: request(path),
                reply
            })
            .is_ok());
        replies.push(received);
    }
    started(&counts, 2).await;
    drop(client);
    // First receipt is a positive owner-progress barrier after channel close;
    // the second admitted future must still be alive when its gate releases.
    first_gate.add_permits(1);
    tokio::time::timeout(Duration::from_secs(3), replies.remove(0))
        .await
        .unwrap()
        .unwrap()
        .unwrap();
    second_gate.add_permits(1);
    tokio::time::timeout(Duration::from_secs(3), replies.remove(0))
        .await
        .unwrap()
        .unwrap()
        .unwrap();
    tokio::time::timeout(Duration::from_secs(3), task)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(counts.effects.load(Ordering::SeqCst), 2);
    assert_eq!(counts.active.load(Ordering::SeqCst), 0);
}

#[tokio::test]
async fn aborting_the_owner_drops_all_active_and_queued_replies() {
    let counts = Arc::new(Counts::default());
    let gate = Arc::new(tokio::sync::Semaphore::new(0));
    let (client, task) = owner(held_router(counts.clone(), gate));
    let mut callers = Vec::new();
    for _ in 0..COMMAND_CONCURRENCY {
        let client = client.clone();
        callers.push(tokio::spawn(async move {
            client.request(request("/held")).await
        }));
    }
    started(&counts, COMMAND_CONCURRENCY).await;
    let (reply, queued) = oneshot::channel();
    assert!(client
        .commands
        .try_send(Envelope {
            command: request("/held"),
            reply
        })
        .is_ok());
    task.abort();
    assert!(task.await.unwrap_err().is_cancelled());
    assert_eq!(counts.active.load(Ordering::SeqCst), 0);
    assert_eq!(counts.dropped.load(Ordering::SeqCst), COMMAND_CONCURRENCY);
    assert_eq!(counts.effects.load(Ordering::SeqCst), 0);
    assert!(tokio::time::timeout(Duration::from_secs(3), queued)
        .await
        .unwrap()
        .is_err());
    for caller in callers {
        assert!(tokio::time::timeout(Duration::from_secs(3), caller)
            .await
            .unwrap()
            .unwrap()
            .unwrap_err()
            .to_string()
            .contains("outcome is unknown"));
    }
    assert!(client
        .request(request("/held"))
        .await
        .unwrap_err()
        .to_string()
        .contains("stopped; command was not submitted"));
}

#[tokio::test(start_paused = true)]
async fn thirty_second_limits_preserve_unknown_outcomes_and_drop_dispatch() {
    let counts = Arc::new(Counts::default());
    let gate = Arc::new(tokio::sync::Semaphore::new(0));
    let (client, task) = owner(held_router(counts.clone(), gate));
    let waiting = client.clone();
    let caller = tokio::spawn(async move { waiting.request(request("/held")).await });
    started(&counts, 1).await;
    tokio::time::advance(Duration::from_secs(29)).await;
    assert!(!caller.is_finished(), "deadline shortened before30 seconds");
    tokio::time::advance(Duration::from_secs(1)).await;
    let error = caller.await.unwrap().unwrap_err();
    assert!(error.to_string().contains("outcome is unknown"));
    assert!(error.downcast_ref::<CommandRejected>().is_none());
    drop(client);
    tokio::time::timeout(Duration::from_secs(1), task)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(counts.active.load(Ordering::SeqCst), 0);
    assert_eq!(counts.dropped.load(Ordering::SeqCst), 1);
    assert_eq!(counts.effects.load(Ordering::SeqCst), 0);
}
fn request(path: &str) -> Command {
    Command::Request {
        method: "GET".into(),
        path: path.into(),
        payload: None,
    }
}

#[tokio::test]
async fn a_held_command_does_not_block_a_fast_sibling() {
    let entered = Arc::new(tokio::sync::Notify::new());
    let release = Arc::new(tokio::sync::Semaphore::new(0));
    let fast_count = Arc::new(AtomicUsize::new(0));
    let (began, gate, count) = (entered.clone(), release.clone(), fast_count.clone());
    let router = Router::new()
        .route(
            "/held",
            get(move || {
                let (began, gate) = (began.clone(), gate.clone());
                async move {
                    began.notify_one();
                    gate.acquire().await.unwrap().forget();
                    Json(json!({"held":true}))
                }
            }),
        )
        .route(
            "/fast",
            get(move || {
                let count = count.clone();
                async move {
                    count.fetch_add(1, Ordering::SeqCst);
                    Json(json!({"fast":true}))
                }
            }),
        );
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let (stop, stopped) = oneshot::channel();
    let http_router = router.clone();
    let http_owner = tokio::spawn(async move {
        axum::serve(listener, http_router)
            .with_graceful_shutdown(async {
                let _ = stopped.await;
            })
            .await
            .unwrap();
    });
    let (client, task) = owner(router);
    let slow_client = client.clone();
    let slow = tokio::spawn(async move { slow_client.request(request("/held")).await });
    tokio::time::timeout(Duration::from_secs(3), entered.notified())
        .await
        .unwrap();
    // Positive control: the exact same Router serves a real HTTP sibling while
    // the first route is held. The test never releases that route on a timer.
    let http = reqwest::Client::builder()
        .no_proxy()
        .timeout(Duration::from_secs(3))
        .build()
        .unwrap();
    let direct: Value = http
        .get(format!("http://{address}/fast"))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(direct, json!({"fast":true}));
    let fast_client = client.clone();
    let mut fast = tokio::spawn(async move { fast_client.request(request("/fast")).await });
    let before_release = tokio::time::timeout(Duration::from_secs(3), &mut fast).await;
    release.add_permits(1);
    assert_eq!(slow.await.unwrap().unwrap(), json!({"held":true}));
    let independent = match before_release {
        Ok(result) => {
            assert_eq!(result.unwrap().unwrap(), json!({"fast":true}));
            true
        }
        Err(_) => {
            assert_eq!(fast.await.unwrap().unwrap(), json!({"fast":true}));
            false
        }
    };
    assert_eq!(fast_count.load(Ordering::SeqCst), 2);
    task.abort();
    assert!(task.await.unwrap_err().is_cancelled());
    stop.send(()).unwrap();
    http_owner.await.unwrap();
    assert!(independent,"owned command queue held an unrelated fast read behind a gated route, although the same HTTP router answered immediately");
}
