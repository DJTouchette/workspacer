//! Changes under design v2: every changed file stacked in one list with
//! sticky per-file headers, git's header lines hidden, unchanged regions
//! folded, unified or side-by-side lines, `]c` / `[c` hunk jumps and
//! tinted lines with stronger intra-line emphasis. It shows in a resizable
//! dock beside the conversation (the tool rail's Changes, `g d`) and on the
//! full page (the dock's expand button, commit links), which adds the file
//! list, the project tree and the commit log.
use super::stack::{self, FileChange, Item, Layout, Section};
use super::*;
use crate::ui::primitives::{self, ButtonSize, ButtonVariant, Intent, tonal, wash};
use gpui::{
    Bounds, HighlightStyle, Hsla, ListHorizontalSizingBehavior, Pixels, UniformListDecoration,
    WeakEntity, anchored, canvas, deferred, point,
};
use std::ops::Range;
use wks_native::features::{DIFF_SET_MAX_FILES, DiffTarget};

mod page;

/// The slot the working tree's stacked diff answers in.
const SET: &str = "diff-set";
/// The slot a commit's patch answers in (shared with v1's single diffs).
const COMMIT: &str = "diff";

/// Design v2 Changes state, kept on [`ReviewUi`].
#[derive(Default)]
pub(in crate::ui) struct StackUi {
    /// The dock is open (it shows while the conversation is on screen).
    pub(in crate::ui) docked: bool,
    /// The next `open_feature(Changes)` opens the page, not the dock.
    pub(in crate::ui) page_once: bool,
    cache: Option<Cache>,
    coloring: Option<gpui::Task<()>>,
    layout: Option<(LayoutKey, Arc<Layout>)>,
    collapsed: BTreeSet<String>,
    collapsed_rev: u64,
    menu: Option<ReviewMenu>,
    /// A discard waiting for its confirmation dialog.
    confirm: Option<DiscardTarget>,
    /// A drag of the dock's leading edge: pointer x and width at start.
    drag: Option<(Pixels, f32)>,
    focus: Option<FocusHandle>,
    /// `]` or `[` pressed; `c` completes the jump (true = forward).
    bracket: Option<bool>,
    scroll: gpui::UniformListScrollHandle,
    opened: u64,
}

/// The overflow menu of one file header, at the click.
#[derive(Clone, Debug)]
pub(in crate::ui) struct ReviewMenu {
    file: usize,
    position: gpui::Point<Pixels>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct DiscardTarget {
    cwd: String,
    path: String,
    untracked: bool,
}

/// What a working-tree file's diff was read from.
#[derive(Clone, Debug, Default)]
struct FileInfo {
    path: String,
    error: Option<String>,
}

/// A parsed stacked diff, for one answer and one look.
#[derive(Clone)]
struct Cache {
    slot: &'static str,
    number: u64,
    look: (Appearance, Design),
    diff: Arc<diff::Diff>,
    sections: Arc<Vec<Section>>,
    emphasis: Arc<Vec<Option<Range<usize>>>>,
    highlights: Arc<syntax::RowHighlights>,
    files: Arc<Vec<FileInfo>>,
}

#[derive(Clone, PartialEq)]
struct LayoutKey {
    slot: &'static str,
    number: u64,
    split: bool,
    collapsed: u64,
    slots: usize,
}

/// Everything a sticky file header draws, prepared once per frame.
#[derive(Clone)]
struct HeaderData {
    path: String,
    name: String,
    dir: String,
    renamed_from: Option<String>,
    badges: Vec<(&'static str, Intent)>,
    added: usize,
    removed: usize,
    collapsed: bool,
    /// Stage (true) or unstage it, and the git write that does it.
    stage: Option<(bool, GitAction)>,
    open: Option<String>,
}

/// Join a `diff-set` answer into one patch, each file under its own
/// `diff --git` line so sections map one to one onto the files read.
fn join_set(value: &serde_json::Value) -> (String, Vec<FileInfo>) {
    let mut text = String::new();
    let mut files = Vec::new();
    for file in value["files"].as_array().into_iter().flatten() {
        let path = file["path"].as_str().unwrap_or_default().to_owned();
        let diff = file["diff"].as_str().unwrap_or_default();
        if !diff.starts_with("diff --git ") {
            text.push_str(&format!("diff --git a/{path} b/{path}\n"));
        }
        text.push_str(diff);
        if !text.ends_with('\n') {
            text.push('\n');
        }
        files.push(FileInfo {
            path,
            error: file["error"].as_str().map(str::to_owned),
        });
    }
    (text, files)
}

/// The badges for a working-tree change ("Modified", "New", "Staged").
fn change_badges(change: &Change) -> Vec<(&'static str, Intent)> {
    let mut out = vec![match change.letter {
        '?' | 'A' => ("New", Intent::Success),
        'D' => ("Deleted", Intent::Danger),
        'R' => ("Renamed", Intent::Info),
        'U' => ("Conflict", Intent::Warning),
        _ => ("Modified", Intent::Neutral),
    }];
    if change.has_staged() {
        out.push(if change.has_unstaged() {
            ("Partly staged", Intent::Neutral)
        } else {
            ("Staged", Intent::Neutral)
        });
    }
    out
}

/// The badge for a file of a commit, from its diff header.
fn section_badge(section: &Section) -> (&'static str, Intent) {
    match section.change {
        FileChange::Added => ("New", Intent::Success),
        FileChange::Deleted => ("Deleted", Intent::Danger),
        FileChange::Renamed => ("Renamed", Intent::Info),
        FileChange::Modified => ("Modified", Intent::Neutral),
    }
}

/// Merge syntax colours with the intra-line emphasis background: the
/// ranges are split where the emphasis starts and ends.
fn with_emphasis(
    text_len: usize,
    syntax: &[(Range<usize>, HighlightStyle)],
    emphasis: Option<Range<usize>>,
    background: Hsla,
) -> Vec<(Range<usize>, HighlightStyle)> {
    let Some(em) = emphasis.filter(|e| e.end <= text_len && !e.is_empty()) else {
        return syntax.to_vec();
    };
    let mut cuts = vec![0, text_len, em.start, em.end];
    for (r, _) in syntax {
        cuts.push(r.start.min(text_len));
        cuts.push(r.end.min(text_len));
    }
    cuts.sort_unstable();
    cuts.dedup();
    let mut out = Vec::new();
    for pair in cuts.windows(2) {
        let (a, b) = (pair[0], pair[1]);
        if a >= b {
            continue;
        }
        let mut style = syntax
            .iter()
            .find(|(r, _)| r.start <= a && b <= r.end)
            .map(|(_, s)| *s)
            .unwrap_or_default();
        if em.start <= a && b <= em.end {
            style.background_color = Some(background);
        }
        if style != HighlightStyle::default() {
            out.push((a..b, style));
        }
    }
    out
}

/// "+12 −4" in the diff tones.
fn counts(added: usize, removed: usize, p: &Palette) -> Div {
    let t = p.tokens;
    div()
        .flex_shrink_0()
        .flex()
        .gap(px(t.space.xs))
        .font_family(mono_font())
        .text_size(px(t.text.caption))
        .child(div().text_color(rgb(p.success)).child(format!("+{added}")))
        .child(div().text_color(rgb(p.error)).child(format!("−{removed}")))
}

/// Colours of one diff line kind: (line tint, emphasis, bar, marker).
fn tones(kind: Kind, p: &Palette) -> Option<(Hsla, Hsla, u32, &'static str)> {
    let t = p.tokens.panels;
    let tone = match kind {
        Kind::Added => p.success,
        Kind::Removed => p.error,
        _ => return None,
    };
    Some((
        wash(tone, t.diff_line_alpha),
        wash(tone, t.diff_emphasis_alpha),
        tone,
        if kind == Kind::Added { "+" } else { "−" },
    ))
}

/// The pieces of one side of a diff line: number gutter(s), marker, text.
#[allow(clippy::too_many_arguments)]
fn line_cells(
    row: &diff::Row,
    numbers: &[Option<u32>],
    highlights: &[(Range<usize>, HighlightStyle)],
    emphasis: Option<Range<usize>>,
    p: &Palette,
) -> Div {
    let t = p.tokens;
    let tone = tones(row.kind, p);
    let gutter = |n: Option<u32>| {
        div()
            .w(px(t.panels.diff_gutter))
            .flex_shrink_0()
            .pr(px(t.space.md))
            .flex()
            .justify_end()
            .text_size(px(t.text.overline))
            .text_color(rgb(p.roles.text_tertiary))
            .child(n.map(|n| n.to_string()).unwrap_or_default())
    };
    let text = if row.text.is_empty() {
        StyledText::new(" ")
    } else {
        let em_bg = tone.map_or_else(gpui::transparent_black, |(_, em, ..)| em);
        StyledText::new(row.text.clone()).with_highlights(with_emphasis(
            row.text.len(),
            highlights,
            emphasis,
            em_bg,
        ))
    };
    div()
        .h_full()
        .flex()
        .items_center()
        .whitespace_nowrap()
        .font_family(mono_font())
        // The bar keeps every line's text on one left edge.
        .border_l(px(t.panels.diff_bar))
        .border_color(match tone {
            Some((_, _, bar, _)) => Hsla::from(rgb(bar)),
            None => gpui::transparent_black(),
        })
        .when_some(tone, |d, (line, ..)| d.bg(line))
        .children(numbers.iter().map(|n| gutter(*n)))
        .child(
            div()
                .w(px(t.panels.diff_marker))
                .flex_shrink_0()
                .text_color(rgb(tone.map_or(p.roles.text_tertiary, |(_, _, c, _)| c)))
                .child(tone.map_or("", |(.., m)| m)),
        )
        .child(
            div()
                .pr(px(t.space.xl))
                .text_color(rgb(if row.kind == Kind::Note {
                    p.roles.text_tertiary
                } else {
                    p.prose
                }))
                .child(text),
        )
}

/// One slot of the stacked list.
fn stack_item(item: &Item, ix: usize, cache: &Cache, p: &Palette) -> Stateful<Div> {
    let t = p.tokens;
    let rows = &cache.diff.rows;
    let colors = |r: usize| cache.highlights.get(r).map_or(&[][..], Vec::as_slice);
    let em = |r: usize| cache.emphasis.get(r).cloned().flatten();
    let slot = div()
        .id(("stack-row", ix))
        .w_full()
        .min_w_full()
        .h(px(t.panels.diff_row));
    match item {
        Item::Gap => slot.bg(rgb(p.surface)),
        Item::Header(_) | Item::HeaderPad(_) => slot.bg(rgb(p.roles.raised)),
        Item::Fold {
            hidden, context, ..
        } => slot
            .flex()
            .items_center()
            .gap(px(t.space.md))
            .pl(px(t.panels.diff_gutter + t.panels.diff_bar))
            .pr(px(t.space.xl))
            .bg(tonal(p, t.wash.rest))
            .text_size(px(t.text.caption))
            .text_color(rgb(p.roles.text_tertiary))
            .whitespace_nowrap()
            .child(
                Icon::new(IconName::ChevronsUpDown)
                    .size(px(t.icon.xs))
                    .flex_shrink_0(),
            )
            .child(div().flex_shrink_0().child(format!(
                "{hidden} unchanged line{}",
                if *hidden == 1 { "" } else { "s" }
            )))
            .when(!context.is_empty(), |d| {
                d.child(div().min_w_0().truncate().child(context.clone()))
            }),
        Item::Line { row, .. } => {
            let r = &rows[*row];
            slot.child(line_cells(r, &[r.old, r.new], colors(*row), em(*row), p))
        }
        Item::Pair { old, new, .. } => {
            let half = |side: Option<usize>, old_side: bool| {
                let cell = div().flex_1().min_w_0().h_full().overflow_hidden();
                match side {
                    Some(r) => {
                        let row = &rows[r];
                        // A context row shows on both sides untinted.
                        let number = if old_side { row.old } else { row.new };
                        cell.child(line_cells(row, &[number], colors(r), em(r), p))
                    }
                    None => cell.bg(tonal(p, t.wash.disabled)),
                }
            };
            slot.flex()
                .child(half(*old, true))
                .child(div().w(px(t.stroke.hairline)).h_full().bg(rgb(p.border)))
                .child(half(*new, false))
        }
        Item::Placeholder { text, .. } => slot
            .flex()
            .items_center()
            .pl(px(t.panels.diff_gutter + t.panels.diff_bar))
            .text_size(px(t.text.caption))
            .text_color(rgb(p.roles.text_tertiary))
            .child(*text),
    }
}

/// Draws each visible file's header over the list, pinned to the top
/// while its file scrolls and pushed up by the next one.
struct StickyHeaders {
    files: Arc<Vec<HeaderData>>,
    headers: Arc<Vec<usize>>,
    ends: Arc<Vec<usize>>,
    slots: usize,
    busy: bool,
    p: Palette,
    view: WeakEntity<Workspace>,
}

impl UniformListDecoration for StickyHeaders {
    fn compute(
        &self,
        _visible: Range<usize>,
        bounds: Bounds<Pixels>,
        scroll: gpui::Point<Pixels>,
        item_height: Pixels,
        _count: usize,
        _window: &mut Window,
        _cx: &mut App,
    ) -> AnyElement {
        let top = -scroll.y;
        let bottom = top + bounds.size.height;
        let height = item_height * self.slots as f32;
        let mut layer = div().relative().size_full();
        for (f, data) in self.files.iter().enumerate() {
            let (Some(&start), Some(&end)) = (self.headers.get(f), self.ends.get(f)) else {
                continue;
            };
            let (start_y, end_y) = (item_height * start as f32, item_height * end as f32);
            if end_y <= top || start_y >= bottom {
                continue;
            }
            let y = start_y.max(top).min(end_y - height);
            layer = layer.child(
                file_header(f, data, self.busy, &self.p, self.view.clone())
                    .absolute()
                    .left(-scroll.x)
                    .top(y)
                    .w(bounds.size.width)
                    .h(height),
            );
        }
        layer.into_any_element()
    }
}

/// One file's header: collapse chevron, name and folder, status badges,
/// +/− counts, then Stage / Open / More.
fn file_header(
    f: usize,
    data: &HeaderData,
    busy: bool,
    p: &Palette,
    view: WeakEntity<Workspace>,
) -> Stateful<Div> {
    let t = p.tokens;
    let path = data.path.clone();
    let toggle = {
        let view = view.clone();
        let path = path.clone();
        div()
            .id(("stack-file-toggle", f))
            .debug_selector({
                let name = format!("review-stack-toggle-{path}");
                move || name
            })
            .flex_1()
            .min_w_0()
            .h_full()
            .flex()
            .items_center()
            .gap(px(t.space.sm))
            .cursor_pointer()
            .on_click(move |_, _, cx| {
                let _ = view.update(cx, |this, cx| this.toggle_stack_file(&path, cx));
            })
            .child(
                Icon::new(if data.collapsed {
                    IconName::ChevronRight
                } else {
                    IconName::ChevronDown
                })
                .size(px(t.icon.sm))
                .flex_shrink_0()
                .text_color(rgb(p.roles.text_tertiary)),
            )
            .child(
                div()
                    .min_w_0()
                    .truncate()
                    .text_size(px(t.text.body))
                    .font_weight(FontWeight(t.weight.semibold))
                    .text_color(rgb(p.text))
                    .child(data.name.clone()),
            )
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .truncate()
                    .text_size(px(t.text.caption))
                    .text_color(rgb(p.roles.text_tertiary))
                    .child(match &data.renamed_from {
                        Some(from) => format!("from {from}"),
                        None => data.dir.clone(),
                    }),
            )
    };
    let counts = div()
        .flex_shrink_0()
        .flex()
        .gap(px(t.space.xs))
        .font_family(mono_font())
        .text_size(px(t.text.caption))
        .when(data.added > 0, |d| {
            d.child(
                div()
                    .text_color(rgb(p.success))
                    .child(format!("+{}", data.added)),
            )
        })
        .when(data.removed > 0, |d| {
            d.child(
                div()
                    .text_color(rgb(p.error))
                    .child(format!("−{}", data.removed)),
            )
        });
    let stage = data.stage.clone().map(|(stage, action)| {
        let view = view.clone();
        primitives::button(
            ("stack-stage", f),
            if stage { "Stage" } else { "Unstage" },
            None,
            ButtonVariant::Tertiary,
            ButtonSize::Small,
            !busy,
            p,
        )
        .debug_selector({
            let name = format!("review-stack-stage-{path}");
            move || name
        })
        .when(!busy, |d| {
            d.on_click(move |_, _, cx| {
                let _ = view.update(cx, |this, cx| this.git_action(action.clone(), cx));
            })
        })
    });
    let open = data.open.clone().map(|target| {
        let view = view.clone();
        primitives::icon_button(
            ("stack-open", f),
            IconName::File,
            "Open in editor",
            ButtonVariant::Tertiary,
            true,
            p,
        )
        .debug_selector({
            let name = format!("review-stack-open-{path}");
            move || name
        })
        .on_click(move |_, _, cx| {
            let _ = view.update(cx, |this, cx| this.review_open_file(&target, cx));
        })
    });
    let more = {
        let view = view.clone();
        primitives::overflow_trigger(("stack-more", f), p)
            .debug_selector({
                let name = format!("review-stack-more-{path}");
                move || name
            })
            .on_click(move |event, _, cx| {
                let position = event.position();
                let _ = view.update(cx, |this, cx| {
                    this.review.v2.menu = Some(ReviewMenu { file: f, position });
                    cx.notify();
                });
            })
    };
    div()
        .id(("stack-header", f))
        .debug_selector({
            let name = format!("review-stack-header-{path}");
            move || name
        })
        .occlude()
        .flex()
        .items_center()
        .gap(px(t.space.md))
        .pl(px(t.space.md))
        .pr(px(t.space.sm))
        .bg(rgb(p.roles.raised))
        .border_t(px(t.stroke.hairline))
        .border_b(px(t.stroke.hairline))
        .border_color(rgb(p.border))
        .child(toggle)
        .children(
            data.badges
                .iter()
                .map(|(label, intent)| primitives::pill(*label, *intent, p)),
        )
        .child(counts)
        .children(stage)
        .children(open)
        .child(more)
}

impl StackUi {
    /// The stack's current layout, for tests.
    #[cfg(all(test, feature = "ui-tests"))]
    pub(in crate::ui) fn laid_out(&self) -> Option<Arc<Layout>> {
        self.layout.as_ref().map(|(_, l)| l.clone())
    }
}

impl Workspace {
    /// The Changes dock is on screen this frame.
    pub(in crate::ui) fn review_dock_showing(&self) -> bool {
        self.review.v2.docked
            && self.design().is_v2()
            && self.screen == Screen::Conversation
            && !self.new_session
            && self.view.selected.is_some()
            && !self.viewer_in_slot()
    }

    /// The dock's width beside the conversation (unzoomed), or `None` when
    /// the window cannot keep the conversation usable beside it.
    fn review_dock_width(&self, window: &Window) -> Option<f32> {
        let t = self.tokens().panels;
        let width = unzoom(window.viewport_size().width);
        let available = width - self.sidebar_footprint(width);
        let widest = (available - file_viewer::MIN_CHAT_BESIDE).min(t.dock_max);
        if widest < t.dock_min {
            return None;
        }
        Some(
            self.settings
                .review_dock_width
                .unwrap_or(available * t.dock_share)
                .clamp(t.dock_min, widest),
        )
    }

    /// Whether the window has room for the dock (beside, or below when
    /// tall).
    fn review_dock_fits(&self, window: &Window) -> bool {
        if self.portrait(window) {
            matches!(self.dock(window), Some(portrait::Dock::Below { .. }))
        } else {
            self.review_dock_width(window).is_some()
        }
    }

    /// `open_feature(Changes)` under v2: open the dock (or close it when it
    /// is open). False when the page should open instead: v1, a commit
    /// link or the expand button, no session, or no room.
    pub(in crate::ui) fn open_changes_dock(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> bool {
        if std::mem::take(&mut self.review.v2.page_once)
            || !self.design().is_v2()
            || self.new_session
            || self.view.selected.is_none()
            || !self.review_dock_fits(window)
        {
            return false;
        }
        if self.review_dock_showing() {
            self.close_review_dock(cx);
            return true;
        }
        // One panel at a time; the editor asks about unsaved edits first.
        if self.viewer_in_slot() {
            self.close_file_viewer(window, cx);
            if self.viewer_in_slot() {
                return true;
            }
        }
        if self.tasks.open {
            self.close_tasks(cx);
        }
        if self.screen != Screen::Conversation {
            self.show_screen(Screen::Conversation, window, cx);
        }
        self.review.v2.docked = true;
        self.review.v2.opened += 1;
        self.review.v2.menu = None;
        self.extras.notice.clear();
        if let Some(cwd) = self.selected_session().map(|s| s.cwd.clone()) {
            self.request(Request::Changes { cwd }, cx);
        }
        if let Some(focus) = &self.review.v2.focus {
            window.focus(focus);
        }
        cx.notify();
        true
    }

    pub(in crate::ui) fn close_review_dock(&mut self, cx: &mut Context<Self>) {
        self.review.v2.docked = false;
        self.review.v2.menu = None;
        self.review.v2.confirm = None;
        self.review.v2.drag = None;
        cx.notify();
    }

    /// Read every changed file's diff for the stacked view.
    pub(super) fn request_stack(&mut self, cx: &mut Context<Self>) {
        let Some((cwd, _)) = self.review_paths() else {
            return;
        };
        let files = self
            .review_changes()
            .iter()
            .take(DIFF_SET_MAX_FILES)
            .map(|c| {
                let staged = !c.has_unstaged();
                DiffTarget {
                    path: c.path.clone(),
                    staged,
                    untracked: c.untracked() && !staged,
                }
            })
            .collect();
        self.request(Request::DiffSet { cwd, files }, cx);
    }

    fn toggle_stack_file(&mut self, path: &str, cx: &mut Context<Self>) {
        let v2 = &mut self.review.v2;
        if !v2.collapsed.remove(path) {
            v2.collapsed.insert(path.to_owned());
        }
        v2.collapsed_rev += 1;
        cx.notify();
    }

    /// The parsed stack for `slot`, parsed once per answer and look; its
    /// syntax colours follow from a background task.
    fn stack_cache(&mut self, slot: &'static str, cx: &mut Context<Self>) -> Option<Cache> {
        let state = self.view.requests.get(slot)?;
        if state.loading || state.error.is_some() {
            return None;
        }
        if slot == COMMIT
            && !matches!(&state.request,
                Request::CommitDiff { hash, .. } if Some(hash) == self.review.commit.as_ref())
        {
            return None;
        }
        let look = (self.appearance, self.design());
        if let Some(cache) = &self.review.v2.cache
            && cache.slot == slot
            && cache.number == state.number
            && cache.look == look
        {
            return Some(cache.clone());
        }
        let (text, files) = if slot == SET {
            join_set(&state.value)
        } else {
            (
                state.value["diff"].as_str().unwrap_or_default().to_owned(),
                Vec::new(),
            )
        };
        let diff = Arc::new(diff::parse(&text));
        let mut sections = stack::sections(&diff);
        if sections.len() == files.len() {
            for (section, file) in sections.iter_mut().zip(&files) {
                section.path = file.path.clone();
            }
        }
        let cache = Cache {
            slot,
            number: state.number,
            look,
            emphasis: Arc::new(stack::emphasis(&diff)),
            diff: diff.clone(),
            sections: Arc::new(sections),
            highlights: Arc::default(),
            files: Arc::new(files),
        };
        self.review.v2.cache = Some(cache.clone());
        let theme = syntax::highlight_theme(look.0, look.1);
        let colored = cx
            .background_executor()
            .spawn(async move { syntax::highlight_diff(&diff, &theme) });
        let number = state.number;
        self.review.v2.coloring = Some(cx.spawn(async move |this, cx| {
            let highlights = Arc::new(colored.await);
            let _ = this.update(cx, |this, cx| {
                if let Some(cache) = &mut this.review.v2.cache
                    && (cache.slot, cache.number, cache.look) == (slot, number, look)
                {
                    cache.highlights = highlights;
                    cx.notify();
                }
            });
        }));
        Some(cache)
    }

    fn header_slots(&self) -> usize {
        let t = self.tokens().panels;
        (t.header_height / t.diff_row).ceil().max(1.) as usize
    }

    fn stack_layout(&mut self, cache: &Cache) -> Arc<Layout> {
        let key = LayoutKey {
            slot: cache.slot,
            number: cache.number,
            split: self.settings.review_split,
            collapsed: self.review.v2.collapsed_rev,
            slots: self.header_slots(),
        };
        if let Some((k, layout)) = &self.review.v2.layout
            && *k == key
        {
            return layout.clone();
        }
        let collapsed = self.review.v2.collapsed.clone();
        let layout = Arc::new(stack::layout(
            &cache.diff,
            &cache.sections,
            key.split,
            &|path| collapsed.contains(path),
            key.slots,
        ));
        self.review.v2.layout = Some((key, layout.clone()));
        layout
    }

    /// Scroll to the next (or previous) hunk below the sticky header.
    pub(in crate::ui) fn jump_hunk(&mut self, forward: bool, cx: &mut Context<Self>) {
        let Some((_, layout)) = &self.review.v2.layout else {
            return;
        };
        let slots = self.header_slots();
        // The slot at the list's top edge (every slot is one diff row tall).
        let offset = self.review.v2.scroll.0.borrow().base_handle.offset().y;
        let row = f32::from(px(self.tokens().panels.diff_row)).max(1.);
        let top = (-f32::from(offset) / row).round().max(0.) as usize + slots;
        let target = if forward {
            layout.hunks.iter().find(|&&h| h > top)
        } else {
            layout.hunks.iter().rev().find(|&&h| h < top)
        };
        if let Some(&h) = target {
            self.review.v2.scroll.scroll_to_item_strict_with_offset(
                h,
                gpui::ScrollStrategy::Top,
                slots,
            );
            cx.notify();
        }
    }

    fn stack_headers(&self, cache: &Cache, layout: &Layout) -> Vec<HeaderData> {
        let changes = self.review_changes();
        let cwd = self.review_paths().map(|(cwd, _)| cwd).unwrap_or_default();
        let root = self
            .review_paths()
            .map(|(_, root)| root)
            .unwrap_or_default();
        cache
            .sections
            .iter()
            .enumerate()
            .map(|(f, section)| {
                let (dir, name) = match section.path.rsplit_once(['/', '\\']) {
                    Some((dir, name)) => (dir.to_owned(), name.to_owned()),
                    None => (String::new(), section.path.clone()),
                };
                let change = (cache.slot == SET)
                    .then(|| changes.iter().find(|c| c.path == section.path))
                    .flatten();
                let mut badges = match change {
                    Some(change) => change_badges(change),
                    None => vec![section_badge(section)],
                };
                if cache.files.get(f).is_some_and(|file| file.error.is_some()) {
                    badges.push(("Unreadable", Intent::Danger));
                }
                let stage = change.map(|c| {
                    let stage = c.has_unstaged();
                    let path = Some(c.path.clone());
                    let cwd = cwd.clone();
                    (
                        stage,
                        if stage {
                            GitAction::Stage { cwd, path }
                        } else {
                            GitAction::Unstage { cwd, path }
                        },
                    )
                });
                let open = match change {
                    Some(c) if c.letter != 'D' => c.absolute.clone(),
                    Some(_) => None,
                    None if section.change != FileChange::Deleted => {
                        links::join_within(&root, &section.path)
                    }
                    None => None,
                };
                HeaderData {
                    path: section.path.clone(),
                    name,
                    dir,
                    renamed_from: section.old_path.clone(),
                    badges,
                    added: section.added,
                    removed: section.removed,
                    collapsed: self.review.v2.collapsed.contains(&section.path)
                        && layout.headers.get(f).is_some(),
                    stage,
                    open,
                }
            })
            .collect()
    }

    /// The stacked diff list, or the state that stands in for it.
    fn render_stack(&mut self, slot: &'static str, cx: &mut Context<Self>) -> AnyElement {
        let p = self.palette();
        let t = p.tokens;
        let state = self.view.requests.get(slot);
        let quiet = |icon, title: &'static str, body: Option<&'static str>| {
            div()
                .flex_1()
                .flex()
                .items_center()
                .justify_center()
                .child(primitives::empty_state(
                    icon,
                    title,
                    body.map(Into::into),
                    &p,
                ))
                .into_any_element()
        };
        if slot == COMMIT && self.review.commit.is_none() {
            return quiet(
                IconName::GalleryVerticalEnd,
                "Choose a commit",
                Some("Its changes show here, every file stacked."),
            );
        }
        if let Some(state) = state
            && state.error.is_some()
        {
            return div()
                .p(px(t.space.xl))
                .child(self.feature_message(slot))
                .into_any_element();
        }
        let Some(cache) = self.stack_cache(slot, cx) else {
            if slot == SET && self.review_status().is_some() && self.review_changes().is_empty() {
                return quiet(
                    IconName::CircleCheck,
                    "No uncommitted changes",
                    Some("The working tree matches the last commit."),
                );
            }
            return div()
                .debug_selector(|| "review-stack-loading".into())
                .flex_1()
                .flex()
                .items_center()
                .justify_center()
                .gap(px(t.space.md))
                .text_size(px(t.text.meta))
                .text_color(rgb(p.roles.text_tertiary))
                .child(primitives::status_indicator(
                    primitives::StatusKind::Working,
                    &p,
                ))
                .child("Reading changes…")
                .into_any_element();
        };
        if cache.sections.is_empty() {
            return quiet(IconName::CircleCheck, "No changes to show", None);
        }
        let layout = self.stack_layout(&cache);
        let files = Arc::new(self.stack_headers(&cache, &layout));
        let split = self.settings.review_split;
        let decoration = StickyHeaders {
            files,
            headers: Arc::new(layout.headers.clone()),
            ends: Arc::new(layout.ends.clone()),
            slots: self.header_slots(),
            busy: self.git_busy() || !self.view.connected,
            p,
            view: cx.entity().downgrade(),
        };
        let items = layout.clone();
        let rows = cache.clone();
        let truncated = cache.diff.truncated;
        let skipped = self
            .review_changes()
            .len()
            .saturating_sub(DIFF_SET_MAX_FILES);
        let list = uniform_list(
            "review-stack",
            items.items.len(),
            cx.processor(move |_this, range: Range<usize>, _, _| {
                range
                    .map(|ix| stack_item(&items.items[ix], ix, &rows, &p))
                    .collect::<Vec<_>>()
            }),
        )
        .with_decoration(decoration)
        .with_horizontal_sizing_behavior(if split {
            ListHorizontalSizingBehavior::FitList
        } else {
            ListHorizontalSizingBehavior::Unconstrained
        })
        .when(!split, |l| l.with_width_from_item(Some(layout.widest)))
        .track_scroll(self.review.v2.scroll.clone())
        .debug_selector(|| "review-stack".into())
        .flex_1()
        .min_h_0()
        .bg(rgb(p.surface))
        .text_size(px(t.text.meta));
        div()
            .flex_1()
            .min_h_0()
            .flex()
            .flex_col()
            .child(list)
            .when(truncated || skipped > 0, |d| {
                d.child(
                    div()
                        .px(px(t.space.lg))
                        .py(px(t.space.md))
                        .border_t(px(t.stroke.hairline))
                        .border_color(rgb(p.border))
                        .text_size(px(t.text.caption))
                        .text_color(rgb(p.roles.text_secondary))
                        .child(if skipped > 0 {
                            format!(
                                "{skipped} more changed files are not shown. Open them from the file list."
                            )
                        } else {
                            format!(
                                "Showing the first {} lines. Open a file to see all of it.",
                                diff::MAX_ROWS
                            )
                        }),
                )
            })
            .into_any_element()
    }

    /// "3 files · +12 −4" for the stack's toolbar.
    fn stack_summary(&self, slot: &'static str) -> Option<(usize, usize, usize)> {
        let cache = self.review.v2.cache.as_ref().filter(|c| c.slot == slot)?;
        Some((
            cache.sections.len(),
            cache.sections.iter().map(|s| s.added).sum(),
            cache.sections.iter().map(|s| s.removed).sum(),
        ))
    }

    /// Unified / Split, and the hunk jumps.
    fn stack_controls(&self, cx: &mut Context<Self>) -> Div {
        let p = self.palette();
        let t = p.tokens;
        let split = self.settings.review_split;
        let mode = |id: &'static str, label: &'static str, on: bool| {
            primitives::segment(id, label, on == split, &p)
                .debug_selector(move || id.into())
                .on_click(cx.listener(move |this, _, _, cx| {
                    if this.settings.review_split != on {
                        this.settings.review_split = on;
                        this.save_settings(cx);
                        cx.notify();
                    }
                }))
        };
        div()
            .flex()
            .items_center()
            .gap(px(t.space.xs))
            .flex_shrink_0()
            .child(
                primitives::segmented(&p)
                    .child(mode("review-unified", "Unified", false))
                    .child(mode("review-split", "Split", true)),
            )
            .child(
                primitives::icon_button_with_shortcut(
                    "review-prev-hunk",
                    IconName::ChevronUp,
                    "Previous change",
                    Some("[ c"),
                    ButtonVariant::Tertiary,
                    true,
                    &p,
                )
                .debug_selector(|| "review-prev-hunk".into())
                .on_click(cx.listener(|this, _, _, cx| this.jump_hunk(false, cx))),
            )
            .child(
                primitives::icon_button_with_shortcut(
                    "review-next-hunk",
                    IconName::ChevronDown,
                    "Next change",
                    Some("] c"),
                    ButtonVariant::Tertiary,
                    true,
                    &p,
                )
                .debug_selector(|| "review-next-hunk".into())
                .on_click(cx.listener(|this, _, _, cx| this.jump_hunk(true, cx))),
            )
    }

    /// `]c` / `[c` while the review has focus; Esc gives focus back.
    fn review_keys(&mut self, event: &KeyDownEvent, window: &mut Window, cx: &mut Context<Self>) {
        let Some(focus) = self.review.v2.focus.clone() else {
            return;
        };
        // Only the list's own focus: a commit message may contain "]c".
        if !focus.is_focused(window) {
            return;
        }
        let k = &event.keystroke;
        if k.modifiers.control || k.modifiers.alt || k.modifiers.platform {
            self.review.v2.bracket = None;
            return;
        }
        match (k.key.as_str(), self.review.v2.bracket.take()) {
            ("c", Some(forward)) => {
                self.jump_hunk(forward, cx);
                cx.stop_propagation();
            }
            ("]", _) => {
                self.review.v2.bracket = Some(true);
                cx.stop_propagation();
            }
            ("[", _) => {
                self.review.v2.bracket = Some(false);
                cx.stop_propagation();
            }
            ("escape", _) => {
                window.focus(&self.focus);
                cx.stop_propagation();
            }
            _ => {}
        }
    }

    fn review_focus(&mut self, cx: &mut Context<Self>) -> FocusHandle {
        self.review
            .v2
            .focus
            .get_or_insert_with(|| cx.focus_handle())
            .clone()
    }

    /// The commit box: message, then one primary (Commit) with stage-all
    /// and push/pull beside it as quieter actions.
    fn commit_box(&mut self, changes: &[Change], root: &str, cx: &mut Context<Self>) -> Div {
        let p = self.palette();
        let t = p.tokens;
        let busy = self.git_busy() || !self.view.connected;
        let staged = changes.iter().filter(|c| c.has_staged()).count();
        let unstaged = changes.iter().filter(|c| c.has_unstaged()).count();
        let status = self.review_status();
        let (ahead, behind) = status.map_or((0, 0), |s| (s.ahead, s.behind));
        let unpublished = status.is_some_and(|s| s.upstream.is_none() && s.branch.is_some());
        let cwd = self.review_paths().map(|(cwd, _)| cwd).unwrap_or_default();
        let committing = self.view.requests.get("git-action").is_some_and(|s| {
            s.loading && matches!(s.request, Request::Git(GitAction::Commit { .. }))
        });
        let can_commit = !busy && staged > 0;
        let summary = match (staged, unstaged) {
            (0, 0) => "Nothing to commit".to_owned(),
            (0, n) => format!("{n} unstaged · stage files to commit them"),
            (s, 0) => format!("{s} staged"),
            (s, n) => format!("{s} staged · {n} unstaged"),
        };
        let bulk = if unstaged > 0 {
            let (cwd, root) = (cwd.clone(), root.to_owned());
            Some(
                primitives::button(
                    "review-stage-all",
                    "Stage all",
                    None,
                    ButtonVariant::Tertiary,
                    ButtonSize::Small,
                    !busy,
                    &p,
                )
                .debug_selector(|| "review-stage-all".into())
                .when(!busy, |d| {
                    d.on_click(cx.listener(move |this, _, _, cx| {
                        this.git_action(
                            GitAction::Stage {
                                cwd: cwd.clone(),
                                path: Some(root.clone()),
                            },
                            cx,
                        )
                    }))
                }),
            )
        } else if staged > 0 {
            let (cwd, root) = (cwd.clone(), root.to_owned());
            Some(
                primitives::button(
                    "review-unstage-all",
                    "Unstage all",
                    None,
                    ButtonVariant::Tertiary,
                    ButtonSize::Small,
                    !busy,
                    &p,
                )
                .debug_selector(|| "review-unstage-all".into())
                .when(!busy, |d| {
                    d.on_click(cx.listener(move |this, _, _, cx| {
                        this.git_action(
                            GitAction::Unstage {
                                cwd: cwd.clone(),
                                path: Some(root.clone()),
                            },
                            cx,
                        )
                    }))
                }),
            )
        } else {
            None
        };
        let sync = |id: &'static str, label: String, action: GitAction| {
            primitives::button(
                id,
                label,
                None,
                ButtonVariant::Secondary,
                ButtonSize::Small,
                !busy,
                &p,
            )
            .debug_selector(move || id.into())
            .when(!busy, |d| {
                d.on_click(cx.listener(move |this, _, _, cx| this.git_action(action.clone(), cx)))
            })
        };
        let pull = (behind > 0).then(|| {
            sync(
                "review-pull",
                format!("Pull ↓{behind}"),
                GitAction::Pull { cwd: cwd.clone() },
            )
        });
        let push = (ahead > 0 || unpublished).then(|| {
            sync(
                "review-push",
                if unpublished {
                    "Publish branch".to_owned()
                } else {
                    format!("Push ↑{ahead}")
                },
                GitAction::Push { cwd: cwd.clone() },
            )
        });
        let commit = primitives::button(
            "review-commit",
            if committing {
                "Committing…".to_owned()
            } else if staged > 0 {
                format!("Commit {staged}")
            } else {
                "Commit".to_owned()
            },
            Some(IconName::Check),
            ButtonVariant::Primary,
            ButtonSize::Small,
            can_commit,
            &p,
        )
        .debug_selector(|| "review-commit".into())
        .when(can_commit, |d| {
            d.on_click(cx.listener(|this, _, window, cx| this.commit_staged(window, cx)))
        });
        div()
            .debug_selector(|| "review-commit-box".into())
            .flex_shrink_0()
            .p(px(t.space.lg))
            .flex()
            .flex_col()
            .gap(px(t.space.md))
            .border_t(px(t.stroke.hairline))
            .border_color(rgb(p.border))
            .bg(rgb(p.surface))
            .child(
                div()
                    .debug_selector(|| "review-commit-message".into())
                    .on_key_down(cx.listener(|this, event: &KeyDownEvent, window, cx| {
                        let k = &event.keystroke;
                        if k.key == "enter" && (k.modifiers.control || k.modifiers.platform) {
                            cx.stop_propagation();
                            this.commit_staged(window, cx);
                        }
                    }))
                    .child(Input::new(&self.extras.commit_message).small()),
            )
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap(px(t.space.sm))
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .truncate()
                            .text_size(px(t.text.caption))
                            .text_color(rgb(p.roles.text_tertiary))
                            .child(summary),
                    )
                    .children(bulk)
                    .children(pull)
                    .children(push)
                    .child(commit),
            )
            // Pull only fast-forwards; a branch that moved both ways needs a
            // rebase or merge, which Changes does not attempt.
            .when(behind > 0 && ahead > 0, |d| {
                d.child(
                    div()
                        .text_size(px(t.text.caption))
                        .text_color(rgb(p.warning))
                        .child("This branch and its upstream have both moved; rebase or merge in a terminal."),
                )
            })
    }

    /// The working tree's stacked diff in a panel: toolbar, list, commit.
    fn review_dock_panel(&mut self, window: &mut Window, cx: &mut Context<Self>) -> Stateful<Div> {
        let p = self.palette();
        let t = p.tokens;
        let _ = window;
        // The selection moved to a session in another folder: read it.
        if let Some(cwd) = self.selected_session().map(|s| s.cwd.clone())
            && self.review_paths().is_some_and(|(c, _)| c != cwd)
            && !self.view.requests.get("changes").is_some_and(|s| s.loading)
        {
            self.request(Request::Changes { cwd }, cx);
        }
        let (files, added, removed) = self.stack_summary(SET).unwrap_or_default();
        let branch = self.review_status().and_then(|s| s.branch.clone());
        let meta = {
            let mut parts = Vec::new();
            if files > 0 {
                parts.push(format!("{files} file{}", if files == 1 { "" } else { "s" }));
            }
            if let Some(branch) = branch {
                parts.push(branch);
            }
            parts.join(" · ")
        };
        let header = div()
            .debug_selector(|| "review-dock-header".into())
            .flex_shrink_0()
            .h(px(t.panels.header_height))
            .pl(px(t.space.lg))
            .pr(px(t.space.sm))
            .flex()
            .items_center()
            .gap(px(t.space.md))
            .border_b(px(t.stroke.hairline))
            .border_color(rgb(p.border))
            .child(
                div()
                    .flex_shrink_0()
                    .text_size(px(t.text.subheading))
                    .font_weight(FontWeight(t.weight.semibold))
                    .text_color(rgb(p.text))
                    .child("Changes"),
            )
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .flex()
                    .items_center()
                    .gap(px(t.space.sm))
                    .text_size(px(t.text.caption))
                    .child(
                        primitives::truncated("review-dock-meta", meta, &p)
                            .text_color(rgb(p.roles.text_tertiary)),
                    )
                    .when(added + removed > 0, |d| d.child(counts(added, removed, &p))),
            )
            .child(self.stack_controls(cx))
            .child(
                primitives::icon_button(
                    "review-dock-refresh",
                    IconName::Redo,
                    "Refresh",
                    ButtonVariant::Tertiary,
                    self.view.connected,
                    &p,
                )
                .debug_selector(|| "review-dock-refresh".into())
                .on_click(cx.listener(|this, _, _, cx| {
                    if let Some((cwd, _)) = this.review_paths() {
                        this.request(Request::Changes { cwd }, cx);
                    }
                })),
            )
            .child(
                primitives::icon_button(
                    "review-dock-expand",
                    IconName::Maximize,
                    "Open full page (files, tree and commits)",
                    ButtonVariant::Tertiary,
                    true,
                    &p,
                )
                .debug_selector(|| "review-dock-expand".into())
                .on_click(cx.listener(|this, _, window, cx| {
                    // The dock stays open underneath: Back to chat finds it.
                    this.review.v2.page_once = true;
                    this.review.mode = Mode::Changed;
                    this.open_feature(Screen::Changes, window, cx);
                })),
            )
            .child(
                primitives::icon_button(
                    "review-dock-close",
                    IconName::Close,
                    "Close",
                    ButtonVariant::Tertiary,
                    true,
                    &p,
                )
                .debug_selector(|| "review-dock-close".into())
                .on_click(cx.listener(|this, _, _, cx| this.close_review_dock(cx))),
            );
        let notice = (!self.extras.notice.is_empty()).then(|| {
            let tone = chrome::notice_tone(&self.extras.notice);
            div()
                .px(px(t.space.lg))
                .pt(px(t.space.md))
                .child(chrome::notice_line(self.extras.notice.clone(), tone, p))
                .debug_selector(|| "feature-notice".into())
        });
        let changes = self.review_changes();
        let root = self.review_paths().map(|(_, r)| r).unwrap_or_default();
        let list = self.render_stack(SET, cx);
        let commit = (!changes.is_empty()).then(|| self.commit_box(&changes, &root, cx));
        let focus = self.review_focus(cx);
        div()
            .id("review-dock-panel")
            .debug_selector(|| "review-dock-panel".into())
            .track_focus(&focus)
            .on_key_down(cx.listener(|this, event: &KeyDownEvent, window, cx| {
                this.review_keys(event, window, cx)
            }))
            .on_mouse_down(
                gpui::MouseButton::Left,
                cx.listener(move |_, _, window, _| window.focus(&focus)),
            )
            .size_full()
            .flex()
            .flex_col()
            .rounded(px(t.radius.panel))
            .bg(rgb(p.surface))
            .border(px(t.stroke.hairline))
            .border_color(rgb(p.roles.card_border))
            .overflow_hidden()
            .child(header)
            .children(notice)
            .child(self.feature_message("changes"))
            .child(self.feature_message("git-action"))
            .child(list)
            .children(commit)
            .children(self.review_overlays(window, cx))
    }

    /// The menu and discard dialog over everything.
    fn review_overlays(&mut self, window: &Window, cx: &mut Context<Self>) -> Vec<AnyElement> {
        let p = self.palette();
        let t = p.tokens;
        let viewport = window.viewport_size();
        let mut out = Vec::new();
        let close = cx.listener(|this, _: &gpui::MouseDownEvent, _, cx| {
            this.review.v2.menu = None;
            cx.notify();
        });
        if let Some(menu) = self.review.v2.menu.clone() {
            let section = self
                .review
                .v2
                .cache
                .as_ref()
                .and_then(|c| c.sections.get(menu.file).cloned().map(|s| (c.slot, s)));
            if let Some((slot, section)) = section {
                let change = (slot == SET)
                    .then(|| {
                        self.review_changes()
                            .iter()
                            .find(|c| c.path == section.path)
                            .cloned()
                    })
                    .flatten();
                let root = self.review_paths().map(|(_, r)| r).unwrap_or_default();
                let absolute = links::join_within(&root, &section.path);
                let discardable = change.as_ref().is_some_and(Change::has_unstaged)
                    && !self.git_busy()
                    && self.view.connected;
                let target = change.as_ref().map(|c| DiscardTarget {
                    cwd: self.review_paths().map(|(cwd, _)| cwd).unwrap_or_default(),
                    path: c.path.clone(),
                    untracked: c.untracked(),
                });
                let copy_path = absolute.clone().unwrap_or(section.path.clone());
                let open = absolute.filter(|_| section.change != FileChange::Deleted);
                let items = primitives::menu(&p)
                    .debug_selector(|| "review-menu".into())
                    .occlude()
                    .on_mouse_down(gpui::MouseButton::Left, |_, _, cx| cx.stop_propagation())
                    .child(
                        primitives::menu_item(
                            "review-menu-open",
                            "Open in editor",
                            Some(IconName::File),
                            false,
                            open.is_some(),
                            &p,
                        )
                        .when_some(open, |d, open| {
                            d.on_click(cx.listener(move |this, _, _, cx| {
                                this.review.v2.menu = None;
                                this.review_open_file(&open, cx);
                            }))
                        }),
                    )
                    .child(
                        primitives::menu_item(
                            "review-menu-copy",
                            "Copy path",
                            Some(IconName::Copy),
                            false,
                            true,
                            &p,
                        )
                        .on_click(cx.listener(move |this, _, _, cx| {
                            cx.write_to_clipboard(gpui::ClipboardItem::new_string(
                                copy_path.clone(),
                            ));
                            this.review.v2.menu = None;
                            cx.notify();
                        })),
                    )
                    .when(slot == SET, |d| {
                        let untracked = target.as_ref().is_some_and(|t| t.untracked);
                        d.child(primitives::menu_separator(&p)).child(
                            primitives::menu_item(
                                "review-discard",
                                if untracked {
                                    "Delete file…"
                                } else {
                                    "Discard changes…"
                                },
                                Some(IconName::Undo),
                                true,
                                discardable,
                                &p,
                            )
                            .debug_selector(|| "review-discard".into())
                            .when_some(
                                target.filter(|_| discardable),
                                |d, target| {
                                    d.on_click(cx.listener(move |this, _, _, cx| {
                                        this.review.v2.menu = None;
                                        this.review.v2.confirm = Some(target.clone());
                                        cx.notify();
                                    }))
                                },
                            ),
                        )
                    });
                let width = px(t.layout.menu_width);
                let left = (menu.position.x - width).max(px(t.space.md));
                out.push(
                    deferred(
                        anchored()
                            .position(point(Pixels::ZERO, Pixels::ZERO))
                            .child(
                                div()
                                    .id("review-menu-backdrop")
                                    .w(viewport.width)
                                    .h(viewport.height)
                                    .occlude()
                                    .on_mouse_down(gpui::MouseButton::Left, close)
                                    .child(
                                        div()
                                            .absolute()
                                            .left(left)
                                            .top(menu.position.y + px(t.space.sm))
                                            .child(items),
                                    ),
                            ),
                    )
                    .with_priority(3)
                    .into_any_element(),
                );
            }
        }
        if let Some(target) = self.review.v2.confirm.clone() {
            let name = target
                .path
                .rsplit_once(['/', '\\'])
                .map_or(target.path.clone(), |(_, n)| n.to_owned());
            let busy = self.git_busy() || !self.view.connected;
            let dialog = primitives::dialog(&p)
                .debug_selector(|| "review-discard-dialog".into())
                .occlude()
                .on_mouse_down(gpui::MouseButton::Left, |_, _, cx| cx.stop_propagation())
                .child(primitives::dialog_header(
                    if target.untracked {
                        format!("Delete {name}?")
                    } else {
                        format!("Discard changes to {name}?")
                    },
                    Some(
                        if target.untracked {
                            "The file is not tracked by git, so it cannot be restored."
                        } else {
                            "Its unstaged edits are lost. Staged changes stay."
                        }
                        .into(),
                    ),
                    &p,
                ))
                .child(
                    primitives::dialog_footer(&p)
                        .child(
                            primitives::button(
                                "review-discard-cancel",
                                "Cancel",
                                None,
                                ButtonVariant::Secondary,
                                ButtonSize::Small,
                                true,
                                &p,
                            )
                            .debug_selector(|| "review-discard-cancel".into())
                            .on_click(cx.listener(|this, _, _, cx| {
                                this.review.v2.confirm = None;
                                cx.notify();
                            })),
                        )
                        .child(
                            primitives::button(
                                "review-discard-confirm",
                                if target.untracked {
                                    "Delete file"
                                } else {
                                    "Discard changes"
                                },
                                None,
                                ButtonVariant::Destructive,
                                ButtonSize::Small,
                                !busy,
                                &p,
                            )
                            .debug_selector(|| "review-discard-confirm".into())
                            .when(!busy, |d| {
                                d.on_click(cx.listener(move |this, _, _, cx| {
                                    this.review.v2.confirm = None;
                                    this.git_action(
                                        GitAction::Discard {
                                            cwd: target.cwd.clone(),
                                            path: target.path.clone(),
                                        },
                                        cx,
                                    )
                                }))
                            }),
                        ),
                );
            out.push(
                deferred(
                    anchored()
                        .position(point(Pixels::ZERO, Pixels::ZERO))
                        .child(
                            div()
                                .id("review-discard-scrim")
                                .relative()
                                .w(viewport.width)
                                .h(viewport.height)
                                .occlude()
                                .on_mouse_down(
                                    gpui::MouseButton::Left,
                                    cx.listener(|this, _, _, cx| {
                                        this.review.v2.confirm = None;
                                        cx.notify();
                                    }),
                                )
                                .child(primitives::scrim(&p).child(dialog)),
                        ),
                )
                .with_priority(3)
                .into_any_element(),
            );
        }
        out
    }

    /// The dock beside (or below) the conversation.
    pub(in crate::ui) fn render_docked_review(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Option<AnyElement> {
        if !self.review_dock_showing() {
            return None;
        }
        let t = self.tokens();
        if self.portrait(window) {
            let Some(portrait::Dock::Below { height, .. }) = self.dock(window) else {
                return None;
            };
            let panel = self.review_dock_panel(window, cx);
            return Some(
                self.dock_below_slot("review-dock", height, panel, cx)
                    .into_any_element(),
            );
        }
        let width = self.review_dock_width(window)?;
        let p = self.palette();
        let panel = self.review_dock_panel(window, cx);
        let handle = div()
            .id("review-dock-resize")
            .debug_selector(|| "review-dock-resize".into())
            .absolute()
            .top_0()
            .left_0()
            .h_full()
            .w(px(t.panels.dock_handle))
            .cursor(gpui::CursorStyle::ResizeLeftRight)
            .hover(move |s| s.bg(rgb(p.border)))
            .on_mouse_down(
                gpui::MouseButton::Left,
                cx.listener(move |this, event: &gpui::MouseDownEvent, _, cx| {
                    if event.click_count == 2 {
                        this.settings.review_dock_width = None;
                        this.review.v2.drag = None;
                        this.save_settings(cx);
                    } else {
                        this.review.v2.drag = Some((event.position.x, width));
                    }
                    cx.stop_propagation();
                    cx.notify();
                }),
            );
        let slot = div()
            .id("review-dock")
            .debug_selector(|| "review-dock".into())
            .relative()
            .h_full()
            .w(px(width))
            .flex_shrink_0()
            .child(
                div()
                    .absolute()
                    .top_0()
                    .left_0()
                    .size_full()
                    .py(px(t.space.md))
                    .pr(px(t.space.md))
                    .pl(px(t.panels.dock_handle))
                    .when(chrome::custom_caption(), |d| {
                        d.pt(px(chrome::CAPTION_HEIGHT + t.space.xs))
                    })
                    .child(panel),
            )
            .child(handle)
            .child(self.review_drag_watch(cx));
        if self.settings.reduce_motion {
            return Some(slot.into_any_element());
        }
        let duration = std::time::Duration::from_millis(t.panels.slide_ms as u64);
        Some(
            slot.with_animation(
                ("review-dock-slide", self.review.v2.opened),
                gpui::Animation::new(duration).with_easing(|t| 1. - (1. - t).powi(3)),
                move |slot, progress| slot.w(px(width * progress)),
            )
            .into_any_element(),
        )
    }

    /// While the dock's edge is dragged, follow the pointer anywhere in the
    /// window and save the width on release.
    fn review_drag_watch(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let entity = cx.entity().downgrade();
        let dragging = self.review.v2.drag.is_some();
        canvas(
            |_, _, _| (),
            move |_, _, window, _| {
                if !dragging {
                    return;
                }
                let moving = entity.clone();
                window.on_mouse_event(move |event: &gpui::MouseMoveEvent, phase, window, cx| {
                    if phase.capture() {
                        let _ = moving.update(cx, |this, cx| {
                            if let Some((start, width)) = this.review.v2.drag {
                                // Dragging left grows the dock.
                                let moved = unzoom(start - event.position.x);
                                this.settings.review_dock_width = Some(width + moved);
                                if let Some(fit) = this.review_dock_width(window) {
                                    this.settings.review_dock_width = Some(fit);
                                }
                                cx.stop_propagation();
                                cx.notify();
                            }
                        });
                    }
                });
                let releasing = entity.clone();
                window.on_mouse_event(move |_: &gpui::MouseUpEvent, phase, _, cx| {
                    if phase.capture() {
                        let _ = releasing.update(cx, |this, cx| {
                            if this.review.v2.drag.take().is_some() {
                                this.save_settings(cx);
                                cx.stop_propagation();
                            }
                        });
                    }
                });
            },
        )
        .absolute()
        .size(px(0.))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_set_answer_joins_one_section_per_file() {
        let value = serde_json::json!({"files": [
            {"path": "src/a.rs", "diff": "diff --git a/src/a.rs b/src/a.rs\n--- a/src/a.rs\n+++ b/src/a.rs\n@@ -1 +1 @@\n-a\n+b"},
            {"path": "img.png", "diff": "", "error": null},
            {"path": "gone.rs", "diff": "", "error": "unreadable"},
        ]});
        let (text, files) = join_set(&value);
        let diff = diff::parse(&text);
        let sections = stack::sections(&diff);
        assert_eq!(sections.len(), 3);
        assert_eq!(sections[0].path, "src/a.rs");
        assert_eq!(sections[1].path, "img.png");
        assert_eq!(files[2].error.as_deref(), Some("unreadable"));
    }

    #[test]
    fn emphasis_splits_syntax_ranges() {
        let red = HighlightStyle {
            color: Some(gpui::red()),
            ..Default::default()
        };
        let bg = gpui::blue();
        let merged = with_emphasis(10, &[(0..6, red)], Some(4..8), bg);
        assert_eq!(merged.len(), 3);
        assert_eq!(merged[0], (0..4, red));
        assert_eq!(merged[1].0, 4..6);
        assert_eq!(merged[1].1.background_color, Some(bg));
        assert_eq!(merged[1].1.color, red.color);
        assert_eq!(merged[2].0, 6..8);
        assert_eq!(merged[2].1.color, None);
        // No emphasis: the syntax ranges as they were.
        assert_eq!(with_emphasis(10, &[(0..6, red)], None, bg), [(0..6, red)]);
    }
}
