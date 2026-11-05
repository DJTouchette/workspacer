//! What a chat link does besides opening: a status line naming where the
//! hovered link goes, and a right-click menu (open, copy, open in the default
//! app, show in folder). Every surface that routes link clicks wires these
//! through the same `cwd` rule, so the status names the file a click opens.
use super::*;
use gpui::{AnyElement, MouseButton, Pixels, Point, WeakEntity, relative};
use gpui_component::text::{LinkHoverFn, LinkMenuFn};
use wks_native::links::{self, Link};

#[derive(Default)]
pub(super) struct LinkUi {
    /// The text run reporting the hover, and where its link goes.
    pub(super) hovered: Option<(usize, Link)>,
    pub(super) menu: Option<LinkMenu>,
}

pub(super) struct LinkMenu {
    pub link: Link,
    /// Session whose machine the link's file lives on.
    pub owner: String,
    pub position: Point<Pixels>,
}

/// One menu row: what it says and what it does.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Action {
    Open,
    CopyPath,
    CopyLocation,
    OpenDefault,
    ShowInFolder,
    CopyLink,
    CopyHash,
}

impl Action {
    fn label(self) -> &'static str {
        match self {
            Self::Open => "Open",
            Self::CopyPath => "Copy path",
            Self::CopyLocation => "Copy path:line",
            Self::OpenDefault => "Open in default app",
            Self::ShowInFolder => "Show in folder",
            Self::CopyLink => "Copy link",
            Self::CopyHash => "Copy hash",
        }
    }
    fn id(self) -> &'static str {
        match self {
            Self::Open => "open",
            Self::CopyPath => "copy-path",
            Self::CopyLocation => "copy-location",
            Self::OpenDefault => "open-default",
            Self::ShowInFolder => "show-in-folder",
            Self::CopyLink => "copy-link",
            Self::CopyHash => "copy-hash",
        }
    }
}

/// The rows a link's menu offers. The system actions only appear for files
/// that exist on this computer: a remote session's paths name its machine.
pub(super) fn actions(link: &Link, on_this_computer: bool) -> Vec<Action> {
    match link {
        Link::File(target) => {
            let mut rows = vec![Action::Open, Action::CopyPath];
            if target.line.is_some() {
                rows.push(Action::CopyLocation);
            }
            if on_this_computer {
                rows.extend([Action::OpenDefault, Action::ShowInFolder]);
            }
            rows
        }
        Link::Web(_) | Link::Reference(_) => vec![Action::Open, Action::CopyLink],
        Link::Commit(_) => vec![Action::Open, Action::CopyHash],
        Link::Anchor | Link::Refused(_) => Vec::new(),
    }
}

/// `cwd` when it is absolute (a tool's own working directory), else the
/// selected session's, read when the link is used.
fn resolve_cwd(this: &Workspace, cwd: &Option<String>) -> String {
    match cwd {
        Some(cwd) if wks_native::transcript::absolute(cwd) => cwd.clone(),
        _ => this
            .selected_session()
            .map(|s| s.cwd.clone())
            .unwrap_or_default(),
    }
}

/// Hover and right-click routing for links resolved against `cwd` (see
/// [`resolve_cwd`]).
pub(super) fn link_events(
    workspace: WeakEntity<Workspace>,
    cwd: Option<String>,
) -> (Arc<LinkHoverFn>, Arc<LinkMenuFn>) {
    let hover = {
        let (workspace, cwd) = (workspace.clone(), cwd.clone());
        Arc::new(
            move |source: usize, url: Option<&str>, _: &mut Window, cx: &mut App| {
                let _ = workspace.update(cx, |this, cx| {
                    let link = url.map(|url| links::classify(&resolve_cwd(this, &cwd), url));
                    this.hover_link(source, link, cx);
                });
            },
        ) as Arc<LinkHoverFn>
    };
    let menu = Arc::new(
        move |url: &str, position: Point<Pixels>, _: &mut Window, cx: &mut App| {
            let _ = workspace.update(cx, |this, cx| {
                let Some(owner) = this.view.selected.clone() else {
                    return;
                };
                let link = links::classify(&resolve_cwd(this, &cwd), url);
                if actions(&link, true).is_empty() {
                    return;
                }
                this.links.menu = Some(LinkMenu {
                    link,
                    owner,
                    position,
                });
                this.links.hovered = None;
                cx.notify();
            });
        },
    ) as Arc<LinkMenuFn>;
    (hover, menu)
}

impl Workspace {
    fn hover_link(&mut self, source: usize, link: Option<Link>, cx: &mut Context<Self>) {
        let next = match link {
            Some(link) => Some((source, link)),
            // A late "left" from another run never clears the current link.
            None if self
                .links
                .hovered
                .as_ref()
                .is_some_and(|(s, _)| *s != source) =>
            {
                return;
            }
            None => None,
        };
        if self.links.hovered != next {
            self.links.hovered = next;
            cx.notify();
        }
    }

    /// Forget a hover whose text may no longer be under the pointer (the
    /// conversation scrolled or changed).
    pub(super) fn clear_link_hover(&mut self) {
        self.links.hovered = None;
    }

    /// Where the hovered link goes, shown above the composer like a
    /// browser's status line. It takes no pointer events.
    pub(super) fn render_link_status(&self) -> Option<Div> {
        let (_, link) = self.links.hovered.as_ref()?;
        let text = links::describe(link)?;
        let p = self.appearance.palette();
        Some(
            div()
                .absolute()
                .left(px(12.))
                .bottom(self.composer_dock_bounds.size.height + px(6.))
                .max_w(relative(0.8))
                .child(
                    div()
                        .debug_selector(|| "link-status".into())
                        .px_2()
                        .py(px(3.))
                        .rounded_md()
                        .border_1()
                        .border_color(rgb(p.border))
                        .bg(rgb(p.surface))
                        .shadow(chrome::floating_shadow(p))
                        .text_size(px(11.))
                        .text_color(rgb(if matches!(link, Link::Refused(_)) {
                            p.warning
                        } else {
                            p.muted
                        }))
                        .truncate()
                        .child(text),
                ),
        )
    }

    /// The open link menu over everything, with a backdrop that closes it.
    pub(super) fn render_link_menu(&self, cx: &mut Context<Self>) -> Option<AnyElement> {
        let menu = self.links.menu.as_ref()?;
        let p = self.appearance.palette();
        let local = match &menu.link {
            Link::File(target) => std::path::Path::new(&target.path).exists(),
            _ => false,
        };
        let rows = actions(&menu.link, local);
        let workspace = cx.entity().downgrade();
        let close = move |_: &gpui::MouseDownEvent, _: &mut Window, cx: &mut App| {
            let _ = workspace.update(cx, |this, cx| {
                this.links.menu = None;
                cx.notify();
            });
        };
        let layer = div()
            .id("link-menu-backdrop")
            .absolute()
            .inset_0()
            .occlude()
            .on_mouse_down(MouseButton::Left, close.clone())
            .on_mouse_down(MouseButton::Right, close)
            .child(
                div()
                    .id("link-menu")
                    .debug_selector(|| "link-menu".into())
                    .absolute()
                    .left(menu.position.x)
                    .top(menu.position.y)
                    .min_w(px(180.))
                    .p_1()
                    .flex()
                    .flex_col()
                    .gap(px(2.))
                    .rounded_md()
                    .border_1()
                    .border_color(rgb(p.border))
                    .bg(rgb(p.surface))
                    .shadow(chrome::floating_shadow(p))
                    .occlude()
                    .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
                    .children(rows.into_iter().map(|action| {
                        div()
                            .id(action.id())
                            .debug_selector(move || format!("link-menu-{}", action.id()))
                            .px_2()
                            .py_1()
                            .rounded_md()
                            .cursor_pointer()
                            .hover(|d| d.bg(rgb(p.selected)))
                            .text_size(px(12.))
                            .text_color(rgb(p.text))
                            .child(action.label())
                            .on_click(
                                cx.listener(move |this, _, _, cx| this.run_link_action(action, cx)),
                            )
                    })),
            );
        Some(gpui::deferred(layer).with_priority(2).into_any_element())
    }

    fn run_link_action(&mut self, action: Action, cx: &mut Context<Self>) {
        let Some(LinkMenu { link, owner, .. }) = self.links.menu.take() else {
            return;
        };
        let copy = |text: String, cx: &mut Context<Self>| {
            cx.write_to_clipboard(ClipboardItem::new_string(text))
        };
        match (action, &link) {
            (Action::Open, _) => self.open_link(&owner, link.clone(), cx),
            (Action::CopyPath, Link::File(target)) => copy(target.path.clone(), cx),
            (Action::CopyLocation, Link::File(target)) => copy(target.location(), cx),
            (Action::OpenDefault, Link::File(target)) => {
                cx.open_with_system(std::path::Path::new(&target.path))
            }
            (Action::ShowInFolder, Link::File(target)) => {
                cx.reveal_path(std::path::Path::new(&target.path))
            }
            (Action::CopyLink, Link::Web(url)) => copy(url.clone(), cx),
            (Action::CopyLink, Link::Reference(number)) => {
                let url = self.reference_url(*number);
                copy(url.unwrap_or_else(|| format!("#{number}")), cx)
            }
            (Action::CopyHash, Link::Commit(hash)) => copy(hash.clone(), cx),
            _ => {}
        }
        cx.notify();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn file_menus_offer_system_actions_only_for_local_files() {
        let file = links::classify("/repo", "src/a.rs:3");
        assert_eq!(
            actions(&file, false),
            [Action::Open, Action::CopyPath, Action::CopyLocation]
        );
        assert_eq!(
            actions(&file, true),
            [
                Action::Open,
                Action::CopyPath,
                Action::CopyLocation,
                Action::OpenDefault,
                Action::ShowInFolder
            ]
        );
        let plain = links::classify("/repo", "README.md");
        assert!(!actions(&plain, false).contains(&Action::CopyLocation));
        let web = links::classify("/repo", "https://example.com");
        assert_eq!(actions(&web, false), [Action::Open, Action::CopyLink]);
        assert!(actions(&links::classify("/repo", "~/x.md"), true).is_empty());
        assert_eq!(
            actions(&Link::Commit("9a1b2c3".into()), true),
            [Action::Open, Action::CopyHash]
        );
        assert_eq!(
            actions(&Link::Reference(4), true),
            [Action::Open, Action::CopyLink]
        );
    }
}
