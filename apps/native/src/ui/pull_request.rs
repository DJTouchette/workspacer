//! The Pull request page: the selected session's linked GitHub or Azure
//! DevOps pull request, its review threads (reply, resolve, hand to the
//! agent), and linking, changing or unlinking it. Everything runs on the
//! hub, as the user signed in to `gh` / `az` there.
use super::*;
use gpui_component::input::Input;
use wks_native::{
    features::{PrAction, Request},
    pull_requests::{self, Link, Thread, Threads},
};

pub(super) const DESCRIPTION: &str = "Review comments from the pull request this session works on. Replies and resolutions are posted as you.";

pub(super) struct PrUi {
    pub url: Entity<InputState>,
    pub reply: Entity<InputState>,
    pub comment: Entity<InputState>,
    /// The thread whose reply box is open.
    pub replying: Option<String>,
    pub show_resolved: bool,
    /// Change the link even though one is set.
    pub relinking: bool,
}

impl PrUi {
    pub fn new(window: &mut Window, cx: &mut Context<Workspace>) -> Self {
        Self {
            url: cx.new(|cx| {
                InputState::new(window, cx)
                    .placeholder("Paste a GitHub or Azure DevOps pull request URL")
            }),
            reply: cx.new(|cx| {
                InputState::new(window, cx)
                    .auto_grow(2, 8)
                    .placeholder("Reply")
            }),
            comment: cx.new(|cx| {
                InputState::new(window, cx)
                    .auto_grow(2, 8)
                    .placeholder("Add a comment to the pull request")
            }),
            replying: None,
            show_resolved: false,
            relinking: false,
        }
    }
}

pub(super) fn icon() -> Icon {
    Icon::empty().path("lucide/git-pull-request.svg")
}

/// The colour a pull request's state reads in.
pub(super) fn state_color(link: &Link, p: Palette) -> u32 {
    match link.state.as_str() {
        "open" => p.success,
        "merged" => p.accent,
        "closed" => p.error,
        _ => p.muted,
    }
}

impl Workspace {
    /// Load what the page shows: the threads when linked, else a suggestion
    /// for the session's branch.
    pub(super) fn open_pull_request(&mut self, cx: &mut Context<Self>) {
        let Some(session) = self.selected_session().cloned() else {
            return;
        };
        self.pr.replying = None;
        self.pr.relinking = false;
        match &session.pull_request {
            Some(link) => self.request(
                Request::PrThreads {
                    url: link.url.clone(),
                },
                cx,
            ),
            None => self.suggest_pull_request(&session.cwd, cx),
        }
    }

    fn suggest_pull_request(&mut self, cwd: &str, cx: &mut Context<Self>) {
        if !cwd.is_empty() {
            self.request(
                Request::PrForBranch {
                    cwd: cwd.to_owned(),
                },
                cx,
            );
        }
    }

    /// The newest discussion for `url`: a plain refresh or the answer to a
    /// reply or resolution, whichever came last.
    fn pr_threads(&self, url: &str) -> (Option<Threads>, bool, Option<String>) {
        let read = self
            .view
            .requests
            .get("pr-threads")
            .filter(|s| matches!(&s.request, Request::PrThreads { url: u } if u == url));
        let wrote = self
            .view
            .requests
            .get("pr-action")
            .filter(|s| matches!(&s.request, Request::PrAction(a) if a.url() == url));
        let newest = match (read, wrote) {
            (Some(r), Some(w)) if w.number > r.number && !w.loading && w.error.is_none() => Some(w),
            (Some(r), _) => Some(r),
            (None, w) => w.filter(|w| !w.loading && w.error.is_none()),
        };
        let loading = read.is_some_and(|r| r.loading);
        let threads = newest
            .filter(|s| !s.value.is_null())
            .map(|s| Threads::parse(&s.value));
        let error = read.and_then(|r| r.error.clone());
        (threads, loading, error)
    }

    /// A finished request's error line under the control that made it.
    fn request_error(&self, key: &str, p: Palette) -> Option<Div> {
        let error = self.view.requests.get(key)?.error.clone()?;
        Some(chrome::notice_line(error, chrome::Tone::Error, p))
    }

    fn pr_busy(&self, key: &str) -> bool {
        self.view.requests.get(key).is_some_and(|s| s.loading)
    }

    fn link_pull_request(&mut self, url: String, cx: &mut Context<Self>) {
        let Some(session) = self.view.selected.clone() else {
            return;
        };
        self.pr.relinking = false;
        self.request(
            Request::PrLink {
                session,
                url: Some(url.clone()),
            },
            cx,
        );
        // The snapshot carries the link; the threads can load now.
        self.request(Request::PrThreads { url }, cx);
    }

    fn pr_action(&mut self, action: PrAction, cx: &mut Context<Self>) {
        self.request(Request::PrAction(action), cx);
    }

    /// "Send to agent": the threads, quoted, in the composer for the user to
    /// edit and send.
    fn send_threads_to_agent(
        &mut self,
        link: &Link,
        threads: &[Thread],
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if threads.is_empty() {
            return;
        }
        let refs: Vec<&Thread> = threads.iter().collect();
        let text = pull_requests::agent_brief(link, &refs);
        self.prefill(&text, window, cx);
    }

    pub(super) fn render_pull_request(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Div {
        let p = self.appearance.palette();
        let Some(session) = self.selected_session().cloned() else {
            return features::empty_note("Select a session in the sidebar first.", p);
        };
        let link = session.pull_request.clone();
        let mut page = div().flex().flex_col().gap_4();
        if let Some(link) = &link {
            page = page.child(self.render_pr_header(link, cx));
        }
        if link.is_none() || self.pr.relinking {
            page = page.child(self.render_pr_linker(&session, link.is_some(), cx));
        }
        if let Some(link) = &link {
            page = page.child(self.render_pr_threads(link, window, cx));
        }
        page
    }

    fn render_pr_header(&self, link: &Link, cx: &mut Context<Self>) -> Div {
        let p = self.appearance.palette();
        let connected = self.view.connected;
        let open = link.url.clone();
        let refresh = link.url.clone();
        let source = match link.source {
            pull_requests::Source::User => "Linked by you",
            pull_requests::Source::Branch => "Linked from the session's branch",
            pull_requests::Source::Agent => "Linked by the agent",
        };
        let title = if link.title.is_empty() {
            format!("{} {}", link.provider_label(), link.short())
        } else {
            link.title.clone()
        };
        chrome::card(p)
            .debug_selector(|| "pr-header".into())
            .p_4()
            .flex()
            .flex_col()
            .gap_3()
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap_2()
                    .text_size(px(chrome::scale::META))
                    .text_color(rgb(p.muted))
                    .child(format!("{} · {}", link.provider_label(), link.repo))
                    .child(div().flex_1())
                    .child(
                        div()
                            .px_2()
                            .py_0p5()
                            .rounded_full()
                            .bg(rgb(p.selected))
                            .font_weight(FontWeight::MEDIUM)
                            .text_color(rgb(state_color(link, p)))
                            .child(link.state_label()),
                    ),
            )
            .child(
                div()
                    .flex()
                    .items_baseline()
                    .gap_2()
                    .min_w_0()
                    .child(
                        div()
                            .flex_shrink_0()
                            .text_size(px(chrome::scale::HEADING))
                            .text_color(rgb(p.muted))
                            .child(link.short()),
                    )
                    .child(
                        div()
                            .min_w_0()
                            .text_size(px(chrome::scale::HEADING))
                            .font_weight(FontWeight::SEMIBOLD)
                            .child(title),
                    ),
            )
            .child(
                div()
                    .text_size(px(chrome::scale::META))
                    .text_color(rgb(p.muted))
                    .child(if link.source_branch.is_empty() {
                        source.to_owned()
                    } else {
                        format!("{} · {source}", link.source_branch)
                    }),
            )
            .child(
                div()
                    .flex()
                    .flex_wrap()
                    .items_center()
                    .gap_1()
                    .child(
                        self.quiet_button(
                            "pr-open",
                            "Open in browser",
                            IconName::ExternalLink,
                            true,
                        )
                        .on_click(move |_, _, cx| cx.open_url(&open)),
                    )
                    .child(
                        self.quiet_button("pr-refresh", "Refresh", IconName::Redo, connected)
                            .when(connected, |d| {
                                d.on_click(cx.listener(move |this, _, _, cx| {
                                    this.request(
                                        Request::PrThreads {
                                            url: refresh.clone(),
                                        },
                                        cx,
                                    )
                                }))
                            }),
                    )
                    .child(
                        self.quiet_button("pr-change", "Change", IconName::Replace, connected)
                            .when(connected, |d| {
                                d.on_click(cx.listener(|this, _, _, cx| {
                                    this.pr.relinking = !this.pr.relinking;
                                    if let Some(cwd) =
                                        this.selected_session().map(|s| s.cwd.clone())
                                    {
                                        this.suggest_pull_request(&cwd, cx);
                                    }
                                    cx.notify();
                                }))
                            }),
                    )
                    .child(div().flex_1())
                    .child(
                        self.danger_button(
                            "pr-unlink",
                            "Unlink",
                            connected && !self.pr_busy("pr-link"),
                        )
                        .debug_selector(|| "pr-unlink".into())
                        .when(connected, |d| {
                            d.on_click(cx.listener(|this, _, _, cx| {
                                if let Some(session) = this.view.selected.clone() {
                                    this.request(Request::PrLink { session, url: None }, cx);
                                }
                            }))
                        }),
                    ),
            )
    }

    /// Link a pull request: the one open for the session's branch, or any
    /// pasted URL.
    fn render_pr_linker(&self, session: &Session, relinking: bool, cx: &mut Context<Self>) -> Div {
        let p = self.appearance.palette();
        let connected = self.view.connected;
        let branch =
            self.view.requests.get("pr-branch").filter(
                |s| matches!(&s.request, Request::PrForBranch { cwd } if *cwd == session.cwd),
            );
        let suggestion = branch
            .filter(|s| !s.loading)
            .and_then(|s| Link::parse(&s.value["pullRequest"]))
            .filter(|found| {
                session
                    .pull_request
                    .as_ref()
                    .is_none_or(|current| current.url != found.url)
            });
        let detected = branch.map(|s| &s.value["detected"]);
        let status: Option<(String, chrome::Tone)> = match branch {
            Some(s) if s.loading => Some((
                "Looking for a pull request on this branch…".into(),
                chrome::Tone::Loading,
            )),
            Some(s) => match (&s.error, suggestion.as_ref()) {
                (Some(error), _) => Some((error.clone(), chrome::Tone::Warning)),
                (None, Some(_)) => None,
                (None, None) => Some((
                    detected
                        .and_then(|d| d["problem"].as_str())
                        .map(str::to_owned)
                        .unwrap_or_else(|| match detected.and_then(|d| d["branch"].as_str()) {
                            Some(branch) => format!("No open pull request for {branch}."),
                            None => "No branch is checked out here.".into(),
                        }),
                    chrome::Tone::Info,
                )),
            },
            None => None,
        };
        let url_input = self.pr.url.clone();
        let mut card = chrome::card(p)
            .debug_selector(|| "pr-linker".into())
            .p_4()
            .flex()
            .flex_col()
            .gap_3()
            .child(features::card_heading(
                if relinking {
                    "Change the linked pull request"
                } else {
                    "Link a pull request"
                },
                Some(
                    "The agent can read its comments, reply and resolve threads, and hears about new comments while it works.",
                ),
                p,
            ));
        if let Some(found) = suggestion {
            let url = found.url.clone();
            card = card.child(
                div()
                    .debug_selector(|| "pr-suggestion".into())
                    .flex()
                    .items_center()
                    .gap_3()
                    .p_3()
                    .rounded(px(p.control_radius))
                    .bg(rgb(p.selected))
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .flex()
                            .flex_col()
                            .gap_0p5()
                            .child(
                                div()
                                    .truncate()
                                    .font_weight(FontWeight::MEDIUM)
                                    .child(format!("{} {}", found.short(), found.title)),
                            )
                            .child(
                                div()
                                    .text_size(px(chrome::scale::META))
                                    .text_color(rgb(p.muted))
                                    .child(format!(
                                        "Open for {} on {}",
                                        found.source_branch,
                                        found.provider_label()
                                    )),
                            ),
                    )
                    .child(
                        self.primary_button("pr-link-suggested", "Link", connected)
                            .debug_selector(|| "pr-link-suggested".into())
                            .when(connected, |d| {
                                d.on_click(cx.listener(move |this, _, _, cx| {
                                    this.link_pull_request(url.clone(), cx)
                                }))
                            }),
                    ),
            );
        }
        if let Some((text, tone)) = status {
            card = card.child(chrome::notice_line(text, tone, p));
        }
        card.child(
            div()
                .flex()
                .items_center()
                .gap_2()
                .child(div().flex_1().min_w_0().child(Input::new(&self.pr.url)))
                .child(
                    self.button_style("pr-link-url", "Link URL", connected, false)
                        .when(connected, |d| {
                            d.on_click(cx.listener(move |this, _, window, cx| {
                                let url = url_input.read(cx).value().trim().to_owned();
                                if url.is_empty() {
                                    return;
                                }
                                url_input.update(cx, |input, cx| input.set_value("", window, cx));
                                this.link_pull_request(url, cx);
                            }))
                        }),
                ),
        )
        .children(self.request_error("pr-link", p))
    }

    fn render_pr_threads(
        &mut self,
        link: &Link,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Div {
        let p = self.appearance.palette();
        let connected = self.view.connected;
        let (threads, loading, error) = self.pr_threads(&link.url);
        let mut column = div().flex().flex_col().gap_3();
        if let Some(error) = error {
            column = column.child(chrome::notice_line(error, chrome::Tone::Error, p));
        }
        let Some(threads) = threads else {
            return column.child(if loading {
                chrome::notice_line(
                    "Loading review comments…".to_owned(),
                    chrome::Tone::Loading,
                    p,
                )
            } else {
                div()
            });
        };
        let open: Vec<Thread> = threads
            .threads
            .iter()
            .filter(|t| t.open())
            .cloned()
            .collect();
        let resolved = threads.threads.iter().filter(|t| t.resolved).count();
        let show_resolved = self.pr.show_resolved;
        let brief_link = link.clone();
        let brief_threads = open.clone();
        column = column.child(
            div()
                .flex()
                .items_center()
                .gap_2()
                .child(
                    div()
                        .flex_1()
                        .text_size(px(chrome::scale::META))
                        .text_color(rgb(p.muted))
                        .child(format!(
                            "{} unresolved · {} resolved{}",
                            open.len(),
                            resolved,
                            if threads.truncated {
                                " · more on the provider"
                            } else {
                                ""
                            }
                        )),
                )
                .when(resolved > 0, |d| {
                    d.child(
                        self.quiet_button(
                            "pr-toggle-resolved",
                            if show_resolved {
                                "Hide resolved"
                            } else {
                                "Show resolved"
                            },
                            IconName::EyeOff,
                            true,
                        )
                        .on_click(cx.listener(|this, _, _, cx| {
                            this.pr.show_resolved = !this.pr.show_resolved;
                            cx.notify();
                        })),
                    )
                })
                .child(
                    self.pill_button(
                        "pr-send-unresolved",
                        "Send unresolved to agent",
                        IconName::Bot,
                        !open.is_empty(),
                        true,
                    )
                    .debug_selector(|| "pr-send-unresolved".into())
                    .when(!open.is_empty(), |d| {
                        d.on_click(cx.listener(move |this, _, window, cx| {
                            this.send_threads_to_agent(&brief_link, &brief_threads, window, cx)
                        }))
                    }),
                ),
        );
        let visible: Vec<Thread> = threads
            .threads
            .iter()
            .filter(|t| show_resolved || !t.resolved)
            .cloned()
            .collect();
        if visible.is_empty() {
            column = column.child(features::empty_note("No open review comments.", p));
        }
        for thread in visible {
            column = column.child(self.render_pr_thread(link, &thread, connected, window, cx));
        }
        let comment = self.pr.comment.clone();
        let url = link.url.clone();
        column.child(
            chrome::card(p)
                .p_3()
                .flex()
                .flex_col()
                .gap_2()
                .child(Input::new(&self.pr.comment))
                .child(
                    div().flex().justify_end().child(
                        self.button_style(
                            "pr-comment-post",
                            "Comment",
                            connected && !self.pr_busy("pr-action"),
                            false,
                        )
                        .when(connected, |d| {
                            d.on_click(cx.listener(move |this, _, window, cx| {
                                let body = comment.read(cx).value().trim().to_owned();
                                if body.is_empty() {
                                    return;
                                }
                                comment.update(cx, |input, cx| input.set_value("", window, cx));
                                this.pr_action(
                                    PrAction::Comment {
                                        url: url.clone(),
                                        body,
                                    },
                                    cx,
                                );
                            }))
                        }),
                    ),
                )
                .children(self.request_error("pr-action", p)),
        )
    }

    fn render_pr_thread(
        &mut self,
        link: &Link,
        thread: &Thread,
        connected: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Div {
        let p = self.appearance.palette();
        let id = thread.id.clone();
        let replying = self.pr.replying.as_deref() == Some(id.as_str());
        let busy = self.pr_busy("pr-action");
        let key = format!("pr-thread-{id}");
        let mut card = chrome::card(p)
            .debug_selector({
                let key = key.clone();
                move || key.clone()
            })
            .p_3()
            .flex()
            .flex_col()
            .gap_2()
            .when(thread.resolved, |d| d.opacity(0.7))
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap_2()
                    .text_size(px(chrome::scale::META))
                    .child(
                        Icon::new(if thread.path.is_some() {
                            IconName::File
                        } else {
                            IconName::Inbox
                        })
                        .size(px(12.))
                        .text_color(rgb(p.muted)),
                    )
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .truncate()
                            .font_family(mono_font())
                            .text_color(rgb(p.muted))
                            .child(thread.location()),
                    )
                    .when(thread.outdated, |d| {
                        d.child(div().text_color(rgb(p.warning)).child("Outdated"))
                    })
                    .when(thread.resolved, |d| {
                        d.child(div().text_color(rgb(p.success)).child("Resolved"))
                    }),
            );
        for (i, comment) in thread.comments.iter().enumerate() {
            card = card.child(
                div()
                    .flex()
                    .flex_col()
                    .gap_1()
                    .when(i > 0, |d| d.pt_2().border_t_1().border_color(rgb(p.border)))
                    .child(
                        div()
                            .text_size(px(chrome::scale::META))
                            .font_weight(FontWeight::SEMIBOLD)
                            .child(if comment.author.is_empty() {
                                "Someone".to_owned()
                            } else {
                                comment.author.clone()
                            }),
                    )
                    .child(self.render_markdown(
                        &format!("{key}-{}", comment.id),
                        &comment.body,
                        window,
                        cx,
                    )),
            );
        }
        let send_link = link.clone();
        let send_thread = thread.clone();
        let mut actions = div()
            .flex()
            .flex_wrap()
            .items_center()
            .gap_1()
            .child(
                self.quiet_button(
                    SharedString::from(format!("{key}-reply")),
                    if replying { "Cancel reply" } else { "Reply" },
                    IconName::ArrowLeft,
                    connected,
                )
                .debug_selector({
                    let key = key.clone();
                    move || format!("{key}-reply")
                })
                .when(connected, |d| {
                    let id = id.clone();
                    d.on_click(cx.listener(move |this, _, window, cx| {
                        this.pr.replying = if this.pr.replying.as_deref() == Some(id.as_str()) {
                            None
                        } else {
                            this.pr.reply.update(cx, |input, cx| {
                                input.set_value("", window, cx);
                                input.focus(window, cx);
                            });
                            Some(id.clone())
                        };
                        cx.notify();
                    }))
                }),
            )
            .child(
                self.quiet_button(
                    SharedString::from(format!("{key}-agent")),
                    "Send to agent",
                    IconName::Bot,
                    true,
                )
                .debug_selector({
                    let key = key.clone();
                    move || format!("{key}-agent")
                })
                .on_click(cx.listener(move |this, _, window, cx| {
                    this.send_threads_to_agent(
                        &send_link,
                        std::slice::from_ref(&send_thread),
                        window,
                        cx,
                    )
                })),
            );
        if thread.resolvable {
            let url = link.url.clone();
            let resolve_id = id.clone();
            let resolved = thread.resolved;
            actions = actions.child(div().flex_1()).child(
                self.quiet_button(
                    SharedString::from(format!("{key}-resolve")),
                    if resolved { "Reopen" } else { "Resolve" },
                    if resolved {
                        IconName::Undo
                    } else {
                        IconName::Check
                    },
                    connected && !busy,
                )
                .debug_selector({
                    let key = key.clone();
                    move || format!("{key}-resolve")
                })
                .when(connected && !busy, |d| {
                    d.on_click(cx.listener(move |this, _, _, cx| {
                        this.pr_action(
                            PrAction::Resolve {
                                url: url.clone(),
                                thread: resolve_id.clone(),
                                resolved: !resolved,
                            },
                            cx,
                        )
                    }))
                }),
            );
        }
        card = card.child(actions);
        if replying {
            let reply = self.pr.reply.clone();
            let url = link.url.clone();
            let thread_id = id.clone();
            card = card.child(
                div()
                    .flex()
                    .flex_col()
                    .gap_2()
                    .child(Input::new(&self.pr.reply))
                    .child(
                        div().flex().justify_end().child(
                            self.primary_button(
                                SharedString::from(format!("{key}-reply-send")),
                                "Post reply",
                                connected && !busy,
                            )
                            .debug_selector({
                                let key = key.clone();
                                move || format!("{key}-reply-send")
                            })
                            .when(connected && !busy, |d| {
                                d.on_click(cx.listener(move |this, _, window, cx| {
                                    let body = reply.read(cx).value().trim().to_owned();
                                    if body.is_empty() {
                                        return;
                                    }
                                    reply.update(cx, |input, cx| input.set_value("", window, cx));
                                    this.pr.replying = None;
                                    this.pr_action(
                                        PrAction::Reply {
                                            url: url.clone(),
                                            thread: thread_id.clone(),
                                            body,
                                        },
                                        cx,
                                    );
                                }))
                            }),
                        ),
                    ),
            );
        }
        card
    }
}
