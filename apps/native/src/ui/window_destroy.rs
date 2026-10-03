//! Recover GPUI 0.2 windows destroyed directly by another X11 client.
//!
//! GPUI ignores DestroyNotify and its X11 `window_handle()` panics. Discover
//! through a unique, temporarily fixed title instead. A checked event-mask
//! request acknowledges attachment; only then may the UI change the title.
//! Discovery is bounded: not finding a window is *not* evidence that it is
//! alive. Every failure docks the viewer, including a closed worker channel.
//! Unsupported platforms keep their ordinary window lifecycle.

// The no-X11 build uses Unsupported; keep the receiver protocol shared.
#[cfg_attr(not(any(target_os = "linux", target_os = "freebsd")), allow(dead_code))]
#[derive(Debug, PartialEq, Eq)]
pub(super) enum Event {
    Attached,
    Destroyed,
    /// Never identified: vanished before discovery, or not discoverable.
    Unidentified,
    Ambiguous,
    Failed(String),
}

#[cfg_attr(not(any(target_os = "linux", target_os = "freebsd")), allow(dead_code))]
pub(super) enum Watch {
    Unsupported,
    Started(async_channel::Receiver<Event>),
}

pub(super) fn discovery_title(title: &str, cx: &gpui::App) -> String {
    if supported(cx) {
        use std::sync::atomic::{AtomicU64, Ordering};
        static NEXT: AtomicU64 = AtomicU64::new(1);
        format!(
            "{title} [preview-{}-{}]",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        )
    } else {
        title.into()
    }
}

fn supported(cx: &gpui::App) -> bool {
    cfg!(any(target_os = "linux", target_os = "freebsd")) && cx.compositor_name() == "X11"
}

pub(super) fn watch(title: &str, cx: &gpui::App) -> Watch {
    #[cfg(any(target_os = "linux", target_os = "freebsd"))]
    if supported(cx) {
        return Watch::Started(x11::start(title.to_owned(), None));
    }
    let _ = (title, cx);
    Watch::Unsupported
}

#[cfg(any(target_os = "linux", target_os = "freebsd"))]
mod x11 {
    use super::Event;
    use std::time::{Duration, Instant};
    use x11rb::connection::Connection;
    use x11rb::errors::ReplyError;
    use x11rb::protocol::{
        ErrorKind, Event as XEvent,
        xproto::{AtomEnum, ChangeWindowAttributesAux, ConnectionExt, EventMask, Window},
    };
    use x11rb::rust_connection::RustConnection;

    const DEPTH: usize = 3;
    const DISCOVERY_BUDGET: Duration = Duration::from_millis(250);
    const RETRY: Duration = Duration::from_millis(10);

    pub(super) fn start(title: String, display: Option<String>) -> async_channel::Receiver<Event> {
        // Attachment and one terminal result can both arrive before the UI runs.
        let (sender, receiver) = async_channel::bounded(2);
        let failure = sender.clone();
        let spawned = std::thread::Builder::new()
            .name("x11-window-destroy".into())
            .spawn(move || {
                let result = wait_for_destroy(&title, display.as_deref(), &sender)
                    .unwrap_or_else(|error| Event::Failed(error.to_string()));
                let _ = sender.try_send(result);
            });
        if let Err(error) = spawned {
            let _ = failure.try_send(Event::Failed(error.to_string()));
        }
        receiver
    }

    fn wait_for_destroy(
        title: &str,
        display: Option<&str>,
        sender: &async_channel::Sender<Event>,
    ) -> anyhow::Result<Event> {
        let (connection, screen) = x11rb::connect(display)?;
        let root = connection.setup().roots[screen].root;
        let atom = |name: &str| -> anyhow::Result<u32> {
            Ok(connection
                .intern_atom(false, name.as_bytes())?
                .reply()?
                .atom)
        };
        let atoms = [
            atom("_NET_WM_PID")?,
            atom("_NET_WM_NAME")?,
            atom("UTF8_STRING")?,
        ];
        let deadline = Instant::now() + DISCOVERY_BUDGET;
        let id = loop {
            let matches = ours(&connection, root, std::process::id(), title, atoms)?;
            match matches[..] {
                [id] => break id,
                [] if Instant::now() < deadline && !sender.is_closed() => {
                    // Only discovery retries. Once attached, block on X events.
                    // Reparenting/property registration can be in flight.
                    std::thread::sleep(RETRY);
                }
                [] => return Ok(Event::Unidentified),
                _ => return Ok(Event::Ambiguous),
            }
        };
        let selected = connection
            .change_window_attributes(
                id,
                &ChangeWindowAttributesAux::new().event_mask(EventMask::STRUCTURE_NOTIFY),
            )?
            .check();
        match selected {
            Ok(()) => {}
            Err(ReplyError::X11Error(error)) if error.error_kind == ErrorKind::Window => {
                return Ok(Event::Destroyed);
            }
            Err(error) => return Err(error.into()),
        }
        // Checked selection is a server round-trip, not just a queued request.
        // Destruction from now on is observed by XID, regardless of title.
        if sender.try_send(Event::Attached).is_err() {
            return Ok(Event::Unidentified);
        }
        loop {
            if let XEvent::DestroyNotify(event) = connection.wait_for_event()?
                && event.window == id
            {
                return Ok(Event::Destroyed);
            }
        }
    }

    fn missing(error: &ReplyError) -> bool {
        matches!(error, ReplyError::X11Error(error) if error.error_kind == ErrorKind::Window)
    }

    /// PID + unique fixed title, under the root or reparenting WM frames.
    /// Ignore only BadWindow (a concurrent destroy), not connection failures.
    fn ours(
        connection: &RustConnection,
        root: Window,
        process: u32,
        title: &str,
        [pid, name, utf8]: [u32; 3],
    ) -> anyhow::Result<Vec<Window>> {
        let mut found = vec![];
        let mut level = vec![root];
        for _ in 0..DEPTH {
            let mut next = vec![];
            for parent in level {
                let tree = match connection.query_tree(parent)?.reply() {
                    Ok(tree) => tree,
                    Err(error) if missing(&error) => continue,
                    Err(error) => return Err(error.into()),
                };
                for window in tree.children {
                    next.push(window);
                    let owner = match connection
                        .get_property(false, window, pid, AtomEnum::CARDINAL, 0, 1)?
                        .reply()
                    {
                        Ok(p) => p.value32().and_then(|mut v| v.next()),
                        Err(error) if missing(&error) => continue,
                        Err(error) => return Err(error.into()),
                    };
                    if owner != Some(process) {
                        continue;
                    }
                    let named = match connection
                        .get_property(false, window, name, utf8, 0, 1024)?
                        .reply()
                    {
                        Ok(p) => p.value == title.as_bytes(),
                        Err(error) if missing(&error) => continue,
                        Err(error) => return Err(error.into()),
                    };
                    if named {
                        found.push(window);
                    }
                }
            }
            level = next;
        }
        Ok(found)
    }
}

#[cfg(all(test, any(target_os = "linux", target_os = "freebsd")))]
mod tests {
    use super::*;
    use x11rb::{
        connection::Connection,
        protocol::xproto::{AtomEnum, ConnectionExt, CreateWindowAux, PropMode, WindowClass},
        wrapper::ConnectionExt as _,
    };

    fn receive(receiver: &async_channel::Receiver<Event>) -> Event {
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(3);
        loop {
            match receiver.try_recv() {
                Ok(event) => return event,
                Err(async_channel::TryRecvError::Closed) => {
                    panic!("watch closed without terminal event")
                }
                Err(_) => assert!(std::time::Instant::now() < deadline, "watch timed out"),
            }
            std::thread::sleep(std::time::Duration::from_millis(1));
        }
    }

    /// Explicit private display only: never connect tests to the user desktop.
    #[test]
    #[ignore = "requires WKS_TEST_X11_DISPLAY pointing at an owned private X server"]
    fn x11_discovery_registration_rename_destroy_and_failure() -> anyhow::Result<()> {
        let display = std::env::var("WKS_TEST_X11_DISPLAY")?;
        let (connection, screen) = x11rb::connect(Some(&display))?;
        let root = connection.setup().roots[screen].root;
        let atom = |s: &str| -> anyhow::Result<u32> {
            Ok(connection.intern_atom(false, s.as_bytes())?.reply()?.atom)
        };
        let pid = atom("_NET_WM_PID")?;
        let name = atom("_NET_WM_NAME")?;
        let utf8 = atom("UTF8_STRING")?;
        let title = format!("watch-test-{}", std::process::id());
        let create = || -> anyhow::Result<u32> {
            let id = connection.generate_id()?;
            connection
                .create_window(
                    x11rb::COPY_DEPTH_FROM_PARENT,
                    id,
                    root,
                    0,
                    0,
                    80,
                    80,
                    0,
                    WindowClass::INPUT_OUTPUT,
                    0,
                    &CreateWindowAux::new(),
                )?
                .check()?;
            connection
                .change_property32(
                    PropMode::REPLACE,
                    id,
                    pid,
                    AtomEnum::CARDINAL,
                    &[std::process::id()],
                )?
                .check()?;
            connection
                .change_property8(PropMode::REPLACE, id, name, utf8, title.as_bytes())?
                .check()?;
            connection.map_window(id)?.check()?;
            Ok(id)
        };
        // The real window was mapped and destroyed before the watcher exists.
        // The old production code closed its channel silently here.
        for _ in 0..3 {
            let id = create()?;
            connection.destroy_window(id)?.check()?;
            let events = x11::start(title.clone(), Some(display.clone()));
            assert_eq!(receive(&events), Event::Unidentified);
        }
        // Delayed property registration: discovery is retried, not abandoned.
        let id = create()?;
        connection.delete_property(id, name)?.check()?;
        let events = x11::start(title.clone(), Some(display.clone()));
        std::thread::sleep(std::time::Duration::from_millis(40));
        assert!(events.try_recv().is_err());
        connection
            .change_property8(PropMode::REPLACE, id, name, utf8, title.as_bytes())?
            .check()?;
        assert_eq!(receive(&events), Event::Attached);
        // Renaming after acknowledgement cannot lose the destruction event.
        connection
            .change_property8(PropMode::REPLACE, id, name, utf8, b"new file title")?
            .check()?;
        connection.destroy_window(id)?.check()?;
        assert_eq!(receive(&events), Event::Destroyed);
        // An unexpected external rename before attachment fails closed.
        let id = create()?;
        connection
            .change_property8(PropMode::REPLACE, id, name, utf8, b"external rename")?
            .check()?;
        let events = x11::start(title.clone(), Some(display.clone()));
        assert_eq!(receive(&events), Event::Unidentified);
        connection.destroy_window(id)?.check()?;
        // Never attach to an arbitrary one of two matching windows.
        let a = create()?;
        let b = create()?;
        let events = x11::start(title, Some(display));
        assert_eq!(receive(&events), Event::Ambiguous);
        connection.destroy_window(a)?.check()?;
        connection.destroy_window(b)?.check()?;
        let events = x11::start("unused".into(), Some("invalid-display".into()));
        assert!(matches!(receive(&events), Event::Failed(_)));
        Ok(())
    }
}
