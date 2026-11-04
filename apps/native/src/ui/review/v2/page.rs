//! The full Changes page under v2: the stacked diff beside the file list
//! (or the project tree, or the commit log) and the commit box.
use super::*;

impl Workspace {
    pub(in crate::ui) fn render_review_v2(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Stateful<Div> {
        let p = self.palette();
        let t = p.tokens;
        let back = primitives::back_link("feature-back", "Back to chat", &p)
            .debug_selector(|| "feature-back".into())
            .on_click(cx.listener(|this, _, window, cx| this.back_from_feature(window, cx)));
        let paths = self.review_paths();
        let branch = self
            .review_status()
            .and_then(|s| s.branch.clone())
            .unwrap_or_else(|| "Repository".into());
        let panel_open = self.settings.review_panel;
        let mut actions = Vec::new();
        if let Some((cwd, root)) = paths.clone() {
            let session = self.view.selected.clone().unwrap_or_default();
            actions.push(
                primitives::button(
                    "review-editor",
                    "Open editor",
                    Some(IconName::FolderOpen),
                    ButtonVariant::Secondary,
                    ButtonSize::Small,
                    true,
                    &p,
                )
                .debug_selector(|| "review-editor".into())
                .on_click(cx.listener(move |this, _, window, cx| {
                    this.open_editor(&session, &root, window, cx)
                }))
                .into_any_element(),
            );
            actions.push(
                primitives::icon_button(
                    "review-panel",
                    if panel_open {
                        IconName::PanelRightClose
                    } else {
                        IconName::PanelRightOpen
                    },
                    if panel_open {
                        "Hide files"
                    } else {
                        "Show files"
                    },
                    ButtonVariant::Ghost,
                    true,
                    &p,
                )
                .debug_selector(|| "review-panel".into())
                .on_click(cx.listener(move |this, _, _, cx| {
                    this.settings.review_panel = !panel_open;
                    this.save_settings(cx);
                    cx.notify();
                }))
                .into_any_element(),
            );
            let connected = self.view.connected;
            actions.push(
                primitives::icon_button(
                    "changes-refresh",
                    IconName::Redo,
                    "Refresh",
                    ButtonVariant::Ghost,
                    connected,
                    &p,
                )
                .debug_selector(|| "changes-refresh".into())
                .when(connected, |d| {
                    d.on_click(cx.listener(move |this, _, _, cx| {
                        this.request(Request::Changes { cwd: cwd.clone() }, cx);
                        if this.review.mode == Mode::Commits {
                            this.request(Request::Log { cwd: cwd.clone() }, cx);
                        }
                        if let Some((_, root)) = this.review_paths()
                            && this.review.mode == Mode::Files
                        {
                            this.refresh_explorer(&root, cx);
                        }
                    }))
                })
                .into_any_element(),
            );
        }
        let description = paths
            .as_ref()
            .map(|(_, root)| SharedString::from(format!("{root} · {branch}")));
        let header = primitives::page_header(
            Some(back.into_any_element()),
            "Changes",
            description,
            actions,
            &p,
        )
        .debug_selector(|| "page-header".into());
        let notice = (!self.extras.notice.is_empty()).then(|| {
            let tone = chrome::notice_tone(&self.extras.notice);
            chrome::notice_line(self.extras.notice.clone(), tone, p)
                .debug_selector(|| "feature-notice".into())
        });
        let body: AnyElement = match paths {
            None => primitives::empty_state(
                IconName::Replace,
                "Nothing to review",
                Some("Select a session to see the changes in its folder.".into()),
                &p,
            )
            .into_any_element(),
            Some((_, root)) => {
                let changes = self.review_changes();
                let diff = self.page_diff_card(cx);
                div()
                    .flex_1()
                    .min_h_0()
                    .flex()
                    .gap(px(t.space.lg))
                    .child(diff)
                    .when(panel_open, |d| {
                        d.child(self.page_files_card(&changes, &root, cx))
                    })
                    .into_any_element()
            }
        };
        let focus = self.review_focus(cx);
        let caption = chrome::custom_caption();
        div()
            .id("review-view")
            .debug_selector(|| "review-view".into())
            .track_focus(&focus)
            .on_key_down(cx.listener(|this, event: &KeyDownEvent, window, cx| {
                this.review_keys(event, window, cx)
            }))
            .relative()
            .flex_1()
            .min_w_0()
            .h_full()
            .bg(rgb(p.chat))
            .flex()
            .flex_col()
            .px(px(t.layout.page_gutter))
            .pt(px(if caption {
                chrome::PAGE_CAPTION_INSET
            } else {
                t.space.xxl
            }))
            .pb(px(t.space.xl))
            .gap(px(t.space.lg))
            .children(chrome::page_drag_strip())
            .child(header)
            .children(notice)
            .child(self.feature_message("changes"))
            .child(self.feature_message("git-action"))
            .child(body)
            .children(self.review_overlays(window, cx))
    }

    /// The page's diff card: a toolbar naming what is shown, then the stack.
    fn page_diff_card(&mut self, cx: &mut Context<Self>) -> Div {
        let p = self.palette();
        let t = p.tokens;
        let commits = self.review.mode == Mode::Commits;
        let slot = if commits { COMMIT } else { SET };
        let list = self.render_stack(slot, cx);
        let summary = self.stack_summary(slot);
        let title: AnyElement = if commits {
            let hash = self.review.commit.clone().unwrap_or_default();
            let subject = self
                .commit_rows()
                .into_iter()
                .find(|(h, ..)| h == &hash)
                .map(|(_, subject, _)| subject)
                .unwrap_or_default();
            div()
                .flex_1()
                .min_w_0()
                .flex()
                .items_center()
                .gap(px(t.space.md))
                .child(
                    div()
                        .flex_shrink_0()
                        .font_family(mono_font())
                        .text_size(px(t.text.caption))
                        .text_color(rgb(p.roles.text_tertiary))
                        .child(hash),
                )
                .child(
                    div()
                        .debug_selector(|| "review-commit-title".into())
                        .min_w_0()
                        .truncate()
                        .text_size(px(t.text.subheading))
                        .font_weight(FontWeight(t.weight.semibold))
                        .text_color(rgb(p.text))
                        .child(subject),
                )
                .into_any_element()
        } else {
            let files = summary.map_or(0, |s| s.0);
            div()
                .debug_selector(|| "review-diff-title".into())
                .flex_1()
                .min_w_0()
                .truncate()
                .text_size(px(t.text.subheading))
                .font_weight(FontWeight(t.weight.semibold))
                .text_color(rgb(p.text))
                .child(if files == 0 {
                    "Working tree".to_owned()
                } else {
                    format!("{files} changed file{}", if files == 1 { "" } else { "s" })
                })
                .into_any_element()
        };
        let counts = summary
            .filter(|s| s.1 + s.2 > 0)
            .map(|(_, added, removed)| counts(added, removed, &p));
        div()
            .debug_selector(|| "review-diff-card".into())
            .flex_1()
            .min_w_0()
            .h_full()
            .flex()
            .flex_col()
            .rounded(px(t.radius.panel))
            .bg(rgb(p.surface))
            .border(px(t.stroke.hairline))
            .border_color(rgb(p.roles.card_border))
            .overflow_hidden()
            .child(
                div()
                    .flex_shrink_0()
                    .h(px(t.panels.header_height))
                    .pl(px(t.space.lg))
                    .pr(px(t.space.sm))
                    .flex()
                    .items_center()
                    .gap(px(t.space.md))
                    .border_b(px(t.stroke.hairline))
                    .border_color(rgb(p.border))
                    .child(title)
                    .children(counts)
                    .child(self.stack_controls(cx)),
            )
            .child(list)
    }

    /// The page's right-hand card: Changed / Files / Commits.
    fn page_files_card(
        &mut self,
        changes: &Arc<Vec<Change>>,
        root: &str,
        cx: &mut Context<Self>,
    ) -> Div {
        let p = self.palette();
        let t = p.tokens;
        let mode = self.review.mode;
        let segment = |id: &'static str, label: String, target: Mode| {
            primitives::segment(id, label, mode == target, &p)
                .debug_selector(move || id.into())
                .on_click(cx.listener(move |this, _, _, cx| this.set_review_mode(target, cx)))
        };
        let toggle = div().p(px(t.space.md)).child(
            primitives::segmented(&p)
                .child(segment(
                    "review-mode-changed",
                    format!("Changed {}", changes.len()),
                    Mode::Changed,
                ))
                .child(segment(
                    "review-mode-all",
                    if self.explorer.show_ignored {
                        "All files".to_owned()
                    } else {
                        "Files".to_owned()
                    },
                    Mode::Files,
                ))
                .child(segment(
                    "review-mode-commits",
                    "Commits".to_owned(),
                    Mode::Commits,
                )),
        );
        let card = div()
            .debug_selector(|| "review-files".into())
            .w(px(t.panels.files_width))
            .flex_shrink_0()
            .h_full()
            .flex()
            .flex_col()
            .rounded(px(t.radius.panel))
            .bg(rgb(p.surface))
            .border(px(t.stroke.hairline))
            .border_color(rgb(p.roles.card_border))
            .overflow_hidden()
            .child(toggle);
        match mode {
            Mode::Files => card.child(
                div()
                    .id("review-files-scroll")
                    .flex_1()
                    .min_h_0()
                    .flex()
                    .flex_col()
                    .child(self.render_review_tree(changes, root, cx)),
            ),
            Mode::Commits => card.child(self.render_commit_list(cx)),
            Mode::Changed => {
                let list: AnyElement = if changes.is_empty() {
                    let loading = self.view.requests.get("changes").is_some_and(|s| s.loading);
                    div()
                        .flex_1()
                        .px(px(t.space.lg))
                        .py(px(t.space.md))
                        .text_size(px(t.text.caption))
                        .text_color(rgb(p.roles.text_tertiary))
                        .child(if loading && self.review_status().is_none() {
                            "Reading changes…"
                        } else {
                            "No uncommitted changes."
                        })
                        .into_any_element()
                } else {
                    let rows = changes.clone();
                    let busy = self.git_busy() || !self.view.connected;
                    uniform_list(
                        "review-changes",
                        rows.len(),
                        cx.processor(move |this, range: Range<usize>, _, cx| {
                            range
                                .map(|ix| this.change_row_v2(ix, &rows[ix], busy, cx))
                                .collect::<Vec<_>>()
                        }),
                    )
                    .track_scroll(self.review.list_scroll.clone())
                    .debug_selector(|| "review-changes".into())
                    .flex_1()
                    .min_h_0()
                    .px(px(t.space.xs))
                    .into_any_element()
                };
                card.child(list).when(!changes.is_empty(), |d| {
                    d.child(self.commit_box(changes, root, cx))
                })
            }
        }
    }

    /// A changed file in the page's list: choosing it scrolls the stack to
    /// it.
    fn change_row_v2(
        &mut self,
        ix: usize,
        change: &Change,
        busy: bool,
        cx: &mut Context<Self>,
    ) -> Stateful<Div> {
        let p = self.palette();
        let t = p.tokens;
        let active = self.review.selected.as_ref() == Some(&change.path);
        let (name, dir) = match change.path.rsplit_once(['/', '\\']) {
            Some((dir, name)) => (name.to_owned(), dir.to_owned()),
            None => (change.path.clone(), String::new()),
        };
        let stage = change.has_unstaged();
        let cwd = self.review_paths().map(|(cwd, _)| cwd).unwrap_or_default();
        let path = Some(change.path.clone());
        let action = if stage {
            GitAction::Stage { cwd, path }
        } else {
            GitAction::Unstage { cwd, path }
        };
        let badges = change_badges(change);
        let (label, intent) = badges
            .last()
            .copied()
            .unwrap_or(("Modified", Intent::Neutral));
        let clicked = change.path.clone();
        primitives::list_row(("review-change", ix), active, true, &p)
            .debug_selector({
                let name = format!("review-change-{}", change.path);
                move || name
            })
            .on_click(cx.listener(move |this, _, _, cx| this.scroll_stack_to(&clicked, cx)))
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .flex()
                    .items_baseline()
                    .gap(px(t.space.sm))
                    .child(div().min_w_0().truncate().child(name))
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .truncate()
                            .text_size(px(t.text.caption))
                            .text_color(rgb(p.roles.text_tertiary))
                            .child(dir),
                    ),
            )
            .child(
                div()
                    .flex_shrink_0()
                    .text_size(px(t.text.caption))
                    .text_color(rgb(if intent == Intent::Neutral {
                        p.roles.text_tertiary
                    } else {
                        intent.color(&p)
                    }))
                    .child(label),
            )
            .child(
                primitives::icon_button(
                    ("review-stage", ix),
                    if stage {
                        IconName::Plus
                    } else {
                        IconName::Minus
                    },
                    if stage { "Stage" } else { "Unstage" },
                    ButtonVariant::Ghost,
                    !busy,
                    &p,
                )
                .debug_selector({
                    let name = format!("review-stage-{}", change.path);
                    move || name
                })
                .when(!busy, |d| {
                    d.on_click(cx.listener(move |this, _, _, cx| {
                        cx.stop_propagation();
                        this.git_action(action.clone(), cx)
                    }))
                }),
            )
    }

    /// Bring `path`'s section to the top of the stack.
    fn scroll_stack_to(&mut self, path: &str, cx: &mut Context<Self>) {
        self.review.selected = Some(path.to_owned());
        let file = self
            .review
            .v2
            .cache
            .as_ref()
            .filter(|c| c.slot == SET)
            .and_then(|c| c.sections.iter().position(|s| s.path == path));
        if let (Some(file), Some((_, layout))) = (file, &self.review.v2.layout)
            && let Some(&header) = layout.headers.get(file)
        {
            self.review
                .v2
                .scroll
                .scroll_to_item_strict(header, gpui::ScrollStrategy::Top);
        }
        cx.notify();
    }
}
