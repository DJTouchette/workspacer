use std::{
    ops::Range,
    rc::Rc,
    sync::{Arc, Mutex},
};

use gpui::{
    App, BorderStyle, Bounds, CursorStyle, Edges, Element, ElementId, GlobalElementId, Half,
    HighlightStyle, Hsla, Hitbox, HitboxBehavior, InspectorElementId, IntoElement, LayoutId,
    MouseMoveEvent, MouseUpEvent, Pixels, Point, SharedString, StyledText, TextLayout, Window,
    point, px, quad,
};

use crate::{ActiveTheme, global_state::GlobalState, input::Selection, text::node::LinkMark};

/// A inline element used to render a inline text and support selectable.
///
/// All text in TextView (including the CodeBlock) used this for text rendering.
pub(super) struct Inline {
    on_link_click: Option<Arc<super::LinkClickFn>>,
    id: ElementId,
    text: SharedString,
    links: Rc<Vec<(Range<usize>, LinkMark)>>,
    highlights: Vec<(Range<usize>, HighlightStyle)>,
    /// Font family for inline code; every highlight run inside one of the
    /// ranges uses it (combined highlights only ever split code ranges).
    mono: Option<(SharedString, Vec<Range<usize>>)>,
    /// Rounded, padded fill painted behind inline code (prose mode); text-run
    /// backgrounds can only be square and flush with the glyphs.
    code_background: Option<Hsla>,
    styled_text: StyledText,

    state: Arc<Mutex<InlineState>>,
}

/// The inline text state, used RefCell to keep the selection state.
#[derive(Debug, Default, PartialEq)]
pub(crate) struct InlineState {
    hovered_index: Option<usize>,
    /// The text that actually rendering, matched with selection.
    pub(super) text: SharedString,
    pub(super) selection: Option<Selection>,
}

impl InlineState {
    /// Save actually rendered text for selected text to use.
    pub(crate) fn set_text(&mut self, text: SharedString) {
        self.text = text;
    }
}

impl Inline {
    pub(super) fn new(
        id: impl Into<ElementId>,
        state: Arc<Mutex<InlineState>>,
        links: Vec<(Range<usize>, LinkMark)>,
        highlights: Vec<(Range<usize>, HighlightStyle)>,
        on_link_click: Option<Arc<super::LinkClickFn>>,
    ) -> Self {
        let text = state.lock().unwrap().text.clone();
        Self {
            on_link_click,
            id: id.into(),
            links: Rc::new(links),
            highlights,
            mono: None,
            code_background: None,
            text: text.clone(),
            styled_text: StyledText::new(text),
            state,
        }
    }

    pub(super) fn mono(
        mut self,
        family: Option<SharedString>,
        background: Option<Hsla>,
        ranges: Vec<Range<usize>>,
    ) -> Self {
        self.mono = family.filter(|_| !ranges.is_empty()).map(|f| (f, ranges));
        self.code_background = background;
        self
    }

    /// One rounded quad per visual line of each inline-code range.
    fn paint_code_backgrounds(&self, layout: &TextLayout, window: &mut Window) {
        let (Some((_, ranges)), Some(color)) = (&self.mono, self.code_background) else {
            return;
        };
        let line_height = layout.line_height();
        let (pad, inset) = (px(3.), line_height * 0.12);
        // `position_for_index` ignores text alignment, while painting applies
        // it per visual line; shift right/center-aligned lines (table cells)
        // by the same amount so each quad sits under its code.
        let align = window.text_style().text_align;
        let line_ends: Vec<(Pixels, Pixels)> = if matches!(align, gpui::TextAlign::Left) {
            Vec::new()
        } else {
            let mut ends: Vec<(Pixels, Pixels)> = Vec::new();
            for ix in self
                .text
                .char_indices()
                .map(|(i, _)| i)
                .chain(std::iter::once(self.text.len()))
            {
                if let Some(pos) = layout.position_for_index(ix) {
                    match ends.iter_mut().find(|(y, _)| *y == pos.y) {
                        Some((_, x)) => *x = (*x).max(pos.x),
                        None => ends.push((pos.y, pos.x)),
                    }
                }
            }
            ends
        };
        let right_edge = layout.bounds().right();
        let shift = |y: Pixels| -> Pixels {
            let Some((_, end)) = line_ends.iter().find(|(line, _)| *line == y) else {
                return px(0.);
            };
            match align {
                gpui::TextAlign::Right => right_edge - *end,
                gpui::TextAlign::Center => (right_edge - *end) / 2.,
                gpui::TextAlign::Left => px(0.),
            }
        };
        for range in ranges {
            let Some(code) = self.text.get(range.clone()) else {
                continue;
            };
            let boundaries = code
                .char_indices()
                .map(|(i, _)| range.start + i)
                .chain(std::iter::once(range.end));
            let mut segments = Vec::new();
            let mut start: Option<Point<Pixels>> = None;
            let mut prev: Option<Point<Pixels>> = None;
            let mut width = px(0.);
            for ix in boundaries {
                let Some(pos) = layout.position_for_index(ix) else {
                    continue;
                };
                match (start, prev) {
                    (Some(s), Some(p)) if pos.y != s.y => {
                        segments.push((s, p.x + width));
                        start = Some(pos);
                    }
                    (None, _) => start = Some(pos),
                    _ => {}
                }
                if let Some(p) = prev.filter(|p| p.y == pos.y) {
                    width = pos.x - p.x;
                }
                prev = Some(pos);
            }
            if let (Some(s), Some(p)) = (start, prev) {
                segments.push((s, p.x));
            }
            for (s, right) in segments.into_iter().filter(|(s, r)| *r - s.x > px(1.)) {
                let dx = shift(s.y);
                window.paint_quad(quad(
                    Bounds::from_corners(
                        point(s.x + dx - pad, s.y + inset),
                        point(right + dx + pad, s.y + line_height - inset),
                    ),
                    px(4.),
                    color,
                    Edges::default(),
                    gpui::transparent_black(),
                    BorderStyle::default(),
                ));
            }
        }
    }

    /// Get link at given mouse position.
    fn link_for_position(
        layout: &TextLayout,
        links: &Vec<(Range<usize>, LinkMark)>,
        position: Point<Pixels>,
    ) -> Option<LinkMark> {
        let offset = layout.index_for_position(position).ok()?;
        for (range, link) in links.iter() {
            if range.contains(&offset) {
                return Some(link.clone());
            }
        }

        None
    }

    /// Paint selected bounds for debug.
    #[allow(unused)]
    fn paint_selected_bounds(&self, bounds: Bounds<Pixels>, window: &mut Window, cx: &mut App) {
        window.paint_quad(gpui::PaintQuad {
            bounds,
            background: cx.theme().blue.alpha(0.01).into(),
            corner_radii: gpui::Corners::default(),
            border_color: gpui::transparent_black(),
            border_style: BorderStyle::default(),
            border_widths: gpui::Edges::all(px(0.)),
        });
    }

    fn layout_selections(
        &self,
        text_layout: &TextLayout,
        _window: &mut Window,
        cx: &mut App,
    ) -> (bool, bool, Option<Selection>) {
        let Some(text_view_state) = GlobalState::global(cx).text_view_state() else {
            return (false, false, None);
        };

        let text_view_state = text_view_state.read(cx);
        let is_selectable = text_view_state.is_selectable();
        if !text_view_state.has_selection() {
            return (is_selectable, false, None);
        }

        // Workspacer: select from where the drag started to the pointer in
        // reading order. Upstream selected the characters inside the
        // rectangle spanned by the two points, which dropped the drag's
        // direction: dragging up and to the right (or down and to the left)
        // took in text on the upper line left of the pointer, text it never
        // reached, and that text was copied too.
        let Some((anchor, head)) = text_view_state.selection_points() else {
            return (is_selectable, false, None);
        };
        let boxes = self.text.char_indices().filter_map(|(offset, c)| {
            let (origin, width) = char_box(text_layout, c, offset)?;
            Some((offset, c, origin, width))
        });
        let [start, end] = selection_carets(boxes, [anchor, head], text_layout.line_height());
        let selection = (start != end).then(|| (start.min(end)..start.max(end)).into());
        (true, true, selection)
    }

    /// Paint the selection background.
    fn paint_selection(
        selection: &Selection,
        text: &str,
        text_layout: &TextLayout,
        bounds: &Bounds<Pixels>,
        window: &mut Window,
        cx: &mut App,
    ) {
        let mut start = selection.start;
        let mut end = selection.end;
        if end < start {
            std::mem::swap(&mut start, &mut end);
        }
        // A selection that starts at a soft wrap starts on the next line,
        // not at the end of the line GPUI reports for that index.
        let start_position = match text[start..].chars().next() {
            Some(c) => char_box(text_layout, c, start).map(|(origin, _)| origin),
            None => text_layout.position_for_index(start),
        };
        let Some(start_position) = start_position else {
            return;
        };
        let Some(end_position) = text_layout.position_for_index(end) else {
            return;
        };

        let line_height = text_layout.line_height();
        if start_position.y == end_position.y {
            window.paint_quad(quad(
                Bounds::from_corners(
                    start_position,
                    point(end_position.x, end_position.y + line_height),
                ),
                px(0.),
                cx.theme().selection,
                Edges::default(),
                gpui::transparent_black(),
                BorderStyle::default(),
            ));
        } else {
            window.paint_quad(quad(
                Bounds::from_corners(
                    start_position,
                    point(bounds.right(), start_position.y + line_height),
                ),
                px(0.),
                cx.theme().selection,
                Edges::default(),
                gpui::transparent_black(),
                BorderStyle::default(),
            ));

            if end_position.y > start_position.y + line_height {
                window.paint_quad(quad(
                    Bounds::from_corners(
                        point(bounds.left(), start_position.y + line_height),
                        point(bounds.right(), end_position.y),
                    ),
                    px(0.),
                    cx.theme().selection,
                    Edges::default(),
                    gpui::transparent_black(),
                    BorderStyle::default(),
                ));
            }

            window.paint_quad(quad(
                Bounds::from_corners(
                    point(bounds.left(), end_position.y),
                    point(end_position.x, end_position.y + line_height),
                ),
                px(0.),
                cx.theme().selection,
                Edges::default(),
                gpui::transparent_black(),
                BorderStyle::default(),
            ));
        }
    }
}

impl IntoElement for Inline {
    type Element = Self;

    fn into_element(self) -> Self::Element {
        self
    }
}

impl Element for Inline {
    type RequestLayoutState = ();
    type PrepaintState = Hitbox;

    fn id(&self) -> Option<ElementId> {
        Some(self.id.clone())
    }

    fn source_location(&self) -> Option<&'static std::panic::Location<'static>> {
        None
    }

    fn request_layout(
        &mut self,
        global_element_id: Option<&GlobalElementId>,
        inspector_id: Option<&InspectorElementId>,
        window: &mut Window,
        cx: &mut App,
    ) -> (LayoutId, Self::RequestLayoutState) {
        let text_style = window.text_style();

        let mut runs = Vec::new();
        let mut ix = 0;
        for (range, highlight) in self.highlights.iter() {
            if ix < range.start {
                runs.push(text_style.clone().to_run(range.start - ix));
            }
            let mut run = text_style.clone().highlight(*highlight).to_run(range.len());
            if let Some((family, ranges)) = &self.mono {
                if ranges
                    .iter()
                    .any(|code| code.start <= range.start && range.end <= code.end)
                {
                    run.font.family = family.clone();
                }
            }
            runs.push(run);
            ix = range.end;
        }
        if ix < self.text.len() {
            runs.push(text_style.to_run(self.text.len() - ix));
        }

        self.styled_text = StyledText::new(self.text.clone()).with_runs(runs);
        let (layout_id, _) =
            self.styled_text
                .request_layout(global_element_id, inspector_id, window, cx);

        (layout_id, ())
    }

    fn prepaint(
        &mut self,
        id: Option<&GlobalElementId>,
        inspector_id: Option<&InspectorElementId>,
        bounds: Bounds<Pixels>,
        _: &mut Self::RequestLayoutState,
        window: &mut Window,
        cx: &mut App,
    ) -> Self::PrepaintState {
        self.styled_text
            .prepaint(id, inspector_id, bounds, &mut (), window, cx);

        let hitbox = window.insert_hitbox(bounds, HitboxBehavior::Normal);
        hitbox
    }

    fn paint(
        &mut self,
        global_id: Option<&GlobalElementId>,
        _: Option<&InspectorElementId>,
        bounds: Bounds<Pixels>,
        _: &mut Self::RequestLayoutState,
        prepaint: &mut Self::PrepaintState,
        window: &mut Window,
        cx: &mut App,
    ) {
        let current_view = window.current_view();
        let hitbox = prepaint;
        let mut state = self.state.lock().unwrap();

        let text_layout = self.styled_text.layout().clone();
        self.paint_code_backgrounds(&text_layout, window);

        // layout selections
        let (is_selectable, is_selection, selection) =
            self.layout_selections(&text_layout, window, cx);

        state.selection = selection;

        // Workspacer: the selection is painted under the glyphs (after code
        // backgrounds, before text), as Input does. Painting it on top hid
        // selected text whenever the theme's selection color was opaque.
        if let Some(selection) = &state.selection {
            Self::paint_selection(selection, &self.text, &text_layout, &bounds, window, cx);
        }
        self.styled_text
            .paint(global_id, None, bounds, &mut (), &mut (), window, cx);

        if is_selection || is_selectable {
            window.set_cursor_style(CursorStyle::IBeam, &hitbox);
        }

        // link cursor pointer
        let mouse_position = window.mouse_position();
        if let Some(_) = Self::link_for_position(&text_layout, &self.links, mouse_position) {
            window.set_cursor_style(CursorStyle::PointingHand, &hitbox);
        }

        // mouse move, update hovered link
        window.on_mouse_event({
            let hitbox = hitbox.clone();
            let text_layout = text_layout.clone();
            let mut hovered_index = state.hovered_index;
            move |event: &MouseMoveEvent, phase, window, cx| {
                if !phase.bubble() || !hitbox.is_hovered(window) {
                    return;
                }

                let current = hovered_index;
                let updated = text_layout.index_for_position(event.position).ok();
                //  notify update when hovering over different links
                if current != updated {
                    hovered_index = updated;
                    cx.notify(current_view);
                }
            }
        });

        if !is_selection {
            // click to open link
            window.on_mouse_event({
                let links = self.links.clone();
                let text_layout = text_layout.clone();
                let on_link_click = self.on_link_click.clone();

                move |event: &MouseUpEvent, phase, window, cx| {
                    if !bounds.contains(&event.position) || !phase.bubble() {
                        return;
                    }

                    if let Some(link) =
                        Self::link_for_position(&text_layout, &links, event.position)
                    {
                        cx.stop_propagation();
                        if let Some(on_link_click) = &on_link_click {
                            on_link_click(&link.url, window, cx);
                        } else {
                            cx.open_url(&link.url);
                        }
                    }
                }
            });
        }
    }
}

/// Where character `c` at `offset` is drawn, and its advance (workspacer).
fn char_box(layout: &TextLayout, c: char, offset: usize) -> Option<(Point<Pixels>, Pixels)> {
    let position = layout.position_for_index(offset)?;
    let next = layout.position_for_index(offset + c.len_utf8());
    Some(visual_box(
        c,
        position,
        next,
        layout.bounds().left(),
        layout.line_height(),
    ))
}

/// GPUI reports the index of a soft wrap at the end of the line the wrap
/// ends, but the character there is drawn at the start of the next line.
/// Taking GPUI's position for it put the first character of every wrapped
/// line at the end of the line above, so dragging back to the start of a
/// wrapped line left that character unselected. A newline keeps its own
/// line's end and has no width.
fn visual_box(
    c: char,
    position: Point<Pixels>,
    next: Option<Point<Pixels>>,
    line_left: Pixels,
    line_height: Pixels,
) -> (Point<Pixels>, Pixels) {
    match next {
        _ if c == '\n' => (position, px(0.)),
        Some(next) if next.y == position.y => (position, next.x - position.x),
        Some(next) => (point(line_left, next.y), next.x - line_left),
        None => (position, line_height.half()),
    }
}

/// The caret each selection point puts in the text: after every character
/// that precedes the point in reading order. `boxes` are the characters in
/// text order with where they are drawn (`visual_box`). Selecting between
/// the two carets keeps the drag's direction, which a rectangle between the
/// points does not.
fn selection_carets(
    boxes: impl Iterator<Item = (usize, char, Point<Pixels>, Pixels)>,
    points: [Point<Pixels>; 2],
    line_height: Pixels,
) -> [usize; 2] {
    let mut carets = [0; 2];
    let mut settled = [false; 2];
    for (offset, c, origin, width) in boxes {
        for ix in 0..2 {
            if settled[ix] {
                continue;
            }
            let point = points[ix];
            let precedes = if origin.y + line_height <= point.y {
                // On a line above the point's.
                true
            } else if point.y < origin.y {
                // On a line below it.
                false
            } else {
                // On the point's line, which a newline ends: before the
                // point once the point is past the character's middle.
                c != '\n' && origin.x + width.half() <= point.x
            };
            if precedes {
                carets[ix] = offset + c.len_utf8();
            } else {
                settled[ix] = true;
            }
        }
        if settled == [true; 2] {
            break;
        }
    }
    carets
}

#[cfg(test)]
mod tests {
    use super::{selection_carets, visual_box};
    use gpui::{Pixels, Point, point, px};

    const CHAR: f32 = 10.;
    const LINE: f32 = 20.;

    /// "abc def\nghi" in a 10px monospace font, soft wrapped before "def",
    /// at the positions GPUI's `position_for_index` reports: the wrap index
    /// (4, "d") at the end of the first line.
    fn gpui_position(ix: usize) -> Point<Pixels> {
        let (x, line) = match ix {
            0..=4 => (ix, 0),
            5..=7 => (ix - 4, 1),
            _ => (ix - 8, 2),
        };
        point(px(x as f32 * CHAR), px(line as f32 * LINE))
    }

    fn boxes() -> Vec<(usize, char, Point<Pixels>, Pixels)> {
        "abc def\nghi"
            .char_indices()
            .map(|(ix, c)| {
                let (origin, width) = visual_box(
                    c,
                    gpui_position(ix),
                    Some(gpui_position(ix + 1)),
                    px(0.),
                    px(LINE),
                );
                (ix, c, origin, width)
            })
            .collect()
    }

    fn selected(anchor: (f32, f32), head: (f32, f32)) -> &'static str {
        let points = [anchor, head].map(|(x, y)| point(px(x), px(y)));
        let [a, b] = selection_carets(boxes().into_iter(), points, px(LINE));
        &"abc def\nghi"[a.min(b)..a.max(b)]
    }

    #[test]
    fn wrapped_characters_are_drawn_on_their_own_line() {
        let boxes = boxes();
        // The wrap index starts the second line.
        assert_eq!(boxes[4].2, point(px(0.), px(LINE)));
        assert_eq!(boxes[4].3, px(CHAR));
        // The space before the wrap ends the first line.
        assert_eq!(boxes[3].2, point(px(30.), px(0.)));
        assert_eq!(boxes[3].3, px(CHAR));
        // A newline stays at its line's end, without width.
        assert_eq!(boxes[7].2, point(px(30.), px(LINE)));
        assert_eq!(boxes[7].3, px(0.));
    }

    #[test]
    fn selection_follows_the_drag_in_reading_order() {
        // Within a line, either way.
        assert_eq!(selected((25., 5.), (5., 5.)), "bc");
        assert_eq!(selected((5., 5.), (25., 5.)), "bc");
        // Backward up and to the right: from after "e" back to after "c".
        // A rectangle between the points also took "bc" on the line above
        // and "f" on the line below.
        assert_eq!(selected((15., 25.), (25., 5.)), " de");
        // Forward down and to the left, the same span.
        assert_eq!(selected((25., 5.), (15., 25.)), " de");
        // Backward up and to the left.
        assert_eq!(selected((15., 45.), (5., 5.)), "bc def\ngh");
    }

    #[test]
    fn selection_reaches_the_start_of_a_wrapped_line() {
        // Dragging back past the start of the wrapped line takes its first
        // character, and nothing from the line above.
        assert_eq!(selected((15., 25.), (-5., 25.)), "de");
        assert_eq!(selected((-5., 25.), (15., 25.)), "de");
        // Past the end of the first line is the same caret as its wrap.
        assert_eq!(selected((90., 5.), (15., 25.)), "de");
        // Past the end of a line ending in a newline stops before it.
        assert_eq!(selected((90., 25.), (-5., 25.)), "def");
        // Across the newline, it is kept.
        assert_eq!(selected((90., 25.), (15., 45.)), "\ngh");
    }

    #[test]
    fn a_line_owns_its_top_edge_only() {
        // The second line's top edge is in the second line: the first line
        // is not reached.
        assert_eq!(selected((15., 25.), (12., LINE)), "e");
        // Just above it is in the first line.
        assert_eq!(selected((15., 25.), (12., LINE - 0.5)), "bc de");
        // Above and below the text clamp to its ends.
        assert_eq!(selected((5., -10.), (-5., 5.)), "");
        assert_eq!(selected((25., -10.), (25., 100.)), "abc def\nghi");
    }
}
