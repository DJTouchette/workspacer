//! The file explorer shared by the editor and the review view. Directories
//! are listed on the session's machine by the connected hub (`fs.listEntries`),
//! one at a time, and cached per window workspace so both trees and a
//! popped-out editor read one copy. Every file is listed, tracked or not,
//! except `.git`; git-ignored entries (build output, dependencies) are hidden
//! until "Show ignored" is on, and a hub too old to include them says so.
use super::*;
use gpui::AnyElement;
use std::collections::{BTreeSet, VecDeque};
use wks_native::features::Request;

/// Rows a tree draws before it says the rest are hidden.
const MAX_ROWS: usize = 4_000;

#[derive(Clone, Debug, Default)]
pub(super) struct Dir {
    pub loading: bool,
    pub error: Option<String>,
    pub entries: Vec<Entry>,
    /// The hub left git-ignored entries out of this listing.
    pub hides_ignored: bool,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct Entry {
    pub name: String,
    pub path: String,
    pub dir: bool,
}

#[derive(Default)]
pub(super) struct Explorer {
    dirs: HashMap<String, Dir>,
    queue: VecDeque<String>,
    inflight: Option<String>,
    seen: u64,
    /// List git-ignored entries too (an explicit choice; off by default).
    pub show_ignored: bool,
}

/// One line of a flattened tree.
#[derive(Clone, Debug)]
pub(super) struct TreeRow {
    pub depth: usize,
    pub entry: Entry,
    pub expanded: bool,
    pub loading: bool,
    pub error: Option<String>,
}

pub(super) struct Tree {
    pub rows: Vec<TreeRow>,
    /// The root's own state, for its loading/error line.
    pub root: Dir,
    pub truncated: bool,
}

impl Explorer {
    /// `root`'s tree with the `expanded` directories open.
    pub fn tree(&self, root: &str, expanded: &BTreeSet<String>) -> Tree {
        let mut rows = Vec::new();
        let mut truncated = false;
        self.walk(root, 0, expanded, &mut rows, &mut truncated);
        Tree {
            rows,
            root: self.dirs.get(root).cloned().unwrap_or(Dir {
                loading: true,
                ..Default::default()
            }),
            truncated,
        }
    }

    fn walk(
        &self,
        path: &str,
        depth: usize,
        expanded: &BTreeSet<String>,
        rows: &mut Vec<TreeRow>,
        truncated: &mut bool,
    ) {
        // A symlink loop cannot recurse forever: depth is bounded too.
        let Some(dir) = self.dirs.get(path).filter(|_| depth < 64) else {
            return;
        };
        for entry in &dir.entries {
            if rows.len() >= MAX_ROWS {
                *truncated = true;
                return;
            }
            let open = entry.dir && expanded.contains(&entry.path);
            let child = self.dirs.get(&entry.path);
            rows.push(TreeRow {
                depth,
                entry: entry.clone(),
                expanded: open,
                loading: open && child.is_none_or(|d| d.loading),
                error: child.filter(|_| open).and_then(|d| d.error.clone()),
            });
            if open {
                self.walk(&entry.path, depth + 1, expanded, rows, truncated);
            }
        }
    }
}

impl Workspace {
    /// Make sure `path` is listed (or being listed). Failed listings retry.
    pub(super) fn explore(&mut self, path: &str, cx: &mut Context<Self>) {
        if path.is_empty()
            || self
                .explorer
                .dirs
                .get(path)
                .is_some_and(|d| d.error.is_none())
        {
            return;
        }
        self.explorer.dirs.insert(
            path.to_owned(),
            Dir {
                loading: true,
                ..Default::default()
            },
        );
        if !self.explorer.queue.iter().any(|p| p == path) {
            self.explorer.queue.push_back(path.to_owned());
        }
        self.pump_explorer(cx);
    }

    /// Forget every listing under `root` and list `root` again.
    pub(super) fn refresh_explorer(&mut self, root: &str, cx: &mut Context<Self>) {
        let root = root.trim_end_matches(['/', '\\']).to_owned();
        self.explorer
            .dirs
            .retain(|path, _| !wks_native::links::within(&root, path));
        self.explorer
            .queue
            .retain(|path| !wks_native::links::within(&root, path));
        self.explore(&root, cx);
    }

    fn pump_explorer(&mut self, cx: &mut Context<Self>) {
        if self.explorer.inflight.is_some() || !self.view.connected {
            return;
        }
        if let Some(path) = self.explorer.queue.pop_front() {
            self.explorer.inflight = Some(path.clone());
            let include_ignored = self.explorer.show_ignored;
            self.command(
                Command::Request(Request::ListDir {
                    path,
                    include_ignored,
                }),
                cx,
            );
        }
    }

    /// Adopt a finished listing and start the next queued one.
    pub(super) fn sync_explorer(&mut self, next: &View, cx: &mut Context<Self>) {
        if let Some(state) = next.requests.get("file-tree")
            && !state.loading
            && state.number > self.explorer.seen
        {
            self.explorer.seen = state.number;
            if let Request::ListDir {
                path,
                include_ignored,
            } = &state.request
            {
                // A listing made under the other ignore setting is stale.
                let current = *include_ignored == self.explorer.show_ignored;
                if self.explorer.inflight.as_ref() == Some(path) {
                    self.explorer.inflight = None;
                }
                let dir = match &state.error {
                    Some(error) => Dir {
                        loading: false,
                        error: Some(error.clone()),
                        entries: vec![],
                        hides_ignored: !include_ignored,
                    },
                    None => Dir {
                        loading: false,
                        error: None,
                        hides_ignored: state.value["includeIgnored"] != true,
                        entries: state.value["entries"]
                            .as_array()
                            .into_iter()
                            .flatten()
                            .filter_map(|e| {
                                Some(Entry {
                                    name: e["name"].as_str()?.to_owned(),
                                    path: e["path"].as_str()?.to_owned(),
                                    dir: e["isDir"].as_bool().unwrap_or(false),
                                })
                            })
                            .collect(),
                    },
                };
                if current {
                    self.explorer.dirs.insert(path.clone(), dir);
                }
                self.refresh_popout(cx);
            }
        }
        // A listing lost to a disconnect is retried after reconnecting.
        if !next.connected
            && let Some(path) = self.explorer.inflight.take()
        {
            self.explorer.queue.push_front(path);
        }
        if next.connected && !self.view.connected {
            self.explorer.inflight = None;
        }
    }

    /// Show or hide git-ignored entries: every listing is read again.
    pub(super) fn set_show_ignored(&mut self, show: bool, cx: &mut Context<Self>) {
        if self.explorer.show_ignored == show {
            return;
        }
        self.explorer.show_ignored = show;
        let listed: Vec<_> = self.explorer.dirs.keys().cloned().collect();
        self.explorer.dirs.clear();
        self.explorer.queue.clear();
        for path in listed {
            self.explore(&path, cx);
        }
        cx.notify();
    }

    /// Continue queued listings once the view (and connection) is current.
    pub(super) fn resume_explorer(&mut self, cx: &mut Context<Self>) {
        self.pump_explorer(cx);
    }
}

/// How a tree marks a path (review status letters), keyed by absolute path.
pub(super) type Marks = HashMap<String, (char, u32)>;

pub(super) type OnRow = std::rc::Rc<dyn Fn(&TreeRow, &mut Window, &mut App)>;

/// Draw `tree`: chevrons for folders, file icons, an optional status mark.
pub(super) fn render_tree(
    id: &'static str,
    tree: &Tree,
    selected: Option<&str>,
    marks: &Marks,
    p: Palette,
    on_row: OnRow,
) -> AnyElement {
    let status = |text: String, tone: u32| {
        div()
            .px_3()
            .py_2()
            .text_size(px(chrome::scale::CAPTION))
            .text_color(rgb(tone))
            .child(text)
    };
    let mut body = div().flex().flex_col().py_1();
    if tree.root.hides_ignored && !tree.root.loading && tree.root.error.is_none() {
        body = body.child(status("Git-ignored files are hidden.".into(), p.disabled));
    }
    if let Some(error) = &tree.root.error {
        body = body.child(status(
            format!("Couldn’t list this folder: {error}"),
            p.error,
        ));
    } else if tree.root.loading {
        body = body.child(status("Loading files…".into(), p.muted));
    } else if tree.rows.is_empty() {
        body = body.child(status("This folder is empty.".into(), p.muted));
    }
    for (ix, row) in tree.rows.iter().enumerate() {
        let active = selected == Some(row.entry.path.as_str());
        let mark = marks.get(&row.entry.path).copied();
        let on_row = on_row.clone();
        let clicked = row.clone();
        let indent = 8. + row.depth as f32 * 14.;
        body = body.child(
            div()
                .id((id, ix))
                .debug_selector({
                    let name = format!("{id}-row-{}", row.entry.name);
                    move || name
                })
                .h(px(24.))
                .pl(px(indent))
                .pr_2()
                .flex()
                .items_center()
                .gap_1()
                .cursor_pointer()
                .text_size(px(chrome::scale::META))
                .when(active, |d| d.bg(rgb(p.selected)))
                .hover(|s| s.bg(rgb(p.selected)))
                .on_click(move |_, window, cx| on_row(&clicked, window, cx))
                .child(div().w(px(12.)).flex_shrink_0().when(row.entry.dir, |d| {
                    d.child(
                        Icon::new(if row.expanded {
                            IconName::ChevronDown
                        } else {
                            IconName::ChevronRight
                        })
                        .size(px(12.))
                        .text_color(rgb(p.muted)),
                    )
                }))
                .child(
                    Icon::new(if row.entry.dir {
                        if row.expanded {
                            IconName::FolderOpen
                        } else {
                            IconName::Folder
                        }
                    } else {
                        IconName::File
                    })
                    .size(px(13.))
                    .flex_shrink_0()
                    .text_color(rgb(if active { p.accent } else { p.muted })),
                )
                .child(
                    div()
                        .flex_1()
                        .min_w_0()
                        .truncate()
                        .text_color(rgb(match mark {
                            Some((_, color)) => color,
                            None if active => p.text,
                            None => p.prose,
                        }))
                        .child(row.entry.name.clone()),
                )
                .when(row.loading, |d| {
                    d.child(div().text_size(px(10.)).text_color(rgb(p.muted)).child("…"))
                })
                .when_some(mark, |d, (letter, color)| {
                    d.child(
                        div()
                            .flex_shrink_0()
                            .font_family(mono_font())
                            .text_size(px(11.))
                            .text_color(rgb(color))
                            .child(letter.to_string()),
                    )
                }),
        );
        if let Some(error) = &row.error {
            body = body.child(
                div()
                    .pl(px(indent + 26.))
                    .pr_2()
                    .text_size(px(chrome::scale::CAPTION))
                    .text_color(rgb(p.error))
                    .child(error.clone()),
            );
        }
    }
    if tree.truncated {
        body = body.child(status(
            format!("Showing the first {MAX_ROWS} entries. Collapse folders to see more."),
            p.muted,
        ));
    }
    body.into_any_element()
}

/// Git's colour for a status letter.
pub(super) fn status_color(letter: char, p: Palette) -> u32 {
    match letter {
        'A' | '?' => p.success,
        'D' => p.error,
        'U' => p.warning,
        'R' | 'C' => p.accent,
        _ => p.warning,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entry(name: &str, path: &str, dir: bool) -> Entry {
        Entry {
            name: name.into(),
            path: path.into(),
            dir,
        }
    }

    #[test]
    fn trees_flatten_open_folders_in_order() {
        let mut explorer = Explorer::default();
        explorer.dirs.insert(
            "/r".into(),
            Dir {
                entries: vec![
                    entry("src", "/r/src", true),
                    entry("a.txt", "/r/a.txt", false),
                ],
                ..Default::default()
            },
        );
        explorer.dirs.insert(
            "/r/src".into(),
            Dir {
                entries: vec![entry("lib.rs", "/r/src/lib.rs", false)],
                ..Default::default()
            },
        );
        let closed = explorer.tree("/r", &BTreeSet::new());
        assert_eq!(closed.rows.len(), 2);
        let open = explorer.tree("/r", &BTreeSet::from(["/r/src".to_owned()]));
        let names: Vec<_> = open
            .rows
            .iter()
            .map(|r| (r.depth, r.entry.name.as_str()))
            .collect();
        assert_eq!(names, [(0, "src"), (1, "lib.rs"), (0, "a.txt")]);
        // An expanded folder not yet listed shows as loading.
        let pending = explorer.tree("/r", &BTreeSet::from(["/r/other".to_owned()]));
        assert!(!pending.rows[0].loading);
        let unlisted = Explorer::default().tree("/x", &BTreeSet::new());
        assert!(unlisted.root.loading);
    }
}
