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
// r2-pages: plain words ("machine", not peer/hub).
const DESCRIPTION_V2: &str = "The same project on each machine, under that machine's own path. New sessions there use the folder shown here.";

impl Workspace {
    pub(super) fn render_project_links(&self, cx: &mut Context<Self>) -> Div {
        let p = self.palette();
        let state = self.view.requests.get("project-links");
        let loading = state.is_some_and(|s| s.loading);
        let v2 = self.design().is_v2();
        // r2-pages: under v2 the page's one Refresh re-reads these too.
        let header = if v2 {
            div().child(super::primitives::section_label(
                "Projects on other machines",
                &p,
            ))
        } else {
            div()
                .flex()
                .items_center()
                .justify_between()
                .child(projects::section_label(
                    if self.design().is_v2() {
                        "Projects on other machines"
                    } else {
                        "Projects"
                    },
                    p,
                ))
                .child(
                    self.pquiet(
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
                )
        };
        let caption = |text: &str| {
            if v2 {
                let t = p.tokens;
                return div()
                    .text_size(px(t.text.meta))
                    .line_height(gpui::relative(t.text.line_height))
                    .text_color(rgb(p.roles.text_secondary))
                    .child(text.to_owned());
            }
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
                caption(if v2 {
                    "Linking projects across machines needs a newer Workspacer on this machine."
                } else {
                    "Project links are not supported by this hub. Update it to link the same project across machines."
                })
                    .debug_selector(|| "projects-unsupported".into()),
            );
        }
        let matrix = match &snapshot.matrix {
            Ok(matrix) => matrix,
            Err(error) => {
                return section.child(chrome::notice_line(error.clone(), chrome::Tone::Error, p));
            }
        };
        let mut section = section.child(caption(if v2 { DESCRIPTION_V2 } else { DESCRIPTION }));
        if matrix.projects.is_empty() {
            return section.child(caption("No local projects yet."));
        }
        if matrix.peers().is_empty() {
            return section.child(caption(if v2 {
                "No other machines to link projects with."
            } else {
                "No federation peers to link projects with."
            }));
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
        if self.design().is_v2() {
            return self.render_link_row_v2(row, project, links, cx);
        }
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
        let mut cells = if self.design().is_v2() {
            // Equal columns, one per machine, filling the card.
            div().flex().gap(px(p.tokens.space.md))
        } else {
            div().flex().flex_wrap().gap_2()
        };
        for (col, cell) in project.peers.iter().enumerate() {
            let view = CellView::new(project, cell, links);
            cells = cells.child(self.render_link_cell(row, col, project, cell, &view, cx));
        }
        self.page_card()
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
        let v2 = self.design().is_v2();
        let t = p.tokens;
        let mono = |text: String| {
            div()
                .min_w_0()
                .truncate()
                .font_family(mono_font())
                .text_size(px(if v2 { t.text.meta } else { chrome::scale::META }))
                .child(text)
        };
        let muted = |text: String| {
            div()
                .text_size(px(if v2 {
                    t.text.meta
                } else {
                    chrome::scale::CAPTION
                }))
                .text_color(rgb(if v2 { p.roles.text_tertiary } else { p.muted }))
                .child(text)
        };
        // r2-pages: v2 cell actions are the shared ghost button.
        let quiet = |id: SharedString, label: &'static str, icon: IconName, enabled: bool| {
            if v2 {
                super::primitives::button(
                    id,
                    label,
                    Some(icon),
                    super::primitives::ButtonVariant::Tertiary,
                    super::primitives::ButtonSize::Small,
                    enabled,
                    &p,
                )
            } else {
                self.pquiet(id, label, icon, enabled)
            }
        };
        let mut body = div().flex().flex_col().gap_1().min_w_0();
        match &view.state {
            // v2: the path, then how it is linked as a quiet line; no pills.
            CellState::Linked(path) if v2 => {
                body = body.child(mono(path.clone())).child(
                    muted("Linked".into())
                        .debug_selector(move || format!("link-linked-{row}-{col}")),
                );
            }
            CellState::Matched { path, via } if v2 => {
                body = body.child(mono(path.clone())).child(
                    muted(format!("Found · {}", via.label()))
                        .debug_selector(move || format!("link-matched-{row}-{col}")),
                );
            }
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
                                quiet(
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
                    muted(if v2 && view.offline && cell.choices.is_empty() {
                        "Can't see its folders while it is offline".to_owned()
                    } else if view.offline && cell.choices.is_empty() {
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
                quiet(
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
                quiet(
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
            .map(|d| {
                if v2 {
                    d.flex_1().flex_basis(px(0.)).min_w_0()
                } else {
                    d.w(px(280.))
                }
            })
            .map(|d| {
                if v2 {
                    // r2-pages: the shared inset; offline reads through its
                    // words, not a faded cell.
                    d.p(px(t.space.lg))
                        .rounded(px(t.radius.control))
                        .bg(super::primitives::tonal(&p, t.wash.disabled))
                        .border(px(t.stroke.hairline))
                        .border_color(rgb(p.border))
                        .flex()
                        .flex_col()
                        .gap(px(t.space.sm))
                } else {
                    d.p_2()
                        .rounded(px(p.control_radius))
                        .border_1()
                        .border_color(rgb(p.border))
                        .bg(rgb(p.chat))
                        .flex()
                        .flex_col()
                        .gap_2()
                        .when(view.offline, |d| d.opacity(0.55))
                }
            })
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
                    .when(view.offline && !v2, |d| {
                        d.child(super::hosts::pill("offline".into(), p.muted, p))
                    })
                    .when(view.offline && v2, |d| d.child(muted("Offline".into()))),
            )
            .child(body)
            // r2-pages: actions pinned to the cell's bottom edge.
            .when(v2, |d| d.child(div().flex_grow()))
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

    // r2-pages
    /// A v2 project row: name and path over one equal column per machine;
    /// every cell's actions sit on the cell's bottom edge, so a row's
    /// buttons share one baseline.
    fn render_link_row_v2(
        &self,
        row: usize,
        project: &Project,
        links: &[wks_native::project_links::Link],
        cx: &mut Context<Self>,
    ) -> Div {
        let p = self.palette();
        let t = p.tokens;
        let head = div()
            .flex()
            .items_baseline()
            .gap(px(t.space.md))
            .min_w_0()
            .child(
                div()
                    .flex_shrink_0()
                    .text_size(px(t.text.subheading))
                    .font_weight(FontWeight(t.weight.semibold))
                    .text_color(rgb(p.text))
                    .child(project.name.clone()),
            )
            .child(
                div()
                    .min_w_0()
                    .truncate()
                    .font_family(mono_font())
                    .text_size(px(t.text.meta))
                    .text_color(rgb(p.roles.text_tertiary))
                    .child(project.path.clone()),
            )
            .child(div().flex_1())
            .child(
                div()
                    .flex_shrink_0()
                    .max_w(px(t.layout.status_column * 2.))
                    .truncate()
                    .text_size(px(t.text.meta))
                    .text_color(rgb(p.roles.text_tertiary))
                    .child(project.remote.clone().unwrap_or_else(|| "No remote".into())),
            );
        let mut cells = div().flex().gap(px(t.space.md));
        for (col, cell) in project.peers.iter().enumerate() {
            let view = CellView::new(project, cell, links);
            cells = cells.child(self.render_link_cell(row, col, project, cell, &view, cx));
        }
        super::primitives::card(&p)
            .debug_selector(move || format!("project-row-{row}"))
            .gap(px(t.space.lg))
            .child(head)
            .child(cells)
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
