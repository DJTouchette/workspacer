#![cfg(feature = "rust-hub")]
//! The Hosts view's reads and the owner's writes against a real, isolated
//! Rust hub (`docs/host-work-queue.md`): resources, queue, targets and
//! limits read together, a limits change, and a refused cancel.
use std::{sync::Arc, time::Duration};
use wks_native::{
    controller::{Command, Controller, View},
    features::{HostAction, Request, RequestState},
    host::{Mode, NativeHost, RustOptions},
    hosts::{self, Snapshot, Support},
};

async fn run(controller: &Controller, request: Request) -> RequestState {
    let key = request.key();
    let mut views = controller.views.clone();
    let before = views.borrow().requests.get(key).map_or(0, |s| s.number);
    controller.command(Command::Request(request)).unwrap();
    tokio::time::timeout(Duration::from_secs(30), async {
        loop {
            let view: Arc<View> = views.borrow_and_update().clone();
            if let Some(state) = view.requests.get(key)
                && !state.loading
                && state.number > before
            {
                return state.clone();
            }
            views.changed().await.unwrap();
        }
    })
    .await
    .unwrap_or_else(|_| panic!("{key} never completed"))
}

#[tokio::test]
async fn hosts_view_reads_and_owner_writes_round_trip_through_the_hub() {
    let root = std::env::temp_dir().join(format!(
        "wks-native-hosts-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    let mut options = RustOptions::isolated(root.join("state")).unwrap();
    options.home_dir = root.join("home");
    std::fs::create_dir(&options.home_dir).unwrap();
    options.usage_poll_on_boot = Some(false);

    let host = NativeHost::start(Mode::Rust(options)).unwrap();
    tokio::time::timeout(Duration::from_secs(30), host.ready())
        .await
        .unwrap()
        .unwrap();
    let controller = host.controller();
    let mut views = controller.views.clone();
    tokio::time::timeout(Duration::from_secs(10), async {
        while !views.borrow_and_update().connected {
            views.changed().await.unwrap();
        }
    })
    .await
    .unwrap();

    let read = run(&controller, Request::Hosts).await;
    assert_eq!(read.error, None);
    let snapshot = Snapshot::parse(&read.value);
    assert_eq!(snapshot.support(), Support::Supported, "{}", read.value);
    let resources = snapshot.resources.clone().expect("host.resources");
    assert!(resources.sampled_at.is_some(), "{}", read.value);
    assert_eq!(resources.agent_slots, Some(4), "the default maxAgents");
    let list = snapshot.list.clone().expect("queue.list");
    assert!(list.local.items.is_empty());
    // `queue.targets` (placement across peers) may land on the hub after
    // the local queue; until then it must read as unsupported, which hides
    // the launch host picker rather than failing it.
    let targets_supported = match &snapshot.targets {
        Ok(targets) => {
            assert!(targets.targets.iter().any(|t| t.local), "{}", read.value);
            true
        }
        Err(error) => {
            assert!(hosts::unsupported(error), "{error}");
            false
        }
    };
    let limits = snapshot.limits.clone().expect("queue.config");
    assert_eq!(limits.get("maxAgents"), Some(4.));
    let cards = hosts::cards(&snapshot, &hosts::Live::default());
    assert_eq!(cards[0].host, "local");

    // The launch picker's own read, with a provider.
    let picker = run(
        &controller,
        Request::QueueTargets {
            provider: Some("claude".into()),
        },
    )
    .await;
    if targets_supported {
        assert_eq!(picker.error, None);
        assert!(hosts::parse_targets(&picker.value).is_some());
    } else {
        assert!(picker.error.as_deref().is_some_and(hosts::unsupported));
    }

    // The owner (this embedded connection) changes a limit; the answer is
    // a fresh snapshot showing it.
    let patch = hosts::config_patch(&limits, &[("maxAgents", "3".into())])
        .unwrap()
        .unwrap();
    let saved = run(
        &controller,
        Request::HostAction(HostAction::SetConfig(patch)),
    )
    .await;
    assert_eq!(saved.error, None);
    let saved = Snapshot::parse(&saved.value);
    assert_eq!(saved.limits.unwrap().get("maxAgents"), Some(3.));

    // A cancel of nothing is refused with the hub's reason.
    let refused = run(
        &controller,
        Request::HostAction(HostAction::Cancel {
            id: "q_missing".into(),
            host: "local".into(),
        }),
    )
    .await;
    assert!(refused.error.is_some(), "{:?}", refused.value);
    host.shutdown().await.unwrap();
    std::fs::remove_dir_all(root).unwrap();
}
