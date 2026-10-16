//! A text field on GPUI alone (replaces gpui-component's input; works on macOS and iOS): single- or multi-line with
//! soft wrap, auto-grow between a min and max number of rows (then it scrolls), placeholder, masked (tokens), a
//! validator, IME (marked text, via `EntityInputHandler`), selection by mouse and keys, word moves, clipboard, undo.
//! Events: Change, PressEnter { secondary }, Focus, Blur. Key context "Input" (the composer binds Enter / ⌘Enter /
//! ⇧Enter / ⌘V under "Composer > Input").
use gpui::prelude::FluentBuilder;
use gpui::*;
use std::ops::Range;
use unicode_segmentation::UnicodeSegmentation;

actions!(
    input,
    [
        Backspace, Delete, DeleteWordLeft, DeleteToLineStart, Left, Right, Up, Down, WordLeft, WordRight, LineStart, LineEnd,
        DocStart, DocEnd, SelectLeft, SelectRight, SelectUp, SelectDown, SelectWordLeft, SelectWordRight, SelectLineStart,
        SelectLineEnd, SelectDocStart, SelectDocEnd, SelectAll, Copy, Cut, Paste, Undo, Redo, Enter, SecondaryEnter,
        ShowCharacterPalette
    ]
);

pub fn bind(cx: &mut App) {
    let c = Some("Input");
    cx.bind_keys([
        KeyBinding::new("backspace", Backspace, c),
        KeyBinding::new("shift-backspace", Backspace, c),
        KeyBinding::new("delete", Delete, c),
        KeyBinding::new("alt-backspace", DeleteWordLeft, c),
        KeyBinding::new("secondary-backspace", DeleteToLineStart, c),
        KeyBinding::new("left", Left, c),
        KeyBinding::new("right", Right, c),
        KeyBinding::new("up", Up, c),
        KeyBinding::new("down", Down, c),
        KeyBinding::new("alt-left", WordLeft, c),
        KeyBinding::new("alt-right", WordRight, c),
        KeyBinding::new("secondary-left", LineStart, c),
        KeyBinding::new("secondary-right", LineEnd, c),
        KeyBinding::new("home", LineStart, c),
        KeyBinding::new("end", LineEnd, c),
        KeyBinding::new("ctrl-a", LineStart, c),
        KeyBinding::new("ctrl-e", LineEnd, c),
        KeyBinding::new("secondary-up", DocStart, c),
        KeyBinding::new("secondary-down", DocEnd, c),
        KeyBinding::new("shift-left", SelectLeft, c),
        KeyBinding::new("shift-right", SelectRight, c),
        KeyBinding::new("shift-up", SelectUp, c),
        KeyBinding::new("shift-down", SelectDown, c),
        KeyBinding::new("alt-shift-left", SelectWordLeft, c),
        KeyBinding::new("alt-shift-right", SelectWordRight, c),
        KeyBinding::new("secondary-shift-left", SelectLineStart, c),
        KeyBinding::new("secondary-shift-right", SelectLineEnd, c),
        KeyBinding::new("secondary-shift-up", SelectDocStart, c),
        KeyBinding::new("secondary-shift-down", SelectDocEnd, c),
        KeyBinding::new("secondary-a", SelectAll, c),
        KeyBinding::new("secondary-c", Copy, c),
        KeyBinding::new("secondary-x", Cut, c),
        KeyBinding::new("secondary-v", Paste, c),
        KeyBinding::new("secondary-z", Undo, c),
        KeyBinding::new("secondary-shift-z", Redo, c),
        KeyBinding::new("enter", Enter, c),
        KeyBinding::new("secondary-enter", SecondaryEnter, c),
        KeyBinding::new("ctrl-cmd-space", ShowCharacterPalette, c),
    ]);
}

#[derive(Clone, Debug, PartialEq)]
pub enum InputEvent {
    Change,
    PressEnter { secondary: bool },
    Focus,
    Blur,
}

type Validator = Box<dyn Fn(&str, &mut Context<InputState>) -> bool>;

pub struct InputState {
    focus_handle: FocusHandle,
    content: String,
    placeholder: SharedString,
    selected: Range<usize>,
    reversed: bool,
    marked: Option<Range<usize>>,
    multi_line: bool,
    rows: (usize, usize),
    masked: bool,
    validate: Option<Validator>,
    pub disabled: bool,
    /// focus asked for before the field was drawn: applied when it is
    want_focus: bool,
    /// the "Paste" bubble shown above this point (window coordinates), after a long press (iOS) / right click
    pub paste_at: Option<Point<Pixels>>,
    /// how the field is drawn (from `Input`): small, boxed, disabled, text size
    opts: (bool, bool, bool, Option<Pixels>),
    undo: Vec<(String, Range<usize>)>,
    redo: Vec<(String, Range<usize>)>,
    selecting: bool,
    /// from the last paint: the lines, where they start (byte offsets in the display text), bounds, line height, scroll
    layout: Option<Laid>,
    scroll_y: Pixels,
    /// up/down keep the column they started from
    goal_x: Option<Pixels>,
    _subs: Vec<Subscription>,
}

struct Laid {
    lines: Vec<(usize, WrappedLine)>,
    bounds: Bounds<Pixels>,
    line_height: Pixels,
    display: String,
}

impl EventEmitter<InputEvent> for InputState {}

impl InputState {
    pub fn new(window: &mut Window, cx: &mut Context<Self>) -> Self {
        let focus_handle = cx.focus_handle();
        let s1 = cx.on_focus(&focus_handle, window, |_, _, cx| {
            cx.emit(InputEvent::Focus);
            cx.notify();
        });
        let s2 = cx.on_blur(&focus_handle, window, |this: &mut Self, _, cx| {
            this.selecting = false;
            this.paste_at = None;
            cx.emit(InputEvent::Blur);
            cx.notify();
        });
        InputState {
            focus_handle,
            content: String::new(),
            placeholder: SharedString::default(),
            selected: 0..0,
            reversed: false,
            marked: None,
            multi_line: false,
            rows: (1, 1),
            masked: false,
            validate: None,
            disabled: false,
            want_focus: false,
            opts: (false, true, false, None),
            paste_at: None,
            undo: vec![],
            redo: vec![],
            selecting: false,
            layout: None,
            scroll_y: px(0.),
            goal_x: None,
            _subs: vec![s1, s2],
        }
    }
    pub fn multi_line(mut self, on: bool) -> Self {
        self.multi_line = on;
        if on && self.rows == (1, 1) {
            self.rows = (2, 12);
        }
        self
    }
    pub fn auto_grow(mut self, min: usize, max: usize) -> Self {
        self.rows = (min.max(1), max.max(min.max(1)));
        self
    }
    pub fn placeholder(mut self, p: impl Into<SharedString>) -> Self {
        self.placeholder = p.into();
        self
    }
    pub fn masked(mut self, on: bool) -> Self {
        self.masked = on;
        self
    }
    pub fn validate(mut self, f: impl Fn(&str, &mut Context<InputState>) -> bool + 'static) -> Self {
        self.validate = Some(Box::new(f));
        self
    }
    pub fn default_value(mut self, v: impl Into<SharedString>) -> Self {
        self.content = v.into().to_string();
        self.selected = self.content.len()..self.content.len();
        self
    }

    pub fn value(&self) -> SharedString {
        SharedString::from(self.content.clone())
    }
    pub fn set_value(&mut self, v: impl Into<SharedString>, window: &mut Window, cx: &mut Context<Self>) {
        let v = v.into().to_string();
        if v == self.content {
            return;
        }
        window.notify_text_input_changed(&self.focus_handle);
        self.content = v;
        self.selected = self.content.len()..self.content.len();
        self.reversed = false;
        self.marked = None;
        self.goal_x = None;
        cx.emit(InputEvent::Change);
        cx.notify();
    }
    pub fn set_placeholder(&mut self, p: impl Into<SharedString>, _window: &mut Window, cx: &mut Context<Self>) {
        self.placeholder = p.into();
        cx.notify();
    }
    pub fn focus(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        window.focus(&self.focus_handle, cx);
        // iOS: the on-screen keyboard (a no-op on the desktop)
        window.request_virtual_keyboard();
        self.want_focus = true;
        cx.notify();
    }
    /// the caret (a byte offset)
    pub fn cursor(&self) -> usize {
        if self.reversed { self.selected.start } else { self.selected.end }
    }
    pub fn set_cursor(&mut self, offset: usize, _window: &mut Window, cx: &mut Context<Self>) {
        let o = self.clamp(offset);
        self.selected = o..o;
        self.reversed = false;
        cx.notify();
    }
    /// insert at the caret (replacing the selection)
    pub fn insert(&mut self, text: &str, window: &mut Window, cx: &mut Context<Self>) {
        self.replace(None, text, window, cx);
    }

    fn clamp(&self, o: usize) -> usize {
        let o = o.min(self.content.len());
        (0..=o).rev().find(|p| self.content.is_char_boundary(*p)).unwrap_or(0)
    }
    fn push_undo(&mut self) {
        self.undo.push((self.content.clone(), self.selected.clone()));
        if self.undo.len() > 200 {
            self.undo.remove(0);
        }
        self.redo.clear();
    }
    /// replace `range` (else the marked text, else the selection) with `text`; validated
    fn replace(&mut self, range: Option<Range<usize>>, text: &str, _window: &mut Window, cx: &mut Context<Self>) -> bool {
        if self.disabled {
            return false;
        }
        let text = if self.multi_line { text.to_string() } else { text.replace(['\n', '\r'], " ") };
        let r = range.or(self.marked.clone()).unwrap_or(self.selected.clone());
        let r = self.clamp(r.start)..self.clamp(r.end);
        let next = format!("{}{}{}", &self.content[..r.start], text, &self.content[r.end..]);
        if let Some(v) = self.validate.take() {
            let ok = v(&next, cx);
            self.validate = Some(v);
            if !ok {
                return false;
            }
        }
        self.push_undo();
        self.paste_at = None;
        self.content = next;
        let c = r.start + text.len();
        self.selected = c..c;
        self.reversed = false;
        self.marked = None;
        self.goal_x = None;
        _window.notify_text_input_changed(&self.focus_handle);
        cx.emit(InputEvent::Change);
        cx.notify();
        true
    }
    fn move_to(&mut self, o: usize, cx: &mut Context<Self>) {
        let o = self.clamp(o);
        self.selected = o..o;
        self.reversed = false;
        cx.notify();
    }
    fn select_to(&mut self, o: usize, cx: &mut Context<Self>) {
        let o = self.clamp(o);
        if self.reversed { self.selected.start = o } else { self.selected.end = o }
        if self.selected.end < self.selected.start {
            self.reversed = !self.reversed;
            self.selected = self.selected.end..self.selected.start;
        }
        cx.notify();
    }
    fn prev_boundary(&self, o: usize) -> usize {
        self.content.grapheme_indices(true).rev().find_map(|(i, _)| (i < o).then_some(i)).unwrap_or(0)
    }
    fn next_boundary(&self, o: usize) -> usize {
        self.content.grapheme_indices(true).find_map(|(i, _)| (i > o).then_some(i)).unwrap_or(self.content.len())
    }
    fn word_left(&self, o: usize) -> usize {
        let b = self.content[..o].trim_end_matches(|c: char| !c.is_alphanumeric());
        b.char_indices().rev().find(|(_, c)| !c.is_alphanumeric()).map(|(i, c)| i + c.len_utf8()).unwrap_or(0)
    }
    fn word_right(&self, o: usize) -> usize {
        let rest = &self.content[o..];
        let skip = rest.len() - rest.trim_start_matches(|c: char| !c.is_alphanumeric()).len();
        let after = &rest[skip..];
        o + skip + after.find(|c: char| !c.is_alphanumeric()).unwrap_or(after.len())
    }
    fn line_start(&self, o: usize) -> usize {
        if let Some(p) = self.visual_row_start(o) {
            return p;
        }
        self.content[..o].rfind('\n').map(|i| i + 1).unwrap_or(0)
    }
    fn line_end(&self, o: usize) -> usize {
        if let Some(p) = self.visual_row_end(o) {
            return p;
        }
        self.content[o..].find('\n').map(|i| o + i).unwrap_or(self.content.len())
    }

    // ---- the display text (masked: one * per char) and its offsets -------------------------------------------------
    fn display(&self) -> String {
        if self.masked { "*".repeat(self.content.chars().count()) } else { self.content.clone() }
    }
    fn to_display(&self, o: usize) -> usize {
        if self.masked { self.content[..self.clamp(o)].chars().count() } else { o }
    }
    fn from_display(&self, d: usize) -> usize {
        if self.masked { self.content.char_indices().nth(d).map(|(i, _)| i).unwrap_or(self.content.len()) } else { self.clamp(d) }
    }

    // ---- positions from the last layout -------------------------------------------------------------------------
    /// the caret's point (relative to the text's origin, before scrolling) for a content offset
    fn point_for(&self, o: usize) -> Option<Point<Pixels>> {
        let l = self.layout.as_ref()?;
        let d = self.to_display(o);
        let mut y = px(0.);
        for (k, (start, line)) in l.lines.iter().enumerate() {
            let end = start + line.len();
            if d <= end || k + 1 == l.lines.len() {
                let p = line.position_for_index(d.saturating_sub(*start).min(line.len()), l.line_height)?;
                return Some(point(p.x, y + p.y));
            }
            y += line.size(l.line_height).height;
        }
        Some(point(px(0.), px(0.)))
    }
    /// the content offset closest to a point (relative to the text's origin, before scrolling)
    fn offset_for(&self, p: Point<Pixels>) -> usize {
        let Some(l) = self.layout.as_ref() else { return self.content.len() };
        if p.y < px(0.) {
            return 0;
        }
        let mut y = px(0.);
        for (start, line) in &l.lines {
            let h = line.size(l.line_height).height;
            if p.y < y + h {
                let local = point(p.x.max(px(0.)), p.y - y);
                let i = match line.closest_index_for_position(local, l.line_height) {
                    Ok(i) | Err(i) => i,
                };
                return self.from_display(start + i);
            }
            y += h;
        }
        self.content.len()
    }
    fn row_starts(&self, line: &WrappedLine) -> Vec<usize> {
        let runs = line.runs();
        std::iter::once(0).chain(line.wrap_boundaries().iter().map(|b| runs[b.run_ix].glyphs[b.glyph_ix].index)).collect()
    }
    fn visual_row_start(&self, o: usize) -> Option<usize> {
        let l = self.layout.as_ref()?;
        let d = self.to_display(o);
        for (start, line) in &l.lines {
            if d <= start + line.len() {
                let local = d - start;
                let rs = self.row_starts(line);
                let s = rs.iter().rev().find(|r| **r <= local).copied().unwrap_or(0);
                return Some(self.from_display(start + s));
            }
        }
        None
    }
    fn visual_row_end(&self, o: usize) -> Option<usize> {
        let l = self.layout.as_ref()?;
        let d = self.to_display(o);
        for (start, line) in &l.lines {
            if d <= start + line.len() {
                let local = d - start;
                let rs = self.row_starts(line);
                let e = rs.iter().find(|r| **r > local).copied().unwrap_or(line.len());
                // a soft-wrapped row ends before the space the wrap happened at
                let e = if e < line.len() { e.saturating_sub(0) } else { e };
                return Some(self.from_display(start + e));
            }
        }
        None
    }
    fn vertical(&mut self, dir: f32, select: bool, cx: &mut Context<Self>) {
        let cur = self.cursor();
        let Some(p) = self.point_for(cur) else { return };
        let lh = self.layout.as_ref().map(|l| l.line_height).unwrap_or(px(20.));
        let x = *self.goal_x.get_or_insert(p.x);
        let target = point(x, p.y + lh * dir + lh / 2.);
        let o = if target.y < px(0.) { 0 } else { self.offset_for(target) };
        let goal = self.goal_x;
        if select { self.select_to(o, cx) } else { self.move_to(o, cx) }
        self.goal_x = goal;
    }

    // ---- actions ------------------------------------------------------------------------------------------------
    fn backspace(&mut self, _: &Backspace, w: &mut Window, cx: &mut Context<Self>) {
        if self.selected.is_empty() {
            let p = self.prev_boundary(self.cursor());
            self.select_to(p, cx);
        }
        self.replace(None, "", w, cx);
    }
    fn delete(&mut self, _: &Delete, w: &mut Window, cx: &mut Context<Self>) {
        if self.selected.is_empty() {
            let n = self.next_boundary(self.cursor());
            self.select_to(n, cx);
        }
        self.replace(None, "", w, cx);
    }
    fn delete_word_left(&mut self, _: &DeleteWordLeft, w: &mut Window, cx: &mut Context<Self>) {
        if self.selected.is_empty() {
            let p = self.word_left(self.cursor());
            self.select_to(p, cx);
        }
        self.replace(None, "", w, cx);
    }
    fn delete_to_line_start(&mut self, _: &DeleteToLineStart, w: &mut Window, cx: &mut Context<Self>) {
        if self.selected.is_empty() {
            let p = self.line_start(self.cursor());
            self.select_to(p, cx);
        }
        self.replace(None, "", w, cx);
    }
    fn left(&mut self, _: &Left, _: &mut Window, cx: &mut Context<Self>) {
        let o = if self.selected.is_empty() { self.prev_boundary(self.cursor()) } else { self.selected.start };
        self.move_to(o, cx);
    }
    fn right(&mut self, _: &Right, _: &mut Window, cx: &mut Context<Self>) {
        let o = if self.selected.is_empty() { self.next_boundary(self.cursor()) } else { self.selected.end };
        self.move_to(o, cx);
    }
    fn up(&mut self, _: &Up, _: &mut Window, cx: &mut Context<Self>) {
        if !self.multi_line {
            return cx.propagate();
        }
        self.vertical(-1., false, cx);
    }
    fn down(&mut self, _: &Down, _: &mut Window, cx: &mut Context<Self>) {
        if !self.multi_line {
            return cx.propagate();
        }
        self.vertical(1., false, cx);
    }
    fn word_left_a(&mut self, _: &WordLeft, _: &mut Window, cx: &mut Context<Self>) {
        let o = self.word_left(self.cursor());
        self.move_to(o, cx);
    }
    fn word_right_a(&mut self, _: &WordRight, _: &mut Window, cx: &mut Context<Self>) {
        let o = self.word_right(self.cursor());
        self.move_to(o, cx);
    }
    fn line_start_a(&mut self, _: &LineStart, _: &mut Window, cx: &mut Context<Self>) {
        let o = self.line_start(self.cursor());
        self.move_to(o, cx);
    }
    fn line_end_a(&mut self, _: &LineEnd, _: &mut Window, cx: &mut Context<Self>) {
        let o = self.line_end(self.cursor());
        self.move_to(o, cx);
    }
    fn doc_start(&mut self, _: &DocStart, _: &mut Window, cx: &mut Context<Self>) {
        self.move_to(0, cx);
    }
    fn doc_end(&mut self, _: &DocEnd, _: &mut Window, cx: &mut Context<Self>) {
        let n = self.content.len();
        self.move_to(n, cx);
    }
    fn select_left(&mut self, _: &SelectLeft, _: &mut Window, cx: &mut Context<Self>) {
        let o = self.prev_boundary(self.cursor());
        self.select_to(o, cx);
    }
    fn select_right(&mut self, _: &SelectRight, _: &mut Window, cx: &mut Context<Self>) {
        let o = self.next_boundary(self.cursor());
        self.select_to(o, cx);
    }
    fn select_up(&mut self, _: &SelectUp, _: &mut Window, cx: &mut Context<Self>) {
        self.vertical(-1., true, cx);
    }
    fn select_down(&mut self, _: &SelectDown, _: &mut Window, cx: &mut Context<Self>) {
        self.vertical(1., true, cx);
    }
    fn select_word_left(&mut self, _: &SelectWordLeft, _: &mut Window, cx: &mut Context<Self>) {
        let o = self.word_left(self.cursor());
        self.select_to(o, cx);
    }
    fn select_word_right(&mut self, _: &SelectWordRight, _: &mut Window, cx: &mut Context<Self>) {
        let o = self.word_right(self.cursor());
        self.select_to(o, cx);
    }
    fn select_line_start(&mut self, _: &SelectLineStart, _: &mut Window, cx: &mut Context<Self>) {
        let o = self.line_start(self.cursor());
        self.select_to(o, cx);
    }
    fn select_line_end(&mut self, _: &SelectLineEnd, _: &mut Window, cx: &mut Context<Self>) {
        let o = self.line_end(self.cursor());
        self.select_to(o, cx);
    }
    fn select_doc_start(&mut self, _: &SelectDocStart, _: &mut Window, cx: &mut Context<Self>) {
        self.select_to(0, cx);
    }
    fn select_doc_end(&mut self, _: &SelectDocEnd, _: &mut Window, cx: &mut Context<Self>) {
        let n = self.content.len();
        self.select_to(n, cx);
    }
    fn select_all(&mut self, _: &SelectAll, _: &mut Window, cx: &mut Context<Self>) {
        self.selected = 0..self.content.len();
        self.reversed = false;
        cx.notify();
    }
    fn copy(&mut self, _: &Copy, _: &mut Window, cx: &mut Context<Self>) {
        if !self.selected.is_empty() && !self.masked {
            cx.write_to_clipboard(ClipboardItem::new_string(self.content[self.selected.clone()].to_string()));
        }
    }
    fn cut(&mut self, _: &Cut, w: &mut Window, cx: &mut Context<Self>) {
        if !self.selected.is_empty() && !self.masked {
            cx.write_to_clipboard(ClipboardItem::new_string(self.content[self.selected.clone()].to_string()));
            self.replace(None, "", w, cx);
        }
    }
    fn paste(&mut self, _: &Paste, w: &mut Window, cx: &mut Context<Self>) {
        if let Some(t) = cx.read_from_clipboard().and_then(|i| i.text()) {
            self.replace(None, &t, w, cx);
        }
    }
    fn undo_a(&mut self, _: &Undo, _: &mut Window, cx: &mut Context<Self>) {
        if let Some((c, s)) = self.undo.pop() {
            self.redo.push((std::mem::replace(&mut self.content, c), self.selected.clone()));
            self.selected = s;
            cx.emit(InputEvent::Change);
            cx.notify();
        }
    }
    fn redo_a(&mut self, _: &Redo, _: &mut Window, cx: &mut Context<Self>) {
        if let Some((c, s)) = self.redo.pop() {
            self.undo.push((std::mem::replace(&mut self.content, c), self.selected.clone()));
            self.selected = s;
            cx.emit(InputEvent::Change);
            cx.notify();
        }
    }
    fn enter(&mut self, _: &Enter, w: &mut Window, cx: &mut Context<Self>) {
        cx.emit(InputEvent::PressEnter { secondary: false });
        if self.multi_line {
            self.replace(None, "\n", w, cx);
        } else {
            // a single-line field leaves Enter to whoever binds it around it too (the palette, dialogs)
            cx.propagate();
        }
    }
    fn secondary_enter(&mut self, _: &SecondaryEnter, _: &mut Window, cx: &mut Context<Self>) {
        cx.emit(InputEvent::PressEnter { secondary: true });
        cx.propagate();
    }
    fn character_palette(&mut self, _: &ShowCharacterPalette, w: &mut Window, _: &mut Context<Self>) {
        w.show_character_palette();
    }

    /// a long press (iOS; the backend has no edit menu) or a right click at `pos`: the caret goes there and, when the
    /// clipboard has text, a "Paste" bubble shows above it
    pub fn long_press(&mut self, pos: Point<Pixels>, window: &mut Window, cx: &mut Context<Self>) {
        window.focus(&self.focus_handle, cx);
        window.request_virtual_keyboard();
        if let Some(l) = self.layout.as_ref() {
            let o = self.offset_for(point(pos.x - l.bounds.left(), pos.y - l.bounds.top() + self.scroll_y));
            self.selected = o..o;
            self.reversed = false;
            window.notify_text_selection_changed(&self.focus_handle);
        }
        let has = !self.disabled && clip_text(cx).is_some_and(|t| !t.is_empty());
        self.paste_at = has.then_some(pos);
        cx.notify();
    }
    fn paste_bubble(&self, at: Point<Pixels>, cx: &mut Context<Self>) -> AnyElement {
        let pill = div()
            .id("paste-bubble")
            .flex()
            .items_center()
            .h(px(36.))
            .px(px(14.))
            .rounded(px(9.))
            .bg(hsla(0., 0., 0.17, 0.98))
            .border_1()
            .border_color(hsla(0., 0., 1., 0.08))
            .shadow(vec![BoxShadow { color: hsla(0., 0., 0., 0.45), offset: point(px(0.), px(6.)), blur_radius: px(18.), spread_radius: px(0.), inset: false }])
            .text_size(px(14.))
            .text_color(white())
            .cursor_pointer()
            .child("Paste")
            .child(crate::ui::probe("input.paste"))
            .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
            .on_mouse_down_out(cx.listener(|s, _, _, cx| {
                s.paste_at = None;
                cx.notify();
            }))
            .on_click(cx.listener(|s, _, window, cx| {
                s.paste_at = None;
                if let Some(t) = clip_text(cx) {
                    s.replace(None, &t, window, cx);
                }
                cx.notify();
            }));
        deferred(anchored().position(point(at.x - px(30.), at.y - px(48.))).snap_to_window_with_margin(px(8.)).child(pill)).with_priority(4).into_any_element()
    }
    fn right_down(&mut self, e: &MouseDownEvent, window: &mut Window, cx: &mut Context<Self>) {
        cx.stop_propagation();
        self.long_press(e.position, window, cx);
    }
    fn mouse_down(&mut self, e: &MouseDownEvent, window: &mut Window, cx: &mut Context<Self>) {
        self.paste_at = None;
        window.focus(&self.focus_handle, cx);
        window.request_virtual_keyboard();
        let Some(l) = self.layout.as_ref() else { return };
        let local = point(e.position.x - l.bounds.left(), e.position.y - l.bounds.top() + self.scroll_y);
        let o = self.offset_for(local);
        self.goal_x = None;
        if e.click_count >= 3 {
            let s = self.content[..o].rfind('\n').map(|i| i + 1).unwrap_or(0);
            let t = self.content[o..].find('\n').map(|i| o + i).unwrap_or(self.content.len());
            self.selected = s..t;
            self.reversed = false;
        } else if e.click_count == 2 {
            let s = self.word_left(self.next_boundary(o).min(self.content.len()));
            let t = self.word_right(s);
            self.selected = s.min(o)..t.max(o);
            self.reversed = false;
        } else if e.modifiers.shift {
            self.select_to(o, cx);
        } else {
            self.move_to(o, cx);
            self.selecting = true;
        }
        cx.notify();
    }
    fn mouse_move(&mut self, e: &MouseMoveEvent, _: &mut Window, cx: &mut Context<Self>) {
        if !self.selecting || e.pressed_button != Some(MouseButton::Left) {
            return;
        }
        let Some(l) = self.layout.as_ref() else { return };
        let local = point(e.position.x - l.bounds.left(), e.position.y - l.bounds.top() + self.scroll_y);
        let o = self.offset_for(local);
        self.select_to(o, cx);
    }
    fn mouse_up(&mut self, _: &MouseUpEvent, _: &mut Window, _: &mut Context<Self>) {
        self.selecting = false;
    }
    fn scroll(&mut self, e: &ScrollWheelEvent, window: &mut Window, cx: &mut Context<Self>) {
        let Some(l) = self.layout.as_ref() else { return };
        let total: Pixels = l.lines.iter().map(|(_, x)| x.size(l.line_height).height).sum();
        let max = (total - l.bounds.size.height).max(px(0.));
        if max <= px(0.) {
            return cx.propagate();
        }
        let dy = e.delta.pixel_delta(window.line_height()).y;
        self.scroll_y = (self.scroll_y - dy).clamp(px(0.), max);
        cx.stop_propagation();
        cx.notify();
    }

    // ---- utf-16 for the platform's input handler -------------------------------------------------------------------
    fn to_utf16(&self, o: usize) -> usize {
        self.content[..self.clamp(o)].encode_utf16().count()
    }
    fn from_utf16(&self, u: usize) -> usize {
        let mut n = 0;
        for (i, ch) in self.content.char_indices() {
            if n >= u {
                return i;
            }
            n += ch.len_utf16();
        }
        self.content.len()
    }
    fn range_from_utf16(&self, r: &Range<usize>) -> Range<usize> {
        self.from_utf16(r.start)..self.from_utf16(r.end)
    }
    fn range_to_utf16(&self, r: &Range<usize>) -> Range<usize> {
        self.to_utf16(r.start)..self.to_utf16(r.end)
    }
}

impl Focusable for InputState {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.focus_handle.clone()
    }
}

impl EntityInputHandler for InputState {
    fn text_for_range(&mut self, r: Range<usize>, actual: &mut Option<Range<usize>>, _: &mut Window, _: &mut Context<Self>) -> Option<String> {
        let r = self.range_from_utf16(&r);
        actual.replace(self.range_to_utf16(&r));
        Some(self.content[r].to_string())
    }
    fn selected_text_range(&mut self, _ignore_disabled: bool, _: &mut Window, _: &mut Context<Self>) -> Option<UTF16Selection> {
        Some(UTF16Selection { range: self.range_to_utf16(&self.selected), reversed: self.reversed })
    }
    fn marked_text_range(&self, _: &mut Window, _: &mut Context<Self>) -> Option<Range<usize>> {
        self.marked.as_ref().map(|r| self.range_to_utf16(r))
    }
    fn text_length_utf16(&mut self, _: &mut Window, _: &mut Context<Self>) -> Option<usize> {
        Some(self.content.encode_utf16().count())
    }
    fn accepts_text_input(&self, _: &mut Window, _: &mut Context<Self>) -> bool {
        !self.disabled
    }
    fn set_selected_text_range(&mut self, r: Range<usize>, window: &mut Window, cx: &mut Context<Self>) {
        self.selected = self.range_from_utf16(&r);
        self.reversed = false;
        window.notify_text_selection_changed(&self.focus_handle);
        cx.notify();
    }
    fn text_input_configuration(&mut self, _: &mut Window, _: &mut Context<Self>) -> TextInputConfiguration {
        // tokens: no autocorrect or suggestions; prose: the keyboard's usual help
        TextInputConfiguration { autocorrect: !self.masked, suggestions: !self.masked, autocapitalize: if self.masked || !self.multi_line { Autocapitalize::None } else { Autocapitalize::Sentences }, ..Default::default() }
    }
    fn selection_bounds_for_range(&mut self, r: Range<usize>, bounds: Bounds<Pixels>, _: &mut Window, _: &mut Context<Self>) -> Vec<Bounds<Pixels>> {
        // one rectangle per visual row the range covers
        let r = self.range_from_utf16(&r);
        let Some(lh) = self.layout.as_ref().map(|l| l.line_height) else { return vec![] };
        let (Some(a), Some(b)) = (self.point_for(r.start), self.point_for(r.end)) else { return vec![] };
        let off = point(bounds.left(), bounds.top() - self.scroll_y);
        let w = bounds.size.width;
        let rect = |x0: Pixels, y0: Pixels, x1: Pixels, y1: Pixels| Bounds::from_corners(point(off.x + x0, off.y + y0), point(off.x + x1, off.y + y1));
        if (a.y - b.y).abs() < px(1.) {
            return vec![rect(a.x, a.y, b.x.max(a.x + px(1.)), a.y + lh)];
        }
        let mut v = vec![rect(a.x, a.y, w, a.y + lh)];
        if b.y - a.y > lh + px(0.5) {
            v.push(rect(px(0.), a.y + lh, w, b.y));
        }
        v.push(rect(px(0.), b.y, b.x, b.y + lh));
        v
    }
    fn unmark_text(&mut self, _: &mut Window, cx: &mut Context<Self>) {
        self.marked = None;
        cx.notify();
    }
    fn replace_text_in_range(&mut self, r: Option<Range<usize>>, text: &str, w: &mut Window, cx: &mut Context<Self>) {
        let r = r.map(|r| self.range_from_utf16(&r));
        self.replace(r, text, w, cx);
    }
    fn replace_and_mark_text_in_range(&mut self, r: Option<Range<usize>>, text: &str, sel: Option<Range<usize>>, w: &mut Window, cx: &mut Context<Self>) {
        let range = r.map(|r| self.range_from_utf16(&r)).or(self.marked.clone()).unwrap_or(self.selected.clone());
        let start = range.start;
        // marked (IME composition) text is not validated or undone step by step
        if self.disabled {
            return;
        }
        let range = self.clamp(range.start)..self.clamp(range.end);
        self.content = format!("{}{}{}", &self.content[..range.start], text, &self.content[range.end..]);
        self.marked = (!text.is_empty()).then(|| start..start + text.len());
        let base = self.to_utf16(start);
        let n = text.encode_utf16().count();
        self.selected = sel.map(|s| self.range_from_utf16(&(base + s.start.min(n)..base + s.end.min(n)))).unwrap_or(start + text.len()..start + text.len());
        self.reversed = false;
        w.notify_text_input_changed(&self.focus_handle);
        cx.emit(InputEvent::Change);
        cx.notify();
    }
    fn bounds_for_range(&mut self, r: Range<usize>, bounds: Bounds<Pixels>, _: &mut Window, _: &mut Context<Self>) -> Option<Bounds<Pixels>> {
        let r = self.range_from_utf16(&r);
        let lh = self.layout.as_ref()?.line_height;
        let a = self.point_for(r.start)?;
        let b = self.point_for(r.end)?;
        Some(Bounds::from_corners(point(bounds.left() + a.x, bounds.top() + a.y - self.scroll_y), point(bounds.left() + b.x.max(a.x + px(1.)), bounds.top() + b.y + lh - self.scroll_y)))
    }
    fn character_index_for_point(&mut self, p: Point<Pixels>, _: &mut Window, _: &mut Context<Self>) -> Option<usize> {
        let l = self.layout.as_ref()?;
        let local = point(p.x - l.bounds.left(), p.y - l.bounds.top() + self.scroll_y);
        Some(self.to_utf16(self.offset_for(local)))
    }
}

/// The field: `Input::new(&state)` with `.small()`, `.appearance(false)` (no box), `.disabled(bool)`,
/// `.text_size(px)`.
#[derive(IntoElement)]
pub struct Input {
    state: Entity<InputState>,
    small: bool,
    boxed: bool,
    disabled: bool,
    text_size: Option<Pixels>,
}

impl Input {
    pub fn new(state: &Entity<InputState>) -> Self {
        Input { state: state.clone(), small: false, boxed: true, disabled: false, text_size: None }
    }
    pub fn small(mut self) -> Self {
        self.small = true;
        self
    }
    pub fn appearance(mut self, boxed: bool) -> Self {
        self.boxed = boxed;
        self
    }
    pub fn disabled(mut self, d: bool) -> Self {
        self.disabled = d;
        self
    }
    pub fn text_size(mut self, s: Pixels) -> Self {
        self.text_size = Some(s);
        self
    }
}

impl RenderOnce for Input {
    fn render(self, _window: &mut Window, cx: &mut App) -> impl IntoElement {
        // the field is its own view (redrawn on its own, its input handler registered even inside cached views)
        self.state.update(cx, |s, cx| {
            let o = (self.small, self.boxed, self.disabled, self.text_size);
            if s.opts != o {
                s.opts = o;
                cx.notify();
            }
        });
        self.state
    }
}

impl Render for InputState {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let k = crate::theme::c(cx).clone();
        let (small, boxed, disabled, text_size) = self.opts;
        self.disabled = disabled;
        let is_focused = self.focus_handle.is_focused(window);
        if is_focused {
            self.want_focus = false;
        }
        if self.want_focus {
            // asked before this field existed: focus it once it is in the frame (until it holds the focus)
            let h = self.focus_handle.clone();
            window.defer(cx, move |w, cx| w.focus(&h, cx));
        }
        let focused = is_focused;
        let size = text_size.unwrap_or(if small { super::font_size(cx) - px(1.) } else { super::font_size(cx) });
        let focus = self.focus_handle.clone();
        let st = cx.entity();
        div()
            .key_context("Input")
            .track_focus(&focus)
            .cursor(CursorStyle::IBeam)
            .w_full()
            .text_size(size)
            .line_height(size * 1.45)
            .text_color(if disabled { k.muted } else { k.text })
            .when(boxed, |d| {
                d.px(px(if small { 8. } else { 10. }))
                    .py(px(if small { 3. } else { 6. }))
                    .rounded(px(6.))
                    .bg(k.bg)
                    .border_1()
                    .border_color(if focused { k.accent.opacity(0.7) } else { k.line2 })
            })
            .on_action(cx.listener(Self::backspace))
            .on_action(cx.listener(Self::delete))
            .on_action(cx.listener(Self::delete_word_left))
            .on_action(cx.listener(Self::delete_to_line_start))
            .on_action(cx.listener(Self::left))
            .on_action(cx.listener(Self::right))
            .on_action(cx.listener(Self::up))
            .on_action(cx.listener(Self::down))
            .on_action(cx.listener(Self::word_left_a))
            .on_action(cx.listener(Self::word_right_a))
            .on_action(cx.listener(Self::line_start_a))
            .on_action(cx.listener(Self::line_end_a))
            .on_action(cx.listener(Self::doc_start))
            .on_action(cx.listener(Self::doc_end))
            .on_action(cx.listener(Self::select_left))
            .on_action(cx.listener(Self::select_right))
            .on_action(cx.listener(Self::select_up))
            .on_action(cx.listener(Self::select_down))
            .on_action(cx.listener(Self::select_word_left))
            .on_action(cx.listener(Self::select_word_right))
            .on_action(cx.listener(Self::select_line_start))
            .on_action(cx.listener(Self::select_line_end))
            .on_action(cx.listener(Self::select_doc_start))
            .on_action(cx.listener(Self::select_doc_end))
            .on_action(cx.listener(Self::select_all))
            .on_action(cx.listener(Self::copy))
            .on_action(cx.listener(Self::cut))
            .on_action(cx.listener(Self::paste))
            .on_action(cx.listener(Self::undo_a))
            .on_action(cx.listener(Self::redo_a))
            .on_action(cx.listener(Self::enter))
            .on_action(cx.listener(Self::secondary_enter))
            .on_action(cx.listener(Self::character_palette))
            .on_mouse_down(MouseButton::Left, cx.listener(Self::mouse_down))
            .on_mouse_down(MouseButton::Right, cx.listener(Self::right_down))
            .on_mouse_move(cx.listener(Self::mouse_move))
            .on_mouse_up(MouseButton::Left, cx.listener(Self::mouse_up))
            .on_mouse_up_out(MouseButton::Left, cx.listener(Self::mouse_up))
            .on_scroll_wheel(cx.listener(Self::scroll))
            .children(self.paste_at.map(|at| self.paste_bubble(at, cx)))
            .child(TextArea { input: st, size, placeholder_color: k.faint, selection: k.selection, caret: k.accent })
    }
}

struct TextArea {
    input: Entity<InputState>,
    size: Pixels,
    placeholder_color: Hsla,
    selection: Hsla,
    caret: Hsla,
}

impl IntoElement for TextArea {
    type Element = Self;
    fn into_element(self) -> Self {
        self
    }
}

pub struct Prepaint {
    lines: Vec<(usize, WrappedLine)>,
    placeholder: bool,
    cursor: Option<PaintQuad>,
    selections: Vec<PaintQuad>,
    line_height: Pixels,
}

fn runs_for(text: &str, font: Font, color: Hsla, marked: Option<Range<usize>>) -> Vec<TextRun> {
    let base = TextRun { len: text.len(), font, color, background_color: None, underline: None, strikethrough: None };
    match marked {
        Some(m) if m.end <= text.len() => [
            TextRun { len: m.start, ..base.clone() },
            TextRun { len: m.end - m.start, underline: Some(UnderlineStyle { color: Some(color), thickness: px(1.), wavy: false }), ..base.clone() },
            TextRun { len: text.len() - m.end, ..base },
        ]
        .into_iter()
        .filter(|r| r.len > 0)
        .collect(),
        _ => vec![base],
    }
}

/// the text to shape: the display text, or the placeholder; one WrappedLine per paragraph, with its start offset
fn shape(window: &mut Window, text: &str, font: Font, size: Pixels, color: Hsla, marked: Option<Range<usize>>, wrap: Option<Pixels>) -> Vec<(usize, WrappedLine)> {
    let runs = runs_for(text, font, color, marked);
    let lines = window.text_system().shape_text(SharedString::from(text.to_string()), size, &runs, wrap, None).unwrap_or_default();
    let mut out = Vec::with_capacity(lines.len());
    let mut start = 0;
    for l in lines {
        let n = l.len();
        out.push((start, l));
        start += n + 1;
    }
    out
}

impl Element for TextArea {
    type RequestLayoutState = ();
    type PrepaintState = Prepaint;
    fn id(&self) -> Option<ElementId> {
        None
    }
    fn source_location(&self) -> Option<&'static core::panic::Location<'static>> {
        None
    }
    fn request_layout(&mut self, _: Option<&GlobalElementId>, _: Option<&InspectorElementId>, window: &mut Window, cx: &mut App) -> (LayoutId, ()) {
        let s = self.input.read(cx);
        let (min, max) = s.rows;
        let text = if s.content.is_empty() { s.placeholder.to_string() } else { s.display() };
        let multi = s.multi_line;
        let mut style = Style::default();
        style.size.width = relative(1.).into();
        let size = self.size;
        let font = window.text_style().font();
        let lh = window.line_height();
        let id = window.request_measured_layout(style, move |known, avail, window, _cx| {
            let width = known.width.or(match avail.width {
                AvailableSpace::Definite(w) => Some(w),
                _ => None,
            });
            let rows = if multi {
                let lines = shape(window, &text, font.clone(), size, black(), None, width);
                lines.iter().map(|(_, l)| l.wrap_boundaries().len() + 1).sum::<usize>().clamp(min, max)
            } else {
                1
            };
            gpui::size(width.unwrap_or(px(100.)), lh * rows as f32)
        });
        (id, ())
    }
    fn prepaint(&mut self, _: Option<&GlobalElementId>, _: Option<&InspectorElementId>, bounds: Bounds<Pixels>, _: &mut (), window: &mut Window, cx: &mut App) -> Prepaint {
        let style = window.text_style();
        let lh = window.line_height();
        let (lines, placeholder, cursor_o, selected, scroll) = {
            let s = self.input.read(cx);
            let placeholder = s.content.is_empty();
            let text = if placeholder { s.placeholder.to_string() } else { s.display() };
            let color = if placeholder { self.placeholder_color } else { style.color };
            let marked = s.marked.as_ref().map(|m| s.to_display(m.start)..s.to_display(m.end));
            let wrap = s.multi_line.then_some(bounds.size.width);
            let lines = shape(window, &text, style.font(), self.size, color, marked, wrap);
            (lines, placeholder, s.cursor(), s.selected.clone(), s.scroll_y)
        };
        // the same lines again for the state (hits, carets; shaping is cached for the frame)
        let again = {
            let s = self.input.read(cx);
            let text = if placeholder { s.placeholder.to_string() } else { s.display() };
            let marked = s.marked.as_ref().map(|m| s.to_display(m.start)..s.to_display(m.end));
            let wrap = s.multi_line.then_some(bounds.size.width);
            let color = if placeholder { self.placeholder_color } else { style.color };
            shape(window, &text, style.font(), self.size, color, marked, wrap)
        };
        self.input.update(cx, |s, _| {
            s.layout = Some(Laid { lines: again, bounds, line_height: lh, display: String::new() });
        });
        let (cursor_p, sel_rects, scroll) = {
            let s = self.input.read(cx);
            let cp = if placeholder { Some(point(px(0.), px(0.))) } else { s.point_for(cursor_o) };
            let mut scroll = scroll;
            if let Some(p) = cp {
                if p.y < scroll {
                    scroll = p.y;
                } else if p.y + lh > scroll + bounds.size.height {
                    scroll = p.y + lh - bounds.size.height;
                }
            }
            let mut rects = vec![];
            if !selected.is_empty() && !placeholder {
                if let (Some(a), Some(b)) = (s.point_for(selected.start), s.point_for(selected.end)) {
                    let w = bounds.size.width;
                    if (a.y - b.y).abs() < px(1.) {
                        rects.push(Bounds::from_corners(a, point(b.x, b.y + lh)));
                    } else {
                        rects.push(Bounds::from_corners(a, point(w, a.y + lh)));
                        if b.y - a.y > lh + px(0.5) {
                            rects.push(Bounds::from_corners(point(px(0.), a.y + lh), point(w, b.y)));
                        }
                        rects.push(Bounds::from_corners(point(px(0.), b.y), point(b.x, b.y + lh)));
                    }
                }
            }
            (cp, rects, scroll)
        };
        self.input.update(cx, |s, _| s.scroll_y = scroll.max(px(0.)));
        let origin = point(bounds.left(), bounds.top() - scroll);
        let cursor = if selected.is_empty() {
            cursor_p.map(|p| fill(Bounds::new(point(origin.x + p.x, origin.y + p.y), gpui::size(px(1.5), lh)), self.caret))
        } else {
            None
        };
        let selections = sel_rects.into_iter().map(|r| fill(Bounds::new(point(origin.x + r.origin.x, origin.y + r.origin.y), r.size), self.selection)).collect();
        Prepaint { lines, placeholder, cursor, selections, line_height: lh }
    }
    fn paint(&mut self, _: Option<&GlobalElementId>, _: Option<&InspectorElementId>, bounds: Bounds<Pixels>, _: &mut (), p: &mut Prepaint, window: &mut Window, cx: &mut App) {
        let focus = self.input.read(cx).focus_handle.clone();
        let scroll = self.input.read(cx).scroll_y;
        window.handle_input(&focus, ElementInputHandler::new(bounds, self.input.clone()), cx);
        // iOS: a long press in the field (the chat button's own long press is outside any field)
        let input = self.input.clone();
        window.on_mouse_event::<LongPressEvent>(move |e, phase, window, cx| {
            if phase != DispatchPhase::Bubble || !matches!(e.phase, TouchPhase::Started) || !bounds.contains(&e.start_position) {
                return;
            }
            cx.stop_propagation();
            input.update(cx, |s, cx| s.long_press(e.start_position, window, cx));
        });
        let _ = p.placeholder;
        window.with_content_mask(Some(ContentMask { bounds }), |window| {
            for q in p.selections.drain(..) {
                window.paint_quad(q);
            }
            let mut y = bounds.top() - scroll;
            for (_, line) in &p.lines {
                let _ = line.paint(point(bounds.left(), y), p.line_height, TextAlign::Left, None, window, cx);
                y += line.size(p.line_height).height;
            }
            if focus.is_focused(window) {
                if let Some(c) = p.cursor.take() {
                    window.paint_quad(c);
                }
            }
        });
    }
}

/// the clipboard's text (UIPasteboard on iOS, NSPasteboard on macOS); UI tests set OPTCHAT_TEST_CLIPBOARD instead of
/// touching the user's clipboard
fn clip_text(cx: &App) -> Option<String> {
    if let Ok(t) = std::env::var("OPTCHAT_TEST_CLIPBOARD") {
        return Some(t);
    }
    cx.read_from_clipboard().and_then(|i| i.text())
}
