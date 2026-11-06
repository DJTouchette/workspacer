//! Hosts → Projects: each local project against each federation peer
//! (`projects.linkMatrix`), so a launch on a peer finds the same project
//! under that machine's own path (`docs/host-work-queue.md`, "Project
//! links"). A cell shows the saved link, an automatic match (same path, or
//! the one folder with the same git remote), suggestions to accept, or the
//! peer's folders to link by hand. The owner's refusal shows in the cell.
//!
//! The model lives in `wks_native::project_links`. The section re-reads on
//! `projects.linksChanged` (watched with the Hosts view) and after a write.
use super::*;
use wks_native::features::{LinkAction, Request};
use wks_native::project_links::{CellState, CellView, Directory, PeerCell, Project, Snapshot};

const DESCRIPTION: &str = "The same project on each peer, under that machine's own path. Launches on a peer use the folder shown here.";

impl Workspace {
    pub(super) fn render_project_links(&self, cx: &mut Context<Self>) -> Div {
        let p = self.palette();
        let state = self.view.requests.get("project-links");
        let loading = state.is_some_and(|s| s.loading);
        let header = div()
            .flex()
            .items_center()
            .justify_between()
            .child(projects::section_label("Projects", p))
            .child(
                self.quiet_button(
                    "projects-refresh",
                    "Refresh",
                    IconName::Redo,
                    self.view.connected && !loading,
                )
                .debug_selector(|| "projects-refresh".into())
                .when(self.view.connected && !loading, |d| {
                    d.on_click(cx.listener(|this, _, _, cx| {
                        this.command(Command::Request(Request::ProjectLinks), cx)
                    }))
                }),
            );
        let caption = |text: &str| {
            div()
                .text_size(px(chrome::scale::CAPTION))
                .text_color(rgb(p.muted))
                .child(text.to_owned())
        };
        let section = div()
            .debug_selector(|| "projects-section".into())
            .pt_2()
            .flex()
            .flex_col()
            .gap_3()
            .child(header);
        let snapshot = state
            .filter(|s| s.value.is_object())
            .map(|s| Snapshot::parse(&s.value));
        let Some(snapshot) = snapshot else {
            return match state.and_then(|s| s.error.clone()) {
                Some(error) if !loading => {
                    section.child(chrome::notice_line(error, chrome::Tone::Error, p))
                }
                _ => section.child(chrome::notice_line(
                    "Loading projects…",
                    chrome::Tone::Loading,
                    p,
                )),
            };
        };
        if snapshot.unsupported() {
            return section.child(
                caption("Project links are not supported by this hub. Update it to link the same project across machines.")
                    .debug_selector(|| "projects-unsupported".into()),
            );
        }
        let matrix = match &snapshot.matrix {
            Ok(matrix) => matrix,
            Err(error) => {
                return section.child(chrome::notice_line(error.clone(), chrome::Tone::Error, p));
            }
        };
        let mut section = section.child(caption(DESCRIPTION));
        if matrix.projects.is_empty() {
            return section.child(caption("No local projects yet."));
        }
        if matrix.peers().is_empty() {
            return section.child(caption("No federation peers to link projects with."));
        }
        for (row, project) in matrix.projects.iter().enumerate() {
            section = section.child(self.render_link_row(row, project, snapshot.saved(), cx));
        }
        section
    }

    fn render_link_row(
        &self,
        row: usize,
        project: &Project,
        links: &[wks_native::project_links::Link],
        cx: &mut Context<Self>,
    ) -> Div {
        let p = self.palette();
        let head = div()
            .flex()
            .items_baseline()
            .gap_2()
            .min_w_0()
            .child(
                div()
                    .flex_shrink_0()
                    .text_size(px(chrome::scale::BODY))
                    .font_weight(FontWeight::SEMIBOLD)
                    .child(project.name.clone()),
            )
            .child(
                div()
                    .min_w_0()
                    .truncate()
                    .font_family(mono_font())
                    .text_size(px(chrome::scale::CAPTION))
                    .text_color(rgb(p.muted))
                    .child(project.path.clone()),
            )
            .child(div().flex_1())
            .child(
                div()
                    .flex_shrink_0()
                    .max_w(px(260.))
                    .truncate()
                    .text_size(px(chrome::scale::CAPTION))
                    .text_color(rgb(p.muted))
                    .opacity(0.8)
                    .child(project.remote.clone().unwrap_or_else(|| "no remote".into())),
            );
        let mut cells = div().flex().flex_wrap().gap_2();
        for (col, cell) in project.peers.iter().enumerate() {
            let view = CellView::new(project, cell, links);
            cells = cells.child(self.render_link_cell(row, col, project, cell, &view, cx));
        }
        chrome::card(p)
            .debug_selector(move || format!("project-row-{row}"))
            .p_4()
            .flex()
            .flex_col()
            .gap_3()
            .child(head)
            .child(cells)
    }

    fn render_link_cell(
        &self,
        row: usize,
        col: usize,
        project: &Project,
        cell: &PeerCell,
        view: &CellView,
        cx: &mut Context<Self>,
    ) -> Div {
        let p = self.palette();
        let action = self.view.requests.get("link-action");
        let busy = action.is_some_and(|s| s.loading);
        let enabled = self.view.connected && !busy;
        let pair = (project.path.clone(), cell.peer.clone());
        let error = action
            .filter(|s| !s.loading)
            .filter(|s| {
                matches!(&s.request, Request::LinkAction(a)
                    if a.pair() == (pair.0.as_str(), pair.1.as_str()))
            })
            .and_then(|s| s.error.clone());
        let set = move |peer_path: String| {
            Request::LinkAction(LinkAction::Set {
                local_path: pair.0.clone(),
                peer: pair.1.clone(),
                peer_path,
            })
        };
        let mono = |text: String| {
            div()
                .min_w_0()
                .truncate()
                .font_family(mono_font())
                .text_size(px(chrome::scale::META))
                .child(text)
        };
        let muted = |text: String| {
            div()
                .text_size(px(chrome::scale::CAPTION))
                .text_color(rgb(p.muted))
                .child(text)
        };
        let mut body = div().flex().flex_col().gap_1().min_w_0();
        match &view.state {
            CellState::Linked(path) => {
                body = body.child(
                    div()
                        .flex()
                        .items_center()
                        .gap_2()
                        .min_w_0()
                        .child(mono(path.clone()))
                        .child(
                            super::hosts::pill("linked".into(), p.accent, p)
                                .debug_selector(move || format!("link-linked-{row}-{col}")),
                        ),
                );
            }
            CellState::Matched { path, via } => {
                body = body.child(
                    div()
                        .flex()
                        .items_center()
                        .gap_2()
                        .min_w_0()
                        .child(mono(path.clone()))
                        .child(
                            super::hosts::pill(via.label().to_owned(), p.success, p)
                                .debug_selector(move || format!("link-matched-{row}-{col}")),
                        ),
                );
            }
            CellState::Suggested(suggestions) => {
                for (ix, suggestion) in suggestions.iter().enumerate() {
                    let request = set(suggestion.path.clone());
                    body = body.child(
                        div()
                            .debug_selector(move || format!("link-suggestion-{row}-{col}-{ix}"))
                            .flex()
                            .items_center()
                            .gap_2()
                            .min_w_0()
                            .child(
                                div()
                                    .flex_1()
                                    .min_w_0()
                                    .flex()
                                    .flex_col()
                                    .child(mono(suggestion.path.clone()))
                                    .child(muted(format!("suggested · {}", suggestion.label()))),
                            )
                            .child(
                                self.quiet_button(
                                    SharedString::from(format!("link-accept-{row}-{col}-{ix}")),
                                    "Accept",
                                    IconName::Check,
                                    enabled,
                                )
                                .debug_selector(move || format!("link-accept-{row}-{col}-{ix}"))
                                .when(enabled, |d| {
                                    d.on_click(cx.listener(move |this, _, _, cx| {
                                        this.extras.link_choose = None;
                                        this.command(Command::Request(request.clone()), cx);
                                        cx.notify();
                                    }))
                                }),
                            ),
                    );
                }
            }
            CellState::Missing => {
                body = body.child(
                    muted(if view.offline && cell.choices.is_empty() {
                        format!("{} is offline", cell.peer)
                    } else {
                        format!("not on {}", cell.peer)
                    })
                    .debug_selector(move || format!("link-missing-{row}-{col}")),
                );
            }
        }
        if let Some(note) = view.stale_note(&cell.peer) {
            body = body.child(
                chrome::notice_line(note, chrome::Tone::Warning, p)
                    .debug_selector(move || format!("link-stale-{row}-{col}")),
            );
        }
        let open =
            self.extras.link_choose.as_ref() == Some(&(project.path.clone(), cell.peer.clone()));
        let mut actions = div().flex().flex_wrap().gap_1();
        if !cell.choices.is_empty() {
            let key = (project.path.clone(), cell.peer.clone());
            actions = actions.child(
                self.quiet_button(
                    SharedString::from(format!("link-choose-{row}-{col}")),
                    "Choose…",
                    IconName::Folder,
                    enabled,
                )
                .debug_selector(move || format!("link-choose-{row}-{col}"))
                .when(enabled, |d| {
                    d.on_click(cx.listener(move |this, _, _, cx| {
                        this.extras.link_choose = if this.extras.link_choose.as_ref() == Some(&key)
                        {
                            None
                        } else {
                            Some(key.clone())
                        };
                        cx.notify();
                    }))
                }),
            );
        }
        if view.saved.is_some() {
            let request = Request::LinkAction(LinkAction::Remove {
                local_path: project.path.clone(),
                peer: cell.peer.clone(),
            });
            actions = actions.child(
                self.quiet_button(
                    SharedString::from(format!("link-unlink-{row}-{col}")),
                    "Unlink",
                    IconName::Close,
                    enabled,
                )
                .debug_selector(move || format!("link-unlink-{row}-{col}"))
                .when(enabled, |d| {
                    d.on_click(cx.listener(move |this, _, _, cx| {
                        this.extras.link_choose = None;
                        this.command(Command::Request(request.clone()), cx);
                        cx.notify();
                    }))
                }),
            );
        }
        let mut choices = div()
            .flex()
            .flex_col()
            .rounded(px(p.control_radius))
            .border_1()
            .border_color(rgb(p.border))
            .overflow_hidden();
        if open {
            for (ix, choice) in cell.choices.iter().enumerate() {
                let request = set(choice.path.clone());
                choices = choices.child(
                    self.render_link_choice(row, col, ix, choice, enabled)
                        .when(enabled, |d| {
                            d.on_click(cx.listener(move |this, _, _, cx| {
                                this.extras.link_choose = None;
                                this.command(Command::Request(request.clone()), cx);
                                cx.notify();
                            }))
                        }),
                );
            }
        }
        div()
            .debug_selector(move || format!("link-cell-{row}-{col}"))
            .w(px(280.))
            .p_2()
            .rounded(px(p.control_radius))
            .border_1()
            .border_color(rgb(p.border))
            .bg(rgb(p.chat))
            .flex()
            .flex_col()
            .gap_2()
            .when(view.offline, |d| d.opacity(0.55))
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap_2()
                    .child(
                        div()
                            .text_size(px(chrome::scale::META))
                            .font_weight(FontWeight::SEMIBOLD)
                            .child(cell.peer.clone()),
                    )
                    .when(view.offline, |d| {
                        d.child(super::hosts::pill("offline".into(), p.muted, p))
                    }),
            )
            .child(body)
            .child(actions)
            .when(open, |d| {
                d.child(choices.debug_selector(move || format!("link-choices-{row}-{col}")))
            })
            .when_some(error, |d, error| {
                d.child(
                    chrome::notice_line(error, chrome::Tone::Error, p)
                        .debug_selector(move || format!("link-error-{row}-{col}")),
                )
            })
    }

    /// One folder in a cell's "Choose…" list: its name, path, and its
    /// remote dimmed.
    fn render_link_choice(
        &self,
        row: usize,
        col: usize,
        ix: usize,
        choice: &Directory,
        enabled: bool,
    ) -> Stateful<Div> {
        let p = self.palette();
        chrome::interactive_control(
            div().id(SharedString::from(format!("link-choice-{row}-{col}-{ix}"))),
            p,
            enabled,
        )
        .debug_selector(move || format!("link-choice-{row}-{col}-{ix}"))
        .when(ix > 0, |d| d.border_t_1().border_color(rgb(p.border)))
        .px_2()
        .py_1()
        .flex()
        .flex_col()
        .min_w_0()
        .when(enabled, |d| {
            d.cursor_pointer().hover(|s| s.bg(rgb(p.selected)))
        })
        .child(
            div()
                .text_size(px(chrome::scale::META))
                .font_weight(FontWeight::MEDIUM)
                .child(choice.name.clone()),
        )
        .child(
            div()
                .min_w_0()
                .truncate()
                .font_family(mono_font())
                .text_size(px(chrome::scale::CAPTION))
                .child(choice.path.clone()),
        )
        .when_some(choice.remote.clone(), |d, remote| {
            d.child(
                div()
                    .min_w_0()
                    .truncate()
                    .text_size(px(chrome::scale::CAPTION))
                    .text_color(rgb(p.muted))
                    .child(remote),
            )
        })
    }
}
