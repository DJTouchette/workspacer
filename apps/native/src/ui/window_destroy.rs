//! Notice when the window system destroys one of our windows without asking.
//!
//! On X11 any client may destroy any window (`xdotool windowclose`,
//! `xkill -id`, a window manager tearing down). GPUI 0.2's X11 backend handles
//! the polite close (WM_DELETE_WINDOW) and UnmapNotify, but not
//! DestroyNotify: the logical window stays registered and every later update
//! of its handle succeeds against a window nobody can see.
//!
//! The watch is a second X11 connection that selects StructureNotify on that
//! one window (event masks are per client, so GPUI's own selection is
//! untouched) and a thread blocked on it. The thread ends at the window's
//! DestroyNotify, which also arrives when GPUI closes the window normally, so
//! it never outlives the window; it ends too if the connection fails.
//! Wayland, macOS and Windows do not let another client destroy a window, so
//! there is nothing to watch there.

/// Resolves once the platform window behind `window` is destroyed, by
/// anyone. `None` when the platform cannot be watched (not GPUI's X11
/// backend, or no second connection); callers keep their ordinary close
/// handling.
pub(super) fn watch(window: &gpui::Window, cx: &gpui::App) -> Option<async_channel::Receiver<()>> {
    #[cfg(any(target_os = "linux", target_os = "freebsd"))]
    {
        use raw_window_handle::{HasWindowHandle, RawWindowHandle};
        // Only GPUI's X11 client has an X11 window to ask for (the test
        // platform's windows have no handle at all).
        if cx.compositor_name() != "X11" {
            return None;
        }
        let id = match HasWindowHandle::window_handle(window).ok()?.as_raw() {
            RawWindowHandle::Xcb(handle) => handle.window.get(),
            RawWindowHandle::Xlib(handle) => u32::try_from(handle.window).ok()?,
            _ => return None,
        };
        x11::watch(id)
    }
    #[cfg(not(any(target_os = "linux", target_os = "freebsd")))]
    {
        let _ = (window, cx);
        None
    }
}

#[cfg(any(target_os = "linux", target_os = "freebsd"))]
mod x11 {
    use x11rb::connection::Connection;
    use x11rb::errors::ReplyError;
    use x11rb::protocol::{
        ErrorKind, Event,
        xproto::{ChangeWindowAttributesAux, ConnectionExt, EventMask},
    };

    pub(super) fn watch(id: u32) -> Option<async_channel::Receiver<()>> {
        // The same display GPUI connected to ($DISPLAY).
        let (connection, _) = x11rb::connect(None).ok()?;
        let (destroyed, receiver) = async_channel::bounded(1);
        let selected = connection
            .change_window_attributes(
                id,
                &ChangeWindowAttributesAux::new().event_mask(EventMask::STRUCTURE_NOTIFY),
            )
            .ok()?
            .check();
        match selected {
            Ok(()) => {}
            // Already gone before the watch began.
            Err(ReplyError::X11Error(error)) if error.error_kind == ErrorKind::Window => {
                let _ = destroyed.try_send(());
                return Some(receiver);
            }
            Err(_) => return None,
        }
        std::thread::Builder::new()
            .name("x11-window-destroy".into())
            .spawn(move || {
                while let Ok(event) = connection.wait_for_event() {
                    if matches!(event, Event::DestroyNotify(e) if e.window == id) {
                        let _ = destroyed.try_send(());
                        return;
                    }
                }
            })
            .ok()?;
        Some(receiver)
    }
}
