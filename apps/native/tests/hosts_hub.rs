#![cfg(feature = "rust-hub")]
//! The Hosts view's reads and the owner's writes against a real, isolated
//! Rust hub (`docs/host-work-queue.md`): resources, queue, targets and
//! limits read together, a limits change, and a refused cancel.
use std::{sync::Arc, time::Duration};
use wks_native::{
    controller::{Command, Controller, View},
    features::{HostAction, LinkAction, Request, RequestState},
    host::{Mode, NativeHost, RustOptions},
    hosts::{self, Snapshot, Support},
    project_links,
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

    // The work queue takes its first sample only after the hub is ready and
    // its own client has connected; on Windows the first sysinfo refresh is
    // slow enough that an immediate read can still carry `sampledAt: 0`.
    let deadline = tokio::time::Instant::now() + Duration::from_secs(20);
    let (read, snapshot, resources) = loop {
        let read = run(&controller, Request::Hosts).await;
        assert_eq!(read.error, None);
        let snapshot = Snapshot::parse(&read.value);
        assert_eq!(snapshot.support(), Support::Supported, "{}", read.value);
        let resources = snapshot.resources.clone().expect("host.resources");
        if resources.sampled_at.is_some() {
            break (read, snapshot, resources);
        }
        assert!(
            tokio::time::Instant::now() < deadline,
            "never sampled: {}",
            read.value
        );
        tokio::time::sleep(Duration::from_millis(250)).await;
    };
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
            cwd: None,
        },
    )
    .await;
    if targets_supported {
        assert_eq!(picker.error, None);
        assert!(hosts::parse_targets(&picker.value).is_some());
    } else {
        assert!(picker.error.as_deref().is_some_and(hosts::unsupported));
    }

    // The picker asks with the local project; a hub without project links
    // simply leaves the resolution fields out.
    let project = root.join("home").to_string_lossy().into_owned();
    let resolved = run(
        &controller,
        Request::QueueTargets {
            provider: Some("claude".into()),
            cwd: Some(project.clone()),
        },
    )
    .await;
    if targets_supported {
        assert_eq!(resolved.error, None);
        assert!(hosts::parse_targets(&resolved.value).is_some());
    }

    // Project links may land on the hub after the queue (built in parallel);
    // until then they must read as unsupported, which hides the Projects
    // section rather than failing it.
    let read = run(&controller, Request::ProjectLinks).await;
    assert_eq!(read.error, None);
    let links = project_links::Snapshot::parse(&read.value);
    eprintln!(
        "project links on this hub: {}",
        if links.unsupported() {
            "not yet"
        } else {
            "supported"
        }
    );
    if links.unsupported() {
        assert!(
            links.links.as_ref().is_err_and(|e| hosts::unsupported(e)),
            "{}",
            read.value
        );
    } else {
        let matrix = links.matrix.clone().expect("projects.linkMatrix");
        assert!(
            matrix.peers().is_empty(),
            "no peers configured: {}",
            read.value
        );
        assert_eq!(links.links.clone().expect("projects.links"), vec![]);
        // The owner links a project to a peer that is not even configured
        // (it may be offline), then removes it.
        let set = run(
            &controller,
            Request::LinkAction(LinkAction::Set {
                local_path: project.clone(),
                peer: "mac".into(),
                peer_path: "/Users/dev/app".into(),
            }),
        )
        .await;
        assert_eq!(set.error, None, "{}", set.value);
        let saved = project_links::parse_links(&set.value).unwrap();
        assert_eq!(saved.len(), 1);
        assert_eq!(saved[0].peer_path, "/Users/dev/app");
        let reread = run(&controller, Request::ProjectLinks).await;
        assert_eq!(
            project_links::Snapshot::parse(&reread.value).saved().len(),
            1,
            "{}",
            reread.value
        );
        let removed = run(
            &controller,
            Request::LinkAction(LinkAction::Remove {
                local_path: project.clone(),
                peer: "mac".into(),
            }),
        )
        .await;
        assert_eq!(removed.error, None);
        assert!(
            project_links::parse_links(&removed.value)
                .unwrap()
                .is_empty()
        );
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
