use super::*;
use std::collections::VecDeque;
use wks_native::model::Row;

impl Workspace {
    pub(super) fn resume_at_visible_tail(
        &mut self,
        session: &str,
        row: &Arc<Row>,
        bounds: gpui::Bounds<gpui::Pixels>,
        cx: &mut Context<Self>,
    ) {
        // The wheel callback runs before virtual-list layout can clamp to its
        // newly measured end. Keyboard scrolls don't call it at all. Use the
        // painted last row, and reject deferred observations from stale views.
        if self.follow
            || self.view.loading
            || self.view.selected.as_deref() != Some(session)
            || !self
                .view
                .transcript
                .rows
                .back()
                .is_some_and(|last| Arc::ptr_eq(last, row))
        {
            return;
        }
        let viewport = self.list.viewport_bounds();
        let reading_bottom = viewport.bottom() - self.composer_dock_bounds.size.height;
        if bounds.bottom() > viewport.top() && bounds.bottom() <= reading_bottom + px(1.) {
            // No scroll_to here: hide the button without snapping the reading
            // position. The next incoming update will follow the tail normally.
            self.follow = true;
            cx.notify();
        }
    }

    /// GPUI exposes bottom-follow as item_count, not the actual visible top.
    /// Normalize it before pausing, resizing a card, or keyboard scrolling.
    pub(super) fn scroll_anchor(&self) -> ListOffset {
        let offset = self.list.logical_scroll_top();
        if self.list.item_count() > 0 && offset.item_ix >= self.list.item_count() {
            let top = self.header_bounds.size.height + px(16.);
            let bottom = if self.view.selected.is_some() {
                self.composer_dock_bounds.size.height + px(12.)
            } else {
                px(0.)
            };
            self.list
                .scroll_by(top + bottom - self.list.viewport_bounds().size.height);
        }
        self.list.logical_scroll_top()
    }

    pub(super) fn pause_follow(&mut self) {
        self.scroll_anchor();
        self.follow = false;
    }
}

/// Splicing GPUI's visible item resets its offset to the replacement's start.
/// Restore the same message and within-message position after the splice.
pub(super) fn remap_anchor(
    anchor: ListOffset,
    old: &VecDeque<Arc<Row>>,
    new: &VecDeque<Arc<Row>>,
) -> ListOffset {
    let Some(row) = old.get(anchor.item_ix) else {
        return anchor;
    };
    let closest = |matches: &dyn Fn(&Row) -> bool| {
        new.iter()
            .enumerate()
            .filter(|(_, r)| matches(r))
            .min_by_key(|(ix, _)| ix.abs_diff(anchor.item_ix))
            .map(|(ix, _)| ix)
    };
    let found = closest(&|r| r.key == row.key)
        .or_else(|| {
            row.tool
                .as_ref()
                .filter(|t| !t.id.is_empty())
                .and_then(|tool| closest(&|r| r.tool.as_ref().is_some_and(|t| t.id == tool.id)))
        })
        .or_else(|| closest(&|r| r.same_content(row)))
        .or_else(|| {
            closest(&|r| {
                r.role == row.role
                    && r.tool.is_none()
                    && (r.text.starts_with(&row.text) || row.text.starts_with(&r.text))
            })
        });
    ListOffset {
        item_ix: found.unwrap_or(anchor.item_ix.min(new.len().saturating_sub(1))),
        offset_in_item: if found.is_some() {
            anchor.offset_in_item
        } else {
            px(0.)
        },
    }
}
