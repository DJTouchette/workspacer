//! Notice when the window system destroys one of our windows without asking.
//!
//! On X11 any client may destroy any window (`xdotool windowclose`,
//! `xkill -id`, a window manager tearing down). GPUI 0.2's X11 backend handles
//! the polite close (WM_DELETE_WINDOW) and UnmapNotify, but not
//! DestroyNotify: the logical window stays registered and every later update
//! of its handle succeeds against a window nobody can see.
//!
//! The watch is a second X11 connection on a thread of its own. GPUI does not
//! expose X11 window ids (its `X11Window::window_handle` is
//! `unimplemented!()`), so the thread finds the window by this process's
//! `_NET_WM_PID` and the exact `_NET_WM_NAME` it was opened with, both of
//! which GPUI sets with checked requests before `open_window` returns. It then
//! selects StructureNotify on that one window (event masks are per client, so
//! GPUI's own selection is untouched) and blocks until its DestroyNotify.
//! That also arrives when GPUI closes the window normally, so the thread never
//! outlives the window; it ends too if the window cannot be identified
//! unambiguously or the connection fails. Wayland, macOS and Windows do not let
//! another client destroy a window, so there is nothing to watch there.

/// Resolves once the platform window titled `title` (just opened by this
/// process) is destroyed, by anyone. `None` when the platform cannot be
/// watched (not GPUI's X11 backend); the channel closes without a message if
/// the window cannot be found. Callers keep their ordinary close handling.
pub(super) fn watch(title: &str, cx: &gpui::App) -> Option<async_channel::Receiver<()>> {
    #[cfg(any(target_os = "linux", target_os = "freebsd"))]
    {
        // Only GPUI's X11 client has X11 windows.
        if cx.compositor_name() != "X11" {
            return None;
        }
        let (destroyed, receiver) = async_channel::bounded(1);
        let title = title.to_owned();
        std::thread::Builder::new()
            .name("x11-window-destroy".into())
            .spawn(move || {
                if x11::wait_for_destroy(&title).unwrap_or(false) {
                    let _ = destroyed.try_send(());
                }
            })
            .ok()?;
        Some(receiver)
    }
    #[cfg(not(any(target_os = "linux", target_os = "freebsd")))]
    {
        let _ = (title, cx);
        None
    }
}

#[cfg(any(target_os = "linux", target_os = "freebsd"))]
mod x11 {
    use x11rb::connection::Connection;
    use x11rb::errors::ReplyError;
    use x11rb::protocol::{
        ErrorKind, Event,
        xproto::{AtomEnum, ChangeWindowAttributesAux, ConnectionExt, EventMask, Window},
    };
    use x11rb::rust_connection::RustConnection;

    /// Top-level windows sit under the root, or one or two frames deeper
    /// with a reparenting window manager.
    const DEPTH: usize = 3;

    /// Blocks until the window is destroyed (`true`), or gives up (`false`).
    pub(super) fn wait_for_destroy(title: &str) -> anyhow::Result<bool> {
        // The same display GPUI connected to ($DISPLAY).
        let (connection, screen) = x11rb::connect(None)?;
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
        let matches = ours(&connection, root, std::process::id(), title, atoms)?;
        let [id] = matches[..] else {
            return Ok(false);
        };
        let selected = connection
            .change_window_attributes(
                id,
                &ChangeWindowAttributesAux::new().event_mask(EventMask::STRUCTURE_NOTIFY),
            )?
            .check();
        match selected {
            Ok(()) => {}
            // Already gone before the watch began.
            Err(ReplyError::X11Error(error)) if error.error_kind == ErrorKind::Window => {
                return Ok(true);
            }
            Err(error) => return Err(error.into()),
        }
        loop {
            if let Event::DestroyNotify(event) = connection.wait_for_event()?
                && event.window == id
            {
                return Ok(true);
            }
        }
    }

    /// This process's windows named `title`.
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
                // Windows can vanish mid-walk; skip them.
                let Ok(tree) = connection.query_tree(parent)?.reply() else {
                    continue;
                };
                for window in tree.children {
                    next.push(window);
                    let owner = connection
                        .get_property(false, window, pid, AtomEnum::CARDINAL, 0, 1)?
                        .reply()
                        .ok()
                        .and_then(|p| p.value32().and_then(|mut v| v.next()));
                    if owner != Some(process) {
                        continue;
                    }
                    let named = connection
                        .get_property(false, window, name, utf8, 0, 1024)?
                        .reply()
                        .is_ok_and(|p| p.value == title.as_bytes());
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
