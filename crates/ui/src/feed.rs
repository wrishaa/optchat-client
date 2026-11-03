//! The chat feed (app/ui/chat.tsx): a virtualized, variable-height list (GPUI `list`, bottom-aligned like a chat log)
//! of rows built from the task's log grouped by turn: user/parent bubbles, report cards, markdown replies, tool cards
//! (input/output, images, per-tool stop), live streaming text and thinking, and each turn's chips.
use crate::fmt;
use crate::markdown::{self, Doc};
use crate::state::{task_name, AppState, ToolState};
use crate::theme::{c, C};
use crate::ui::zx;
use crate::ui::*;
use gpui::prelude::FluentBuilder;
use gpui::*;
use gpui::Context;
use crate::kit::menu::{ContextMenuExt, PopupMenuItem};
use optchat_proto::types::*;
use std::collections::{HashMap, HashSet};
use std::sync::Arc;

#[derive(Clone)]
pub enum Row {
    /// "Load earlier messages (N more)": the feed shows the last PAGE entries until asked for more
    LoadEarlier(usize),
    Divider,
    Empty,
    Skeleton,
    User(Arc<LogEntry>),
    Parent(Arc<LogEntry>),
    Report(Arc<LogEntry>),
    Work(Arc<LogEntry>),
    Talk(Arc<LogEntry>),
    Image(Arc<LogEntry>),
    Tool { e: Arc<LogEntry>, echo: Option<Arc<LogEntry>>, images: Vec<Arc<LogEntry>> },
    Live { turn_id: String },
    Foot { turn_id: String },
}

pub enum FeedEvent {
    SelectRequest(String),
    OpenTask(i64),
    Lightbox(String),
}
impl EventEmitter<FeedEvent> for Feed {}

pub struct Feed {
    pub state: Entity<AppState>,
    pub list: ListState,
    rows: Vec<Row>,
    keys: Vec<(String, u64)>,
    rev: u64,
    task: i64,
    seq: u64,
    md: HashMap<i64, Doc>,
    live_md: HashMap<String, (usize, Doc)>,
    open: HashSet<String>,
    full: HashSet<i64>,
    pub images: HashMap<String, Option<Arc<Image>>>,
    _ticker: Option<Task<()>>,
    _subs: Vec<Subscription>,
    /// first log index shown (paging); None until the task's log is in
    from: Option<usize>,
    loading_more: bool,
    fetching_older: bool,
    /// the user scrolled away from the bottom: new text no longer pulls the view down (follow mode off)
    pub scrolled_up: bool,
    /// the phone / narrow layout: one quiet line per turn foot and tool call (tap to expand), code wraps
    pub compact: bool,
    /// UI tests: the most live text / thinking seen in this task's running turn (chars)
    live_max: (usize, usize),
}

/// entries shown when a task opens, and added per "load earlier" (like the web app's snapshot of 120)
pub const PAGE: usize = 120;
pub const MORE: usize = 200;

/// move `from` back to the start of its turn group, so a turn is never split
fn group_start(log: &[Arc<LogEntry>], mut from: usize) -> usize {
    while from > 0 && from < log.len() && log[from].turn_id.is_some() && log[from - 1].turn_id == log[from].turn_id {
        from -= 1;
    }
    from
}

const WORK_RE: &str = r"^\[[ac]\d+\] ";

impl Feed {
    pub fn new(state: Entity<AppState>, cx: &mut Context<Self>) -> Feed {
        let list = ListState::new(0, ListAlignment::Bottom, px(800.));
        let sub = cx.observe(&state, |this: &mut Feed, _, cx| this.sync(cx));
        let mut f = Feed {
            state,
            list,
            rows: vec![],
            keys: vec![],
            rev: u64::MAX,
            task: -1,
            seq: u64::MAX,
            md: HashMap::new(),
            live_md: HashMap::new(),
            open: HashSet::new(),
            full: HashSet::new(),
            images: HashMap::new(),
            _ticker: None,
            _subs: vec![sub],
            from: None,
            loading_more: false,
            fetching_older: false,
            scrolled_up: false,
            compact: false,
            live_max: (0, 0),
        };
        f.sync(cx);
        let me = cx.entity().downgrade();
        f.list.set_scroll_handler(move |ev: &ListScrollEvent, _, cx| {
            let up = ev.is_scrolled;
            let _ = me.update(cx, |f, cx| {
                if f.scrolled_up != up {
                    f.scrolled_up = up;
                    cx.notify();
                }
            });
        });
        // a 1 s tick while something runs: elapsed timers
        f._ticker = Some(cx.spawn(async move |this, cx| loop {
            Timer::after(std::time::Duration::from_millis(1000)).await;
            let alive = this.update(cx, |f, cx| {
                let running = f.state.read(cx).cur.running.is_some() || f.state.read(cx).cur.tools.values().any(|t| t.running);
                if running {
                    cx.notify();
                }
            });
            if alive.is_err() {
                break;
            }
        }));
        f
    }

    /// Test hook: a continuous scroll at 120 Hz (`px` per step, direction flips every second) through the same
    /// list scrolling, layout and paint path as a trackpad gesture (synthetic OS events need permissions we don't take).
    pub fn test_gesture(&mut self, secs: f32, px_step: f32, cx: &mut Context<Self>) {
        cx.spawn(async move |this, cx| {
            let t0 = std::time::Instant::now();
            let mut flip = t0;
            let mut dir = -1.0f32;
            while t0.elapsed().as_secs_f32() < secs {
                Timer::after(std::time::Duration::from_micros(8333)).await;
                if flip.elapsed().as_secs_f32() >= 1.0 {
                    dir = -dir;
                    flip = std::time::Instant::now();
                }
                let ok = this.update(cx, |f, cx| {
                    frames().lock().last_scroll = Some(std::time::Instant::now());
                    f.list.scroll_by(px(px_step * dir));
                    cx.notify();
                });
                if ok.is_err() {
                    break;
                }
            }
        })
        .detach();
    }

    /// the chat text zoom changed: every row's height changes, so measure them all again
    pub fn relayout(&mut self, cx: &mut Context<Self>) {
        self.list.reset(self.rows.len());
        cx.notify();
    }

    fn toggle(&mut self, key: &str, cx: &mut Context<Self>) {
        if !self.open.remove(key) {
            self.open.insert(key.to_string());
        }
        self.rev = u64::MAX; // force a rebuild of versions
        self.sync(cx);
    }

    pub fn scroll_to_bottom(&mut self) {
        let n = self.rows.len();
        if n > 0 {
            self.list.scroll_to_reveal_item(n - 1);
        }
    }

    /// Rebuild the rows when the state's feed revision moved; splice only from the first changed row.
    fn sync(&mut self, cx: &mut Context<Self>) {
        let s = self.state.read(cx);
        if s.feed_rev == self.rev && s.open_seq == self.seq {
            return;
        }
        let switched = s.open_seq != self.seq || s.current != self.task;
        self.rev = s.feed_rev;
        self.seq = s.open_seq;
        if switched {
            self.task = s.current;
            self.md.clear();
            self.live_md.clear();
            self.open.clear();
            self.full.clear();
            self.from = None;
            self.live_max = (0, 0);
            for p in ["tool.", "img.", "foot.", "feed.more", "feed.jump", "rep.", "work.", "cut.", "toolimg."] {
                crate::ui::clear_probes(p);
            }
        }
        if self.from.is_none() && s.cur.loaded {
            self.from = Some(group_start(&s.cur.log, s.cur.log.len().saturating_sub(PAGE)));
        }
        let rows = build_rows(s, self.from.unwrap_or(0));
        if crate::ui::probes_on() {
            // UI tests: skeletons drawn while a task loads; which empty state (never the root's on a real chat)
            match rows.first() {
                Some(Row::Skeleton) => crate::ui::bump("feedSkeletons"),
                Some(Row::Empty) => crate::ui::seen("feedEmpty", if s.tasks.get(&s.current).map(|t| t.task.parent.is_none()).unwrap_or(true) { "root" } else { "child" }),
                _ => {}
            }
        }
        let keys: Vec<(String, u64)> = rows.iter().map(|r| row_key(r, s, &self.open)).collect();
        if switched {
            self.list.reset(rows.len());
        } else {
            let k = self.keys.iter().zip(keys.iter()).position(|(a, b)| a != b).unwrap_or(self.keys.len().min(keys.len()));
            let old = self.keys.len();
            if k < old || k < keys.len() {
                self.list.splice(k..old, keys.len() - k);
            }
            // the user's own new message: follow again (like sending in the web app)
            let sent = keys.len() > old && rows[old..].iter().any(|r| matches!(r, Row::User(_) | Row::Image(_)));
            if sent && self.scrolled_up {
                self.rows = rows;
                self.keys = keys;
                self.jump_to_bottom(cx);
                return;
            }
        }
        self.rows = rows;
        self.keys = keys;
        cx.notify();
    }

    fn doc(&mut self, e: &LogEntry) -> Doc {
        if let Some(d) = self.md.get(&e.i) {
            return d.clone();
        }
        let d = markdown::parse(&e.text);
        self.md.insert(e.i, d.clone());
        d
    }

    fn image(&mut self, blob: &str, cx: &mut Context<Self>) -> Option<Arc<Image>> {
        if let Some(v) = self.images.get(blob) {
            return v.clone();
        }
        self.images.insert(blob.to_string(), None);
        let blob = blob.to_string();
        let b2 = blob.clone();
        let st = self.state.clone();
        let (tx, rx) = tokio::sync::oneshot::channel();
        if let Some(h) = st.read(cx).hub() {
            st.read(cx).rt.spawn(async move {
                let r = h.get_blob(&b2).await;
                let _ = tx.send(r);
            });
        }
        cx.spawn(async move |this, cx| {
            if let Ok(Some(b)) = rx.await {
                let fmt = match b.media_type.as_str() {
                    "image/png" => ImageFormat::Png,
                    "image/jpeg" => ImageFormat::Jpeg,
                    "image/gif" => ImageFormat::Gif,
                    "image/webp" => ImageFormat::Webp,
                    _ => ImageFormat::Png,
                };
                let img = Arc::new(Image::from_bytes(fmt, b.bytes));
                let _ = this.update(cx, |f, cx| {
                    f.images.insert(blob.clone(), Some(img));
                    // rows holding this image change height once it loads
                    let idx: Vec<usize> = f
                        .rows
                        .iter()
                        .enumerate()
                        .filter(|(_, r)| row_blobs(r).iter().any(|x| *x == blob))
                        .map(|(i, _)| i)
                        .collect();
                    for i in idx {
                        f.list.splice(i..i + 1, 1);
                    }
                    cx.notify();
                });
            }
        })
        .detach();
        None
    }
}

fn row_blobs(r: &Row) -> Vec<String> {
    match r {
        Row::User(e) | Row::Parent(e) | Row::Report(e) | Row::Image(e) => e.image.iter().map(|i| i.blob.clone()).collect(),
        Row::Tool { images, .. } => images.iter().filter_map(|e| e.image.as_ref().map(|i| i.blob.clone())).collect(),
        _ => vec![],
    }
}

fn row_key(r: &Row, s: &AppState, open: &HashSet<String>) -> (String, u64) {
    let o = |k: &str| if open.contains(k) { 1u64 << 40 } else { 0 };
    match r {
        Row::LoadEarlier(n) => ("more".into(), *n as u64),
        Row::Divider => ("div".into(), 0),
        Row::Empty => ("empty".into(), s.tasks.get(&s.current).map(|t| t.task.status as u64).unwrap_or(0)),
        Row::Skeleton => ("skel".into(), 0),
        Row::User(e) | Row::Parent(e) | Row::Work(e) | Row::Talk(e) | Row::Image(e) => (format!("e{}", e.i), e.text.len() as u64 + o(&format!("e{}", e.i))),
        Row::Report(e) => (format!("r{}", e.i), e.text.len() as u64 + o(&format!("r{}", e.i))),
        Row::Tool { e, echo, images } => {
            let k = format!("t{}", e.i);
            let st = e.meta_str("callId").and_then(|c| s.cur.tools.get(c)).map(|t| if t.running { 1 } else { 2 }).unwrap_or(0);
            (k.clone(), echo.as_ref().map(|x| x.i as u64 + 1).unwrap_or(0) * 8 + st + images.len() as u64 * 1000 + o(&k))
        }
        Row::Live { turn_id } => {
            let n: usize = s.cur.live.values().filter(|l| &l.turn_id == turn_id).map(|l| l.text.len() * 3 + l.thinking.len()).sum();
            let st = s.cur.turns.get(turn_id).map(|t| t.status.len()).unwrap_or(0);
            let tools = s.cur.tools.values().filter(|t| &t.turn_id == turn_id && t.running).count();
            let k = format!("live{turn_id}");
            (k.clone(), (n as u64) << 4 | st as u64 | (tools as u64) << 32 | o(&k))
        }
        Row::Foot { turn_id } => {
            let t = s.cur.turns.get(turn_id);
            let reqs = s.cur.requests.values().filter(|r| r.turn_id.as_deref() == Some(turn_id)).count();
            let k = format!("foot{turn_id}");
            let v = t.map(|t| (t.cost.total * 1e6) as u64 ^ (t.status.len() as u64) << 50 ^ t.usage.input as u64).unwrap_or(0) ^ (reqs as u64) << 20 ^ t.and_then(|t| t.error.as_ref()).map(|e| e.len() as u64).unwrap_or(0);
            (k.clone(), v ^ o(&k))
        }
    }
}

pub fn build_rows(s: &AppState, from: usize) -> Vec<Row> {
    if !s.cur.loaded {
        return vec![Row::Skeleton];
    }
    let log = &s.cur.log;
    if log.is_empty() && s.cur.running.is_none() {
        return vec![Row::Empty];
    }
    let unrun = s.tasks.get(&s.current).and_then(|t| t.task.unrun);
    let work = regex_lite(WORK_RE);
    let running_id = s.cur.running.as_ref().map(|r| r.id.clone());
    let mut rows = vec![];
    let from = from.min(log.len());
    // older entries: those loaded but not shown yet, plus those still on the hub (the snapshot carries the last 120)
    let on_hub = log.first().map(|e| e.i.max(0) as usize).unwrap_or(0);
    if from > 0 || on_hub > 0 {
        rows.push(Row::LoadEarlier(from + on_hub));
    }
    // groups by turnId
    let mut k = from;
    let mut seen_running = false;
    while k < log.len() {
        let tid = log[k].turn_id.clone();
        let mut end = k + 1;
        if tid.is_some() {
            while end < log.len() && log[end].turn_id == tid {
                end += 1;
            }
        }
        let g = &log[k..end];
        if unrun == Some(g[0].i) {
            rows.push(Row::Divider);
        }
        // tool echoes by call id
        let mut echoes: HashMap<String, Arc<LogEntry>> = HashMap::new();
        let mut echo_imgs: HashMap<String, Vec<Arc<LogEntry>>> = HashMap::new();
        for e in g {
            if e.kind != LogKind::Echo {
                continue;
            }
            if let Some(cid) = e.meta_str("callId") {
                if e.image.is_some() {
                    echo_imgs.entry(cid.into()).or_default().push(e.clone());
                } else {
                    echoes.entry(cid.into()).or_insert_with(|| e.clone());
                }
            }
        }
        for e in g {
            match e.kind {
                LogKind::User if work(&e.text) => rows.push(Row::Work(e.clone())),
                LogKind::User | LogKind::Note => rows.push(if e.image.is_some() { Row::Image(e.clone()) } else { Row::User(e.clone()) }),
                LogKind::Parent => rows.push(Row::Parent(e.clone())),
                LogKind::Task => rows.push(Row::Report(e.clone())),
                LogKind::Talk => rows.push(Row::Talk(e.clone())),
                LogKind::Tool => {
                    let cid = e.meta_str("callId").unwrap_or("").to_string();
                    rows.push(Row::Tool { e: e.clone(), echo: echoes.get(&cid).cloned(), images: echo_imgs.get(&cid).cloned().unwrap_or_default() });
                }
                LogKind::Echo => {
                    let has_tool = e.meta_str("callId").map(|c| g.iter().any(|x| x.kind == LogKind::Tool && x.meta_str("callId") == Some(c))).unwrap_or(false);
                    if !has_tool {
                        rows.push(if e.image.is_some() { Row::Image(e.clone()) } else { Row::Tool { e: e.clone(), echo: Some(e.clone()), images: vec![] } });
                    }
                }
            }
        }
        if let Some(t) = &tid {
            if Some(t) == running_id.as_ref() {
                rows.push(Row::Live { turn_id: t.clone() });
                seen_running = true;
            }
            if s.cur.turns.contains_key(t) {
                rows.push(Row::Foot { turn_id: t.clone() });
            }
        }
        k = end;
    }
    if let (Some(r), false) = (&running_id, seen_running) {
        rows.push(Row::Live { turn_id: r.clone() });
        rows.push(Row::Foot { turn_id: r.clone() });
    }
    rows
}

fn regex_lite(_: &str) -> impl Fn(&str) -> bool {
    // ^\[[ac]\d+\]
    |t: &str| {
        let b = t.as_bytes();
        if b.len() < 4 || b[0] != b'[' || !(b[1] == b'a' || b[1] == b'c') {
            return false;
        }
        let mut i = 2;
        while i < b.len() && b[i].is_ascii_digit() {
            i += 1;
        }
        i > 2 && i + 1 < b.len() && b[i] == b']' && b[i + 1] == b' '
    }
}

impl Render for Feed {
    fn render(&mut self, _w: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let _t = crate::ui::rtime::guard("feed");
        crate::ui::anim::area(crate::ui::anim::FEED);
        // row probes are re-recorded by the rows rendered in this frame: drop the ones scrolled out of range
        if crate::ui::probes_on() {
            let mut open: Vec<&String> = self.open.iter().collect();
            open.sort();
            let mut full: Vec<&i64> = self.full.iter().collect();
            full.sort();
            let top = self.list.logical_scroll_top();
            crate::ui::set_state_extra("feed", serde_json::json!({ "open": open, "from": self.from.and_then(|k| self.state.read(cx).cur.log.get(k).map(|e| e.i)), "rows": self.rows.len(), "scrolledUp": self.scrolled_up, "full": full, "top": [top.item_ix, f32::from(top.offset_in_item)], "liveText": self.live_max.0, "liveNow": self.state.read(cx).cur.live.values().map(|l| l.text.len()).sum::<usize>(), "liveThinking": self.live_max.1, "task": self.task }));
            for p in ["tool.", "img.", "foot.", "feed.more", "feed.jump", "feed.last", "rep.", "work.", "cut.", "toolimg."] {
                crate::ui::clear_probes(p);
            }
        }
        let this = cx.entity().downgrade();
        let k = c(cx).clone();
        div()
            .id("feedwrap")
            .size_full()
            .text_size(zx(13.5))
            .bg(k.bg)
            .on_scroll_wheel(|_, _, _| {
                frames().lock().last_scroll = Some(std::time::Instant::now());
            })
            .child(
                list(self.list.clone(), move |ix, window, cx| {
                    let Some(feed) = this.upgrade() else { return div().into_any_element() };
                    let t0 = std::time::Instant::now();
                    crate::ui::anim::area(crate::ui::anim::FEED);
                    let el = feed.update(cx, |f, cx| f.render_row(ix, window, cx));
                    let mut fr = frames().lock();
                    if fr.last_scroll.map(|t| t.elapsed().as_millis() < 120).unwrap_or(false) {
                        fr.render_ms.push(t0.elapsed().as_secs_f32() * 1000.0);
                    }
                    el
                })
                .size_full(),
            )
            .when(self.scrolled_up, |d| {
                let k2 = k.clone();
                d.relative().child(
                    div().absolute().bottom(px(14.)).right(px(22.)).child(
                        div()
                            .id("jump-bottom")
                            .size(px(34.))
                            .rounded_full()
                            .flex()
                            .items_center()
                            .justify_center()
                            .bg(k2.panel2)
                            .border_1()
                            .border_color(k2.line2)
                            .shadow_md()
                            .cursor_pointer()
                            .hover(move |s| s.bg(k2.hover))
                            .child(icon("arrow-down", 16., k.text2))
                            .children(probe("feed.jump"))
                            .on_click(cx.listener(|f, _, _, cx| f.jump_to_bottom(cx))),
                    ),
                )
            })
    }
}

const MAXW: f32 = 860.0;

impl Feed {
    fn wrap(&self, el: impl IntoElement) -> AnyElement {
        div().w_full().flex().justify_center().px(zx(if self.compact { 16. } else { 26. })).child(div().w_full().max_w(zx(MAXW)).child(el)).into_any_element()
    }

    fn render_row(&mut self, ix: usize, window: &mut Window, cx: &mut Context<Self>) -> AnyElement {
        let _t = crate::ui::rtime::guard("feedRow");
        let Some(row) = self.rows.get(ix).cloned() else { return div().into_any_element() };
        crate::markdown::set_compact(self.compact);
        let first = ix == 0;
        let last = ix + 1 == self.rows.len();
        let el = match row {
            Row::LoadEarlier(n) => self.load_earlier_row(n, cx),
            Row::Skeleton => self.skeleton(cx),
            Row::Empty => self.empty(cx),
            Row::Divider => self.divider(cx),
            Row::User(e) => self.user(&e, window, cx),
            Row::Image(e) => self.image_msg(&e, cx),
            Row::Parent(e) => self.parent(&e, window, cx),
            Row::Report(e) => self.report(&e, window, cx),
            Row::Work(e) => self.work(&e, window, cx),
            Row::Talk(e) => self.talk(&e, window, cx),
            Row::Tool { e, echo, images } => self.tool(&e, echo.as_ref(), &images, cx),
            Row::Live { turn_id } => self.live(&turn_id, window, cx),
            Row::Foot { turn_id } => self.foot(&turn_id, cx),
        };
        // keyed by the row: element state inside (context menus all use the id "context-menu") stays per row
        let key = self.keys.get(ix).map(|k| k.0.clone()).unwrap_or_else(|| format!("row{ix}"));
        let mut d = div().id(ElementId::Name(key.into())).w_full().child(self.wrap(el));
        if first {
            d = d.pt(zx(26.));
        }
        if last {
            d = d.pb(zx(30.)).children(probe("feed.last"));
        }
        d.into_any_element()
    }

    fn skeleton(&self, cx: &App) -> AnyElement {
        let k = c(cx);
        let mut d = div().flex().flex_col().gap(zx(24.));
        for w in [260.0, 180.0, 320.0] {
            d = d.child(
                div()
                    .flex()
                    .flex_col()
                    .child(div().flex().justify_end().mb(zx(14.)).child(div().w(zx(w)).h(zx(40.)).rounded(zx(14.)).bg(k.panel2)))
                    .child(skeleton_line(None, cx))
                    .child(skeleton_line(None, cx))
                    .child(skeleton_line(Some(220.), cx)),
            );
        }
        d.into_any_element()
    }

    fn empty(&self, cx: &App) -> AnyElement {
        let k = c(cx);
        let s = self.state.read(cx);
        let t = s.tasks.get(&s.current).map(|x| x.task.clone());
        match t {
            Some(t) if t.parent.is_some() => div()
                .mt(zx(120.))
                .flex()
                .flex_col()
                .items_center()
                .gap(zx(8.))
                .text_color(k.muted)
                .child(div().text_size(zx(24.)).text_color(k.text).font_weight(FontWeight::SEMIBOLD).child(t.title.clone()))
                .child({
                    let msg = format!("This task has no messages yet.{}", if t.status == TaskStatus::Active { " Its first turn is starting." } else { "" });
                    crate::ui::set_state_extra("feedEmptyText", serde_json::json!(msg));
                    msg
                })
                .children(probe("feed.empty"))
                .into_any_element(),
            _ => div()
                .mt(zx(110.))
                .flex()
                .flex_col()
                .items_center()
                .gap(zx(10.))
                .text_color(k.muted)
                .max_w(zx(520.))
                .mx_auto()
                .text_center()
                .child(logo(44.0, cx))
                .child(div().text_size(zx(26.)).text_color(k.text).font_weight(FontWeight::SEMIBOLD).child("One chat. It never ends."))
                .child("Every message is kept forever and folded into a tree of summaries. Each turn starts fresh with a ~128 KB view of the whole history, and the agent zooms in when it needs detail. Work can branch into tasks: each one a chat of its own.")
                .child(div().text_size(zx(12.)).child("Running on your local Claude Code and Codex logins."))
                .children(probe("feed.empty"))
                .into_any_element(),
        }
    }

    fn load_earlier_row(&mut self, n: usize, cx: &mut Context<Self>) -> AnyElement {
        let k = c(cx).clone();
        let hover = k.hover;
        // reaching the top loads more on its own (infinite load); the button is there for keyboard / clarity
        if !self.loading_more {
            self.loading_more = true;
            cx.spawn(async move |this, cx| {
                Timer::after(std::time::Duration::from_millis(16)).await;
                let _ = this.update(cx, |f, cx| f.load_earlier(cx));
            })
            .detach();
        }
        div()
            .flex()
            .justify_center()
            .py(zx(10.))
            .child(
                div()
                    .id("load-earlier")
                    .px(zx(12.))
                    .py(zx(5.))
                    .rounded(zx(8.))
                    .border_1()
                    .border_color(k.line2)
                    .text_size(zx(12.))
                    .text_color(k.text2)
                    .cursor_pointer()
                    .hover(move |s| s.bg(hover))
                    .child(format!("Load earlier messages ({n} more)"))
                    .children(probe("feed.more"))
                    .on_click(cx.listener(|f, _, _, cx| f.load_earlier(cx))),
            )
            .into_any_element()
    }

    /// Follow again: back to the bottom, pinned there while new text streams in.
    pub fn jump_to_bottom(&mut self, cx: &mut Context<Self>) {
        // only a reset puts a bottom-aligned list back into its pinned (None) state
        self.list.reset(self.rows.len());
        self.scrolled_up = false;
        cx.notify();
    }

    /// Test hook: the whole history at once (as if "Load earlier messages" was used until the start).
    pub fn load_all(&mut self, cx: &mut Context<Self>) {
        self.load_all_then(cx, |_, _| {});
    }
    /// Test hook: the whole history (fetched from the hub when it isn't loaded yet), then `then`
    pub fn load_all_then(&mut self, cx: &mut Context<Self>, then: impl FnOnce(&mut Feed, &mut Context<Feed>) + 'static) {
        let (task, first) = {
            let s = self.state.read(cx);
            (s.current, s.cur.log.first().map(|e| e.i).unwrap_or(0))
        };
        if first > 0 {
            let state = self.state.clone();
            crate::panels::fetch(&state, cx, move |h| async move { h.log_range(task, 0, first, false).await }, move |f: &mut Feed, r, cx| {
                if let Ok(entries) = r {
                    f.state.update(cx, |s, _| s.prepend_older(task, entries));
                }
                f.show_all(cx);
                then(f, cx);
            });
            return;
        }
        self.show_all(cx);
        then(self, cx);
    }
    fn show_all(&mut self, cx: &mut Context<Self>) {
        let s = self.state.read(cx);
        self.from = Some(0);
        let rows = build_rows(s, 0);
        self.keys = rows.iter().map(|r| row_key(r, s, &self.open)).collect();
        self.rows = rows;
        self.list.reset(self.rows.len());
        cx.notify();
    }

    /// Test hook: scroll the row of log entry `i` into view (the whole history is loaded first)
    pub fn reveal_entry(&mut self, i: i64, cx: &mut Context<Self>) {
        let all = self.from == Some(0) && self.state.read(cx).cur.log.first().map(|e| e.i == 0).unwrap_or(true);
        if !all {
            self.load_all_then(cx, move |f, cx| f.reveal_entry(i, cx));
            return;
        }
        let ix = self.rows.iter().position(|r| match r {
            Row::User(e) | Row::Parent(e) | Row::Report(e) | Row::Work(e) | Row::Talk(e) | Row::Image(e) => e.i == i,
            Row::Tool { e, echo, .. } => e.i == i || echo.as_ref().map(|x| x.i) == Some(i),
            _ => false,
        });
        if let Some(ix) = ix {
            self.list.scroll_to(ListOffset { item_ix: ix, offset_in_item: px(0.) });
            cx.notify();
        }
    }

    /// Show MORE earlier entries, keeping the rows on screen where they are.
    pub fn load_earlier(&mut self, cx: &mut Context<Self>) {
        self.loading_more = false;
        let Some(from) = self.from else { return };
        if from == 0 {
            // everything loaded is shown: ask the hub for the 300 before (rpc log.range, the page's loadOlder)
            let (task, first) = {
                let s = self.state.read(cx);
                (s.current, s.cur.log.first().map(|e| e.i).unwrap_or(0))
            };
            if first <= 0 || self.fetching_older {
                return;
            }
            self.fetching_older = true;
            let state = self.state.clone();
            crate::panels::fetch(&state, cx, move |h| async move { h.log_range(task, (first - 300).max(0), first, false).await }, move |f: &mut Feed, r, cx| {
                f.fetching_older = false;
                let Ok(entries) = r else { return };
                let added = f.state.update(cx, |s, _| s.prepend_older(task, entries));
                if added > 0 {
                    // the rows on screen keep their place: they moved down by `added` entries
                    f.from = Some(added);
                    f.load_earlier(cx);
                }
            });
            return;
        }
        let top = self.list.logical_scroll_top();
        let old = self.rows.len();
        let s = self.state.read(cx);
        self.from = Some(group_start(&s.cur.log, from.saturating_sub(MORE)));
        let rows = build_rows(s, self.from.unwrap());
        let keys: Vec<(String, u64)> = rows.iter().map(|r| row_key(r, s, &self.open)).collect();
        // old rows: [more, r1..rn]; new rows: [more?, earlier..., r1..rn]. Replace the old "more" row so r1..rn
        // keep their measured heights and only the earlier rows get measured.
        let added = rows.len() as isize - old as isize;
        self.list.splice(0..1, rows.len() + 1 - old);
        self.rows = rows;
        self.keys = keys;
        self.list.scroll_to(ListOffset { item_ix: (top.item_ix as isize + added).max(0) as usize, offset_in_item: top.offset_in_item });
        cx.notify();
    }

    fn divider(&self, cx: &App) -> AnyElement {
        let k = c(cx);
        div()
            .flex()
            .items_center()
            .gap(zx(10.))
            .my(zx(12.))
            .text_size(zx(11.))
            .text_color(k.faint)
            .child(div().flex_1().h(zx(1.)).bg(k.line2))
            .child("NOT RUN YET")
            .child(div().flex_1().h(zx(1.)).bg(k.line2))
            .into_any_element()
    }

    fn cut_text<'a>(&self, e: &'a LogEntry) -> (std::borrow::Cow<'a, str>, Option<usize>) {
        // the hub's slim copy (snapshot / log.range): already shortened, the full text on request
        if let Some(n) = e.cut {
            return (std::borrow::Cow::Borrowed(&e.text), Some(n as usize));
        }
        // long texts that arrived whole (live events) are shown shortened too, until "show all"
        let max = if e.kind == LogKind::Tool || e.kind == LogKind::Echo { 2000 } else { 8000 };
        if self.full.contains(&e.i) || e.text.chars().count() <= max {
            return (std::borrow::Cow::Borrowed(&e.text), None);
        }
        let t = if e.kind == LogKind::Tool { optchat_proto::trim::trim_tool_text(&e.text, max).0 } else { optchat_proto::trim::head_tail(&e.text, max) };
        (std::borrow::Cow::Owned(t), Some(js_len(&e.text)))
    }
    fn cut_button(&self, e: &LogEntry, n: Option<usize>, cx: &mut Context<Self>) -> Option<AnyElement> {
        let n = n?;
        let i = e.i;
        Some(
            btn_small(ElementId::Name(format!("cut{i}").into()), format!("shortened · show all {} chars", group_thousands(n)), Tone::Ghost, cx)
                .mt(zx(4.))
                .children(probe(format!("cut.{i}")))
                .on_click(cx.listener(move |f, _, _, cx| {
                    let slim = f.state.read(cx).cur.log.iter().any(|x| x.i == i && x.cut.is_some());
                    if slim {
                        // the full entry from the hub (rpc log.range full), replacing the slim one (the page's fullEntry)
                        let task = f.state.read(cx).current;
                        let state = f.state.clone();
                        crate::panels::fetch(&state, cx, move |h| async move { h.log_range(task, i, i + 1, true).await }, move |f: &mut Feed, r, cx| {
                            if let Ok(mut v) = r {
                                if let Some(e) = v.pop() {
                                    f.state.update(cx, |s, cx| {
                                        s.replace_entry(task, e);
                                        cx.notify();
                                    });
                                }
                            }
                        });
                    }
                    f.full.insert(i);
                    f.md.remove(&i);
                    f.rev = u64::MAX;
                    if let Some(ix) = f.rows.iter().position(|r| matches!(r, Row::Talk(e) | Row::User(e) | Row::Parent(e) | Row::Report(e) if e.i == i) || matches!(r, Row::Tool { e, echo, .. } if e.i == i || echo.as_ref().map(|x| x.i) == Some(i))) {
                        f.list.splice(ix..ix + 1, 1);
                    }
                    cx.notify();
                }))
                .into_any_element(),
        )
    }

    fn meta_line(&self, e: &LogEntry, cx: &App) -> Div {
        let k = c(cx);
        div().flex().justify_end().gap(zx(8.)).mt(zx(4.)).text_size(zx(10.5)).font_family(crate::ui::MONO).text_color(k.faint).child(format!("#{}", e.i)).child(fmt::time(&e.date))
    }

    fn user(&mut self, e: &Arc<LogEntry>, _w: &mut Window, cx: &mut Context<Self>) -> AnyElement {
        let k = c(cx).clone();
        let (text, n) = self.cut_text(e);
        let copy = e.text.clone();
        let text = text.to_string();
        div()
            .flex()
            .justify_end()
            .my(zx(10.))
            .child(
                div()
                    .max_w(relative(0.78))
                    .flex()
                    .flex_col()
                    .items_end()
                    .child(
                        div()
                            .id(ElementId::Name(format!("ub{}", e.i).into()))
                            .bg(k.panel2)
                            .border_1()
                            .border_color(k.line2)
                            .rounded(zx(16.))
                            .rounded_br(zx(4.))
                            .px(zx(14.))
                            .py(zx(10.))
                            .text_size(zx(14.))
                            .line_height(zx(21.5))
                            .text_color(k.text)
                            .child(text)
                            .context_menu(move |m, _, _| m.item(PopupMenuItem::new("Copy text").on_click({ let c2 = copy.clone(); move |_, _, cx| cx.write_to_clipboard(ClipboardItem::new_string(c2.clone())) }))),
                    )
                    .children(self.cut_button(e, n, cx))
                    .child(self.meta_line(e, cx)),
            )
            .into_any_element()
    }

    fn blob_img(&mut self, r: &ImageRef, max_w: f32, max_h: f32, cx: &mut Context<Self>) -> AnyElement {
        let k = c(cx).clone();
        let blob = r.blob.clone();
        let (w, h) = (r.w.unwrap_or(400) as f32, r.h.unwrap_or(300) as f32);
        let scale = (max_w / w).min(max_h / h).min(1.0);
        let (dw, dh) = (w * scale, h * scale);
        let img_el: AnyElement = match self.image(&blob, cx) {
            Some(i) => img(i).w(zx(dw)).h(zx(dh)).rounded(zx(12.)).into_any_element(),
            None => div().w(zx(dw)).h(zx(dh)).rounded(zx(12.)).bg(k.panel2).into_any_element(),
        };
        div()
            .id(ElementId::Name(format!("img{blob}{max_w}").into()))
            .rounded(zx(12.))
            .border_1()
            .border_color(k.line2)
            .overflow_hidden()
            .cursor_pointer()
            .child(img_el)
            .children(probe(format!("img.{}", &blob[..12.min(blob.len())])))
            .on_click(cx.listener(move |_, _, _, cx| cx.emit(FeedEvent::Lightbox(blob.clone()))))
            .into_any_element()
    }

    fn image_msg(&mut self, e: &Arc<LogEntry>, cx: &mut Context<Self>) -> AnyElement {
        let user = e.kind == LogKind::User;
        let im = self.blob_img(e.image.as_ref().unwrap(), 420., 360., cx);
        let k = c(cx);
        let meta = div().mt(zx(4.)).text_size(zx(10.5)).font_family(crate::ui::MONO).text_color(k.faint).child(format!("#{} {}", e.i, e.text));
        if user {
            div().flex().justify_end().my(zx(10.)).child(div().flex().flex_col().items_end().child(im).child(meta)).into_any_element()
        } else {
            div().my(zx(8.)).flex().flex_col().child(im).child(meta).into_any_element()
        }
    }

    fn parent(&mut self, e: &Arc<LogEntry>, w: &mut Window, cx: &mut Context<Self>) -> AnyElement {
        let k = c(cx).clone();
        let s = self.state.read(cx);
        let t = s.tasks.get(&s.current).map(|x| x.task.clone());
        let from = t.as_ref().and_then(|t| t.parent).or(e.meta_i64("from"));
        let by_user = e.meta_str("by") == Some("user");
        let who = |f: i64| if f == 0 { "the main chat".to_string() } else { format!("#{f}") };
        let label = match from {
            None => "from the parent".to_string(),
            Some(f) if by_user => format!("from you, as {}", who(f)),
            Some(f) => format!("from {}", who(f)),
        };
        let body: AnyElement = match &e.image {
            Some(r) => self.blob_img(r, 420., 360., cx),
            None => {
                let (txt, n) = self.cut_text(e);
                let doc = if n.is_some() { markdown::parse(&txt) } else { self.doc(e) };
                let mut d = div().child(markdown::render(&format!("p{}", e.i), &doc, &markdown::Opts { size: 14.0, color: k.text }, w, cx));
                if let Some(b) = self.cut_button(e, n, cx) {
                    d = d.child(b);
                }
                d.into_any_element()
            }
        };
        div()
            .my(zx(10.))
            .border_l(zx(3.))
            .border_color(k.violet)
            .bg(k.violet.opacity(0.06))
            .rounded_r(zx(12.))
            .rounded_l(zx(4.))
            .px(zx(12.))
            .pt(zx(8.))
            .pb(zx(10.))
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap(zx(8.))
                    .text_size(zx(12.))
                    .mb(zx(4.))
                    .child(
                        div()
                            .id(ElementId::Name(format!("pfrom{}", e.i).into()))
                            .text_color(k.violet)
                            .font_weight(FontWeight::MEDIUM)
                            .cursor_pointer()
                            .child(format!("↳ {label}"))
                            .when_some(from, |d, f| d.on_click(cx.listener(move |_, _, _, cx| cx.emit(FeedEvent::OpenTask(f))))),
                    )
                    .child(div().ml_auto().font_family(crate::ui::MONO).text_size(zx(10.5)).text_color(k.faint).child(format!("#{} · {}", e.i, fmt::time(&e.date)))),
            )
            .child(body)
            .into_any_element()
    }

    fn report(&mut self, e: &Arc<LogEntry>, w: &mut Window, cx: &mut Context<Self>) -> AnyElement {
        let k = c(cx).clone();
        let (id, kind, body) = parse_report(e);
        let s = self.state.read(cx);
        let child = s.tasks.get(&id).map(|x| task_name(&x.task)).unwrap_or_default();
        if let Some(r) = &e.image {
            let im = self.blob_img(r, 240., 200., cx);
            return div()
                .my(zx(8.))
                .flex()
                .gap(zx(8.))
                .px(zx(12.))
                .py(zx(8.))
                .border_1()
                .border_color(k.line2)
                .rounded(zx(12.))
                .child(div().font_family(crate::ui::MONO).text_color(k.violet).font_weight(FontWeight::BOLD).child(format!("#{id}")))
                .child(im)
                .into_any_element();
        }
        let key = format!("r{}", e.i);
        let open = self.open.contains(&key);
        let long = body.chars().count() > 320;
        let shown: String = if open || !long { body.clone() } else { format!("{}…", body.chars().take(320).collect::<String>()) };
        let (border, bg, kcol, klabel) = match kind.as_str() {
            "talk" => (k.green.opacity(0.28), k.panel, k.green, "replied"),
            "error" => (k.red.opacity(0.4), k.red.opacity(0.05), k.red, "error"),
            "status" => (k.line2, k.bg2, k.amber, "status"),
            "user" => (k.line2, k.bg2, k.muted, "you wrote to it"),
            _ => (k.line2, k.panel, k.muted, "report"),
        };
        let body_el: AnyElement = if kind == "talk" {
            let doc = markdown::parse(&shown);
            markdown::render(&format!("rb{}", e.i), &doc, &markdown::Opts { size: 13.5, color: k.text2 }, w, cx).into_any_element()
        } else {
            div().text_size(zx(13.5)).line_height(zx(21.)).text_color(k.text2).child(shown).into_any_element()
        };
        let hover = k.hover;
        let copy = body.clone();
        let me = cx.entity().downgrade();
        div()
            .id(ElementId::Name(format!("rc{}", e.i).into()))
            .context_menu(move |m, _, _| {
                let c2 = copy.clone();
                let me = me.clone();
                m.item(PopupMenuItem::new(format!("Open #{id}")).on_click(move |_, _, cx| {
                    let _ = me.update(cx, |_, cx| cx.emit(FeedEvent::OpenTask(id)));
                }))
                .item(PopupMenuItem::new("Copy text").on_click(move |_, _, cx| cx.write_to_clipboard(ClipboardItem::new_string(c2.clone()))))
            })
            .my(zx(8.))
            .border_1()
            .border_color(border)
            .rounded(zx(12.))
            .bg(bg)
            .overflow_hidden()
            .child(
                div()
                    .id(ElementId::Name(format!("rh{}", e.i).into()))
                    .flex()
                    .items_center()
                    .gap(zx(8.))
                    .px(zx(12.))
                    .py(zx(7.))
                    .text_size(zx(12.5))
                    .border_b_1()
                    .border_color(k.line)
                    .cursor_pointer()
                    .hover(move |s| s.bg(hover))
                    .on_click(cx.listener(move |_, _, _, cx| cx.emit(FeedEvent::OpenTask(id))))
                    .child(div().font_family(crate::ui::MONO).font_weight(FontWeight::BOLD).text_color(k.violet).child(format!("#{id}")))
                    .child(div().text_color(kcol).child(klabel))
                    .child(div().flex_1().min_w_0().overflow_hidden().line_clamp(1).text_ellipsis().text_color(k.text2).child(child))
                    .child(div().font_family(crate::ui::MONO).text_size(zx(10.5)).text_color(k.faint).child(format!("#{} · {}", e.i, fmt::time(&e.date))))
                    .child(div().text_size(zx(11.)).text_color(k.accent).child("open ↗")),
            )
            .child(
                div()
                    .id(ElementId::Name(format!("rbody{}", e.i).into()))
                    .px(zx(12.))
                    .pt(zx(8.))
                    .pb(zx(10.))
                    .when(long, |d| d.cursor_pointer().children(probe(format!("rep.{}", e.i))).on_click(cx.listener(move |f, _, _, cx| f.toggle(&format!("r{}", e_i(&key)), cx))))
                    .child(body_el),
            )
            .into_any_element()
    }

    fn work(&mut self, e: &Arc<LogEntry>, w: &mut Window, cx: &mut Context<Self>) -> AnyElement {
        let k = c(cx).clone();
        let parts: Vec<(String, String)> = split_work(&e.text);
        let mut d = div()
            .my(zx(10.))
            .border_1()
            .border_color(k.line2)
            .rounded(zx(12.))
            .bg(k.panel)
            .p(zx(12.))
            .child(
                div()
                    .flex()
                    .text_size(zx(12.))
                    .text_color(k.amber)
                    .mb(zx(6.))
                    .child(if parts.len() == 1 { "⭑ report from a background task".to_string() } else { format!("⭑ {} reports from background tasks", parts.len()) })
                    .child(div().flex_1())
                    .child(div().font_family(crate::ui::MONO).text_size(zx(10.5)).text_color(k.faint).child(format!("#{} · {}", e.i, fmt::time(&e.date)))),
            );
        for (n, (id, text)) in parts.into_iter().enumerate() {
            let tid: Option<i64> = id.strip_prefix('a').and_then(|x| x.parse().ok());
            let doc = markdown::parse(&text);
            d = d.child(
                div()
                    .mt(zx(6.))
                    .when(!id.is_empty(), |d| {
                        d.child(
                            div()
                                .id(ElementId::Name(format!("w{}.{n}", e.i).into()))
                                .text_size(zx(12.))
                                .font_family(crate::ui::MONO)
                                .text_color(k.violet)
                                .cursor_pointer()
                                .children(probe(format!("work.{}.{n}", e.i)))
                                .child(match tid {
                                    Some(t) => format!("{id} → #{t}"),
                                    None => id.clone(),
                                })
                                .when_some(tid, |d, t| d.on_click(cx.listener(move |_, _, _, cx| cx.emit(FeedEvent::OpenTask(t))))),
                        )
                    })
                    .child(markdown::render(&format!("w{}.{n}", e.i), &doc, &markdown::Opts { size: 13.5, color: k.text }, w, cx)),
            );
        }
        d.into_any_element()
    }

    fn talk(&mut self, e: &Arc<LogEntry>, w: &mut Window, cx: &mut Context<Self>) -> AnyElement {
        let k = c(cx).clone();
        let (txt, n) = self.cut_text(e);
        let doc = if n.is_some() { markdown::parse(&txt) } else { self.doc(e) };
        let copy = e.text.clone();
        div()
            .id(ElementId::Name(format!("talk{}", e.i).into()))
            .my(zx(8.))
            .child(markdown::render(&format!("t{}", e.i), &doc, &markdown::Opts { size: 14.0, color: k.text }, w, cx))
            .children(self.cut_button(e, n, cx))
            .context_menu(move |m, _, _| {
                m.item(PopupMenuItem::new("Copy markdown").on_click({
                    let c2 = copy.clone();
                    move |_, _, cx| cx.write_to_clipboard(ClipboardItem::new_string(c2.clone()))
                }))
            })
            .into_any_element()
    }

    fn tool(&mut self, e: &Arc<LogEntry>, echo: Option<&Arc<LogEntry>>, images: &[Arc<LogEntry>], cx: &mut Context<Self>) -> AnyElement {
        let k = c(cx).clone();
        let key = format!("t{}", e.i);
        let open = self.open.contains(&key);
        if self.compact && !open {
            return self.tool_line(e, echo, &key, cx);
        }
        let name = e.meta_str("name").map(String::from).unwrap_or_else(|| e.text.split(' ').next().unwrap_or("").to_string());
        let cid = e.meta_str("callId").unwrap_or("").to_string();
        let st: Option<ToolState> = self.state.read(cx).cur.tools.get(&cid).cloned();
        let input: serde_json::Value = match &st {
            Some(s) => s.input.clone(),
            None if e.kind == LogKind::Tool => {
                let rest = e.text.splitn(2, ' ').nth(1).unwrap_or("");
                serde_json::from_str(rest).unwrap_or_else(|_| lenient_json(rest).unwrap_or(serde_json::Value::String(e.text.clone())))
            }
            None => serde_json::Value::Null,
        };
        let state = match (echo, &st) {
            (Some(x), _) => if x.meta_bool("isError") { "err" } else { "ok" },
            (None, Some(s)) if s.running => "run",
            (None, Some(s)) => if s.result.as_ref().map(|r| r.is_error).unwrap_or(false) { "err" } else { "ok" },
            (None, None) => "run",
        };
        let (scol, stext) = match state {
            "ok" => (k.green, "done".to_string()),
            "err" => (k.red, "error".to_string()),
            _ => (k.amber, match st.as_ref().and_then(|s| s.started_at.as_ref()) {
                Some(t) => format!("running · {}", fmt::elapsed(t)),
                None => "running…".into(),
            }),
        };
        let can_stop = state == "run" && st.as_ref().map(|s| s.running).unwrap_or(false);
        let hover = k.panel;
        let state_e = self.state.clone();
        let cid2 = cid.clone();
        let mut card = div()
            .my(zx(8.))
            .border_1()
            .border_color(k.line)
            .bg(k.bg2)
            .rounded(zx(10.))
            .overflow_hidden()
            .child(
                div()
                    .id(ElementId::Name(format!("th{}", e.i).into()))
                    .children(probe(format!("tool.{}", e.i)))
                    .flex()
                    .items_center()
                    .gap(zx(9.))
                    .px(zx(11.))
                    .py(zx(7.))
                    .text_size(zx(13.))
                    .cursor_pointer()
                    .hover(move |s| s.bg(hover))
                    .on_click(cx.listener({
                        let key = key.clone();
                        move |f, _, _, cx| f.toggle(&key, cx)
                    }))
                    .child(chevron(format!("tool{}.{}", self.task, e.i), 12., k.faint, open))
                    .child(div().font_family(crate::ui::MONO).text_size(zx(12.)).text_color(k.cyan).whitespace_nowrap().child(name.clone()))
                    .child(div().flex_1().min_w_0().overflow_hidden().line_clamp(1).text_ellipsis().font_family(crate::ui::MONO).text_size(zx(12.)).text_color(k.text2).child(arg_preview(&name, &input)))
                    .child(div().text_size(zx(11.)).whitespace_nowrap().text_color(scol).child(stext))
                    .when(can_stop, |d| {
                        d.child(btn_small(ElementId::Name(format!("stop{}", e.i).into()), "stop", Tone::Danger, cx).on_click(move |_, _, cx| {
                            cx.stop_propagation();
                            let c3 = cid2.clone();
                            state_e.update(cx, |s, cx| s.command(cx, false, move |h| async move { { h.stop_tool(&c3); Ok(String::new()) } }));
                        }))
                    })
                    .child(div().font_family(crate::ui::MONO).text_size(zx(10.5)).text_color(k.faint).child(format!("#{}{}", e.i, echo.map(|x| format!("–{}", x.i)).unwrap_or_default()))),
            );
        if name.starts_with("task_") && name != "task_list" {
            let mut ids: Vec<i64> = vec![];
            if let Some(i) = input.get("id").and_then(|v| v.as_i64()) {
                ids.push(i);
            }
            if let Some(x) = echo {
                let mut rest = x.text.as_str();
                while let Some(p) = rest.find('#') {
                    let digits: String = rest[p + 1..].chars().take_while(|c| c.is_ascii_digit()).collect();
                    if let Ok(n) = digits.parse::<i64>() {
                        if !ids.contains(&n) {
                            ids.push(n);
                        }
                    }
                    rest = &rest[p + 1..];
                }
            }
            if !ids.is_empty() {
                let mut chips = div().flex().flex_wrap().gap(zx(6.)).px(zx(10.)).pb(zx(8.));
                for id in ids.into_iter().take(8) {
                    chips = chips.child(
                        chip(format!("#{id} ↗"), ChipTone::Violet, cx)
                            .id(ElementId::Name(format!("tc{}.{id}", e.i).into()))
                            .cursor_pointer()
                            .on_click(cx.listener(move |_, _, _, cx| cx.emit(FeedEvent::OpenTask(id)))),
                    );
                }
                card = card.child(chips);
            }
        }
        if open {
            let lbl = |t: &str| div().text_size(zx(10.5)).text_color(k.muted).px(zx(12.)).pt(zx(8.)).child(t.to_uppercase());
            let pre = |id: String, t: String| {
                div()
                    .id(ElementId::Name(id.into()))
                    .max_h(zx(420.))
                    .overflow_y_scroll()
                    .px(zx(12.))
                    .py(zx(8.))
                    .font_family(crate::ui::MONO)
                    .text_size(zx(12.))
                    .line_height(zx(18.))
                    .text_color(k.text2)
                    .child(t)
            };
            let mut detail = div().border_t_1().border_color(k.line);
            if e.kind == LogKind::Tool {
                let txt = match &input {
                    serde_json::Value::String(s) => s.clone(),
                    v => serde_json::to_string_pretty(v).unwrap_or_default(),
                };
                let copy_in = txt.clone();
                detail = detail.child(lbl("input")).child(pre(format!("tin{}", e.i), txt).context_menu(move |m, _, _| m.item(PopupMenuItem::new("Copy input").on_click({ let c2 = copy_in.clone(); move |_, _, cx| cx.write_to_clipboard(ClipboardItem::new_string(c2.clone())) }))));
            }
            if let Some(x) = echo {
                let (txt, n) = self.cut_text(x);
                let copy_out = x.text.clone();
                detail = detail.child(lbl("output")).child(pre(format!("tout{}", e.i), txt.to_string()).context_menu(move |m, _, _| m.item(PopupMenuItem::new("Copy output").on_click({ let c2 = copy_out.clone(); move |_, _, cx| cx.write_to_clipboard(ClipboardItem::new_string(c2.clone())) }))));
                if let Some(b) = self.cut_button(x, n, cx) {
                    detail = detail.child(div().px(zx(10.)).child(b));
                }
            } else if let Some(r) = st.as_ref().and_then(|s| s.result.clone()) {
                detail = detail.child(lbl("output")).child(pre(format!("tres{}", e.i), optchat_proto::api::result_text(&r.content)));
            }
            let mut ims = div().flex().flex_wrap().gap(zx(8.)).p(zx(10.));
            let mut any = false;
            for x in images {
                if let Some(r) = &x.image {
                    ims = ims.child(self.blob_img(r, 240., 200., cx));
                    any = true;
                }
            }
            if any {
                detail = detail.child(ims.children(probe(format!("toolimg.{}", e.i))));
            }
            card = card.child(detail);
        }
        card.into_any_element()
    }

    fn live(&mut self, turn_id: &str, w: &mut Window, cx: &mut Context<Self>) -> AnyElement {
        let k = c(cx).clone();
        let s = self.state.read(cx);
        let turn = s.cur.turns.get(turn_id).cloned();
        let lives: Vec<(String, String, String)> = s
            .cur
            .live
            .iter()
            .filter(|(id, v)| v.turn_id == turn_id && s.cur.requests.get(*id).map(|r| r.status == "running").unwrap_or(true))
            .map(|(id, v)| (id.clone(), v.text.clone(), v.thinking.clone()))
            .collect();
        let tool_running = s.cur.tools.values().any(|t| t.running && t.turn_id == turn_id);
        let unbuilt = s.cur.view.as_ref().map(|v| v.parts.iter().filter(|p| !p.built).count()).unwrap_or(0);
        let busy = s.busy.len();
        let failed = s.failed.len();
        let mut d = div().flex().flex_col().gap(zx(6.));
        let waiting_line = |text: String, color: Hsla, id: &str| {
            crate::ui::seen("waitingSeen", if text.starts_with("waiting for summaries") { "summaries" } else { "model" });
            crate::ui::set_state_extra("waitingLine", serde_json::json!(text.clone()));
            if text.starts_with("waiting for summaries") {
                crate::ui::set_state_extra("waitingSummaries", serde_json::json!(text.clone()));
            }
            div()
                .flex()
                .items_center()
                .gap(zx(8.))
                .my(zx(8.))
                .text_size(zx(13.))
                .text_color(k.muted)
                .child(pulse(ElementId::Name(format!("wl{id}").into()), dot(color, 7.)))
                .child(text)
        };
        let (tl, th): (usize, usize) = lives.iter().fold((0, 0), |a, x| (a.0 + x.1.len(), a.1 + x.2.len()));
        if tl > self.live_max.0 || th > self.live_max.1 {
            self.live_max = (self.live_max.0.max(tl), self.live_max.1.max(th));
            if tl > 0 {
                crate::ui::seen("liveSeen", "text");
            }
            if th > 0 {
                crate::ui::seen("liveSeen", if tl == 0 { "thinking" } else { "thought" });
            }
        }
        for (rid, text, thinking) in &lives {
            if !thinking.is_empty() {
                let key = format!("th{rid}");
                let open = self.open.contains(&key);
                let last = thinking.lines().filter(|l| !l.trim().is_empty()).last().unwrap_or("").to_string();
                let live_now = text.is_empty();
                d = d.child(
                    div()
                        .flex()
                        .flex_col()
                        .child(
                            div()
                                .id(ElementId::Name(format!("thh{rid}").into()))
                                .flex()
                                .items_center()
                                .gap(zx(7.))
                                .text_size(zx(13.))
                                .text_color(k.muted)
                                .cursor_pointer()
                                .on_click(cx.listener({
                                    let key = key.clone();
                                    move |f, _, _, cx| f.toggle(&key, cx)
                                }))
                                .child(chevron(format!("think{rid}"), 11., k.faint, open))
                                .when(live_now, |d| d.child(pulse(ElementId::Name(format!("thp{rid}").into()), dot(k.accent, 7.))).child("thinking…"))
                                .when(!live_now, |d| d.child("thought"))
                                .when(!open, |d| d.child(div().flex_1().min_w_0().overflow_hidden().line_clamp(1).text_ellipsis().italic().child(last))),
                        )
                        .when(open, |dd| dd.child(div().mt(zx(6.)).pl(zx(12.)).border_l_2().border_color(k.line2).italic().text_size(zx(13.)).line_height(zx(20.)).text_color(k.muted).child(thinking.clone()))),
                );
            }
            if !text.is_empty() {
                let doc = match self.live_md.get(rid) {
                    Some((n, d)) if *n == text.len() => d.clone(),
                    _ => {
                        let d = markdown::parse(text);
                        self.live_md.insert(rid.clone(), (text.len(), d.clone()));
                        d
                    }
                };
                d = d.child(
                    div()
                        .child(markdown::render(&format!("lv{rid}"), &doc, &markdown::Opts { size: 14.0, color: k.text }, w, cx))
                        .child(pulse(ElementId::Name(format!("cur{rid}").into()), div().w(zx(8.)).h(zx(16.)).rounded(zx(2.)).bg(k.accent)).into_any_element())
                        .child({
                            crate::ui::seen("liveSeen", "cursor");
                            div()
                        }),
                );
            }
            if text.is_empty() && thinking.is_empty() {
                d = d.child(waiting_line("waiting for the model…".into(), k.accent, rid));
            }
        }
        if let Some(t) = &turn {
            if t.status == "waiting" {
                let mut s = format!("waiting for summaries: {unbuilt} left");
                if busy > 0 {
                    s.push_str(&format!(" ({busy} in progress)"));
                }
                if failed > 0 {
                    s.push_str(&format!(" · {failed} retrying"));
                }
                s.push_str(&format!(" · {}", fmt::elapsed(&t.started_at)));
                d = d.child(waiting_line(s, k.amber, "w"));
            } else if t.status == "running" && lives.is_empty() && !tool_running {
                d = d.child(waiting_line("waiting for the model…".into(), k.accent, "m"));
            }
        }
        d.into_any_element()
    }

    /// compact: "Opus 5.5 · $0.035 · 8.8s ›", tap for the turn's numbers
    fn foot_compact(&mut self, turn_id: &str, cx: &mut Context<Self>) -> AnyElement {
        let k = c(cx).clone();
        let s = self.state.read(cx);
        let Some(turn) = s.cur.turns.get(turn_id).cloned() else { return div().into_any_element() };
        let model_label = s.model(&turn.model).map(|m| m.label.replace("Claude ", "")).unwrap_or(turn.model.clone());
        let nreq = s.cur.requests.values().filter(|r| r.turn_id.as_deref() == Some(turn_id)).count();
        let last_req = s.cur.requests.values().filter(|r| r.turn_id.as_deref() == Some(turn_id)).max_by(|a, b| a.started_at.cmp(&b.started_at)).map(|r| r.id.clone());
        let key = format!("foot{turn_id}");
        let open = self.open.contains(&key);
        let dur = turn.done_at.as_ref().map(|d| fmt::dur(Some(&turn.started_at), Some(d)));
        let mut line = format!("{model_label} · {}", fmt::cost(Some(&turn.cost)));
        if let Some(d) = &dur {
            line.push_str(&format!(" · {d}"));
        }
        if turn.status == "cancelled" {
            line.push_str(" · stopped");
        }
        let mut d = div().flex().flex_col().mt(zx(6.)).mb(zx(14.));
        if let Some(err) = &turn.error {
            d = d.child(div().text_size(zx(13.)).text_color(k.red).my(zx(6.)).child(format!("⚠ {err}")).children(probe(format!("foot.err.{turn_id}"))));
        }
        d = d.child(
            div()
                .id(ElementId::Name(format!("fc{turn_id}").into()))
                .flex()
                .items_center()
                .gap(zx(4.))
                .text_size(zx(12.5))
                .text_color(k.muted)
                .cursor_pointer()
                .child(line)
                .child(chevron(format!("fcc{turn_id}"), 11., k.faint, open))
                .children(probe(format!("foot.line.{turn_id}")))
                .on_click(cx.listener({
                    let key = key.clone();
                    move |f, _, _, cx| f.toggle(&key, cx)
                })),
        );
        if open {
            let u = &turn.usage;
            let kv = |a: &str, b: String| div().flex().gap(zx(10.)).child(div().w(zx(96.)).flex_none().text_color(k.muted).child(a.to_string())).child(div().flex_1().min_w_0().text_color(k.text2).child(b));
            let mut t = div().mt(zx(6.)).flex().flex_col().gap(zx(3.)).text_size(zx(12.5));
            t = t.child(kv("model", format!("{model_label}{}", turn.effort.as_ref().map(|e| format!(", {e} effort")).unwrap_or_default())));
            t = t.child(kv("steps", nreq.max(1).to_string()));
            t = t.child(kv("input", format!("{} tokens, {}% cached", fmt::tokens(u.input), fmt::cache_pct(Some(u)))));
            t = t.child(kv("output", format!("{} tokens", fmt::tokens(u.output))));
            t = t.child(kv("cost · time", format!("{}{}", fmt::cost(Some(&turn.cost)), dur.map(|d| format!(" · {d}")).unwrap_or_default())));
            if let Some(rid) = last_req {
                t = t.child(div().id(ElementId::Name(format!("fctx{turn_id}").into())).ml(zx(106.)).text_color(k.accent).cursor_pointer().child("Open the context ›").on_click(cx.listener(move |_, _, _, cx| cx.emit(FeedEvent::SelectRequest(rid.clone())))));
            }
            d = d.child(t);
        }
        d.into_any_element()
    }

    /// compact: one quiet line per tool call ("⁘ bash ls -la … 4.2s ■"), tap to expand
    fn tool_line(&mut self, e: &Arc<LogEntry>, echo: Option<&Arc<LogEntry>>, key: &str, cx: &mut Context<Self>) -> AnyElement {
        let k = c(cx).clone();
        let name = e.meta_str("name").map(String::from).unwrap_or_else(|| e.text.split(' ').next().unwrap_or("").to_string());
        let cid = e.meta_str("callId").unwrap_or("").to_string();
        let st = self.state.read(cx).cur.tools.get(&cid).cloned();
        let running = echo.is_none() && st.as_ref().map(|s| s.running).unwrap_or(false);
        let err = echo.map(|x| x.meta_bool("isError")).unwrap_or(false) || st.as_ref().and_then(|s| s.result.as_ref()).map(|r| r.is_error).unwrap_or(false);
        let rest = e.text.splitn(2, ' ').nth(1).unwrap_or("").to_string();
        let summary = match serde_json::from_str::<serde_json::Value>(&rest) {
            Ok(serde_json::Value::Object(o)) => o.values().find_map(|v| v.as_str().map(String::from)).unwrap_or(rest.clone()),
            _ => rest.clone(),
        };
        let summary: String = summary.replace('\n', " ").chars().take(160).collect();
        let time = st.as_ref().and_then(|s| s.started_at.clone()).filter(|_| running).map(|t| fmt::elapsed(&t));
        let key = key.to_string();
        let state_e = self.state.clone();
        div()
            .id(ElementId::Name(format!("tl{}", e.i).into()))
            .flex()
            .items_center()
            .gap(zx(7.))
            .my(zx(5.))
            .text_size(zx(12.5))
            .text_color(k.muted)
            .cursor_pointer()
            .child(if running { spinner(ElementId::Name(format!("tls{}", e.i).into()), 12., k.muted).into_any_element() } else { dot(if err { k.red } else { k.faint }, 6.).into_any_element() })
            .child(div().flex_1().min_w_0().overflow_hidden().whitespace_nowrap().text_ellipsis().child(format!("{name} {summary}")))
            .when_some(time, |d, t| d.child(div().flex_none().text_color(k.accent).child(t)))
            .when(running, |d| {
                let cid = cid.clone();
                d.child(
                    div()
                        .id(ElementId::Name(format!("tlx{}", e.i).into()))
                        .flex_none()
                        .size(zx(22.))
                        .rounded_full()
                        .bg(k.red_bg)
                        .flex()
                        .items_center()
                        .justify_center()
                        .child(div().size(zx(7.)).rounded(zx(1.5)).bg(k.red))
                        .on_click(move |_, _, cx| {
                            cx.stop_propagation();
                            let cid = cid.clone();
                            state_e.update(cx, |s, cx| s.command(cx, false, move |h| async move { h.stop_tool(&cid); Ok(String::new()) }));
                        }),
                )
            })
            .children(probe(format!("tool.{}", e.i)))
            .on_click(cx.listener(move |f, _, _, cx| f.toggle(&key, cx)))
            .into_any_element()
    }

    fn foot(&mut self, turn_id: &str, cx: &mut Context<Self>) -> AnyElement {
        if self.compact {
            return self.foot_compact(turn_id, cx);
        }
        let k = c(cx).clone();
        let s = self.state.read(cx);
        let Some(turn) = s.cur.turns.get(turn_id).cloned() else { return div().into_any_element() };
        let running = s.cur.running.as_ref().map(|r| r.id == turn.id).unwrap_or(false);
        let selected = s.shown_request();
        let mut reqs: Vec<RequestRecord> = s.cur.requests.values().filter(|r| r.turn_id.as_deref() == Some(turn_id)).cloned().collect();
        reqs.sort_by(|a, b| a.started_at.cmp(&b.started_at));
        let key = format!("foot{turn_id}");
        let open = self.open.contains(&key);
        let last = reqs.last().cloned();
        let switched = reqs.iter().any(|r| r.model != turn.model || (r.effort.is_some() && r.effort != turn.effort));
        let lm = last.as_ref().map(|r| r.model.clone()).unwrap_or(turn.model.clone());
        let le = last.as_ref().and_then(|r| r.effort.clone()).or(turn.effort.clone());
        let label = format!(
            "{}{}{}",
            turn.model,
            if lm != turn.model { format!(" → {lm}") } else { String::new() },
            match &turn.effort {
                Some(e) => format!(" · {e}{}", match &le { Some(l) if l != e => format!(" → {l}"), _ => String::new() }),
                None => String::new(),
            }
        );
        let u = &turn.usage;
        let mut row = div().flex().flex_wrap().items_center().gap(zx(6.)).mt(zx(10.)).text_size(zx(11.5));
        row = row.child(chip(label, if turn.backend == Backend::ClaudeCode { ChipTone::Acc } else { ChipTone::Green }, cx).children(probe(format!("foot.model.{}", turn.id))));
        row = row.child(
            chip(format!("{} step{} {}", reqs.len(), if reqs.len() == 1 { "" } else { "s" }, if open { "▴" } else { "▾" }), ChipTone::Plain, cx)
                .id(ElementId::Name(format!("steps{turn_id}").into()))
                .children(probe(format!("foot.steps.{turn_id}")))
                .cursor_pointer()
                .on_click(cx.listener({
                    let key = key.clone();
                    move |f, _, _, cx| f.toggle(&key, cx)
                })),
        );
        row = row.child(chip(format!("in {} ({}% cached)", fmt::tokens(u.input), fmt::cache_pct(Some(u))), ChipTone::Plain, cx));
        row = row.child(chip(format!("out {}", fmt::tokens(u.output)), ChipTone::Plain, cx));
        row = row.child(chip(fmt::cost(Some(&turn.cost)), if turn.cost.priced { ChipTone::Plain } else { ChipTone::Amber }, cx));
        if let Some(d) = &turn.done_at {
            row = row.child(chip(fmt::dur(Some(&turn.started_at), Some(d)), ChipTone::Plain, cx));
        }
        if running {
            row = row.child(chip("live", ChipTone::Amber, cx).child(pulse(ElementId::Name(format!("lv{turn_id}").into()), dot(k.amber, 6.))));
        }
        if turn.status == "cancelled" {
            row = row.child(chip("stopped", ChipTone::Red, cx));
        }
        if let Some(l) = &last {
            let rid = l.id.clone();
            row = row.child(chip("context ↗", ChipTone::Plain, cx).id(ElementId::Name(format!("ctx{turn_id}").into())).children(probe(format!("foot.ctx.{turn_id}"))).cursor_pointer().on_click(cx.listener(move |_, _, _, cx| cx.emit(FeedEvent::SelectRequest(rid.clone())))));
        }
        let mut d = div().flex().flex_col().mb(zx(18.));
        if let Some(err) = &turn.error {
            d = d.child(div().text_size(zx(13.)).text_color(k.red).my(zx(6.)).child(format!("⚠ {err}")).children(probe(format!("foot.err.{turn_id}"))));
        }
        d = d.child(row);
        if open {
            let mut t = div().mt(zx(8.)).flex().flex_col().text_size(zx(12.)).border_1().border_color(k.line).rounded(zx(8.)).overflow_hidden();
            let cols: Vec<&str> = if switched { vec!["#", "model", "stop", "in", "cached", "write", "out", "cost", "time"] } else { vec!["#", "stop", "in", "cached", "write", "out", "cost", "time"] };
            let cell = |s: String, right: bool, w: f32| {
                let d = div().w(zx(w)).flex_none().px(zx(6.)).py(zx(3.)).overflow_hidden().whitespace_nowrap();
                if right { d.flex().justify_end().child(s) } else { d.child(s) }
            };
            let widths = |c: &str| match c {
                "#" => 26.,
                "model" => 170.,
                "stop" => 76.,
                _ => 64.,
            };
            let mut head = div().flex().bg(k.panel).text_color(k.muted);
            for col in &cols {
                head = head.child(cell(col.to_string(), !matches!(*col, "#" | "model" | "stop"), widths(col)));
            }
            t = t.child(head);
            for (n, r) in reqs.iter().enumerate() {
                let rid = r.id.clone();
                let sel = selected.as_deref() == Some(&r.id);
                let u = r.usage.clone().unwrap_or_default();
                let mut tr = div()
                    .id(ElementId::Name(format!("req{}", r.id).into()))
                    .flex()
                    .text_color(k.text2)
                    .cursor_pointer()
                    .when(sel, |d| d.bg(k.accent_bg))
                    .hover(|s| s.bg(k.hover))
                    .on_click(cx.listener(move |_, _, _, cx| cx.emit(FeedEvent::SelectRequest(rid.clone()))));
                for col in &cols {
                    let v = match *col {
                        "#" => (n + 1).to_string(),
                        "model" => format!("{}{}", r.model, r.effort.as_ref().map(|e| format!(" · {e}")).unwrap_or_default()),
                        "stop" => if r.status == "running" { "running".into() } else { r.stop.clone().unwrap_or_default() },
                        "in" => fmt::tokens(u.input),
                        "cached" => fmt::tokens(u.cache_read),
                        "write" => fmt::tokens(u.cache_write),
                        "out" => fmt::tokens(u.output),
                        "cost" => fmt::cost(r.cost.as_ref()),
                        _ => fmt::dur(Some(&r.started_at), r.done_at.as_deref()),
                    };
                    tr = tr.child(cell(v, !matches!(*col, "#" | "model" | "stop"), widths(col)));
                }
                t = t.child(tr);
            }
            d = d.child(t);
        }
        d.into_any_element()
    }
}

fn e_i(key: &str) -> String {
    key.trim_start_matches('r').to_string()
}

pub fn parse_report(e: &LogEntry) -> (i64, String, String) {
    // "#7 talk: …"
    let t = &e.text;
    if let Some(rest) = t.strip_prefix('#') {
        let digits: String = rest.chars().take_while(|c| c.is_ascii_digit()).collect();
        let after = &rest[digits.len()..];
        for kind in ["talk", "user", "error", "status"] {
            let pre = format!(" {kind}: ");
            if after.starts_with(&pre) {
                return (digits.parse().unwrap_or(-1), kind.into(), after[pre.len()..].to_string());
            }
        }
    }
    (e.meta_i64("from").unwrap_or(-1), e.meta_str("report").unwrap_or("report").to_string(), t.clone())
}

fn split_work(text: &str) -> Vec<(String, String)> {
    let mut out = vec![];
    for part in text.split("\n\n[") {
        let p = if out.is_empty() { part.to_string() } else { format!("[{part}") };
        if let Some(rest) = p.strip_prefix('[') {
            if let Some(end) = rest.find("] ") {
                out.push((rest[..end].to_string(), rest[end + 2..].to_string()));
                continue;
            }
        }
        if let Some(last) = out.last_mut() {
            last.1.push_str("\n\n");
            last.1.push_str(&p);
        } else {
            out.push((String::new(), p));
        }
    }
    out
}

/// The complete top-level fields of a JSON object whose text was cut (long tool inputs are stored shortened):
/// `{"mode":"handoff","title":"x","instruction":"very lo` -> {"mode":"handoff","title":"x"}.
pub fn lenient_json(text: &str) -> Option<serde_json::Value> {
    let t = text.trim_start();
    if !t.starts_with('{') {
        return None;
    }
    let mut out = serde_json::Map::new();
    // walk "key": value pairs with a tiny scanner: each value is handed to serde, a cut one ends the walk
    let b = t.as_bytes();
    let mut i = 1;
    loop {
        while i < b.len() && (b[i] == b' ' || b[i] == b',' || b[i] == b'\n' || b[i] == b'\t' || b[i] == b'\r') {
            i += 1;
        }
        if i >= b.len() || b[i] != b'"' {
            break;
        }
        let mut stream = serde_json::Deserializer::from_str(&t[i..]).into_iter::<String>();
        let Some(Ok(key)) = stream.next() else { break };
        i += stream.byte_offset();
        while i < b.len() && (b[i] == b' ' || b[i] == b':') {
            i += 1;
        }
        let mut vs = serde_json::Deserializer::from_str(&t[i..]).into_iter::<serde_json::Value>();
        match vs.next() {
            Some(Ok(v)) => {
                i += vs.byte_offset();
                out.insert(key, v);
            }
            _ => break,
        }
    }
    if out.is_empty() { None } else { Some(serde_json::Value::Object(out)) }
}

pub fn arg_preview(name: &str, input: &serde_json::Value) -> String {
    let s = |k: &str| input.get(k).map(|v| v.as_str().map(String::from).unwrap_or_else(|| v.to_string())).unwrap_or_default();
    let has = |k: &str| input.get(k).map(|v| !v.is_null()).unwrap_or(false);
    let undef = |k: &str| if has(k) { s(k) } else { "undefined".into() };
    match name {
        "bash" => s("command"),
        "computer" => if has("title") { s("title") } else { s("code") },
        "zoom" => format!("{}+{}{}", undef("id"), undef("n"), if has("task") { format!("@{}", s("task")) } else { String::new() }),
        "date" => format!("{}{}", undef("id"), if has("task") { format!("@{}", s("task")) } else { String::new() }),
        "task_spawn" => format!("{}: {}", undef("mode"), undef("title")),
        "task_tell" | "task_resume" => format!("#{}{}", undef("id"), if has("message") && !s("message").is_empty() { format!(": {}", s("message")) } else { String::new() }),
        "task_pause" | "task_cancel" => format!("#{}{}", undef("id"), if has("reason") && !s("reason").is_empty() { format!(": {}", s("reason")) } else { String::new() }),
        "task_close" => format!("#{}: {}", undef("id"), s("result")),
        "task_list" => if has("id") { format!("#{}", s("id")) } else { "children".into() },
        _ => {
            for k in ["path", "pattern", "url"] {
                if let Some(v) = input.get(k).and_then(|v| v.as_str()) {
                    return v.to_string();
                }
            }
            match input {
                serde_json::Value::String(t) => t.clone(),
                v => v.to_string(),
            }
        }
    }
}

fn group_thousands(n: usize) -> String {
    let s = n.to_string();
    let mut out = String::new();
    for (i, ch) in s.chars().enumerate() {
        if i > 0 && (s.len() - i) % 3 == 0 {
            out.push(',');
        }
        out.push(ch);
    }
    out
}

pub fn logo(size: f32, cx: &App) -> Div {
    let k = c(cx);
    div()
        .size(zx(size))
        .rounded(zx(size * 0.3))
        .bg(linear_gradient(200., linear_color_stop(k.accent, 0.), linear_color_stop(k.violet, 1.)))
        .shadow(vec![BoxShadow { color: k.accent.opacity(0.35), offset: point(zx(0.), zx(0.)), blur_radius: zx(18.), spread_radius: zx(0.), inset: false }])
}

#[allow(dead_code)]
fn _unused(_: &C) {}

#[cfg(test)]
mod lenient_tests {
    use super::*;
    #[::core::prelude::v1::test]
    fn cut_tool_input() {
        let v = lenient_json(r#"{"mode":"handoff","title":"clean caches","n":3,"instruction":"do this and th"#).unwrap();
        assert_eq!(v["mode"], "handoff");
        assert_eq!(v["title"], "clean caches");
        assert_eq!(v["n"], 3);
        assert!(v.get("instruction").is_none());
        assert_eq!(arg_preview("task_spawn", &v), "handoff: clean caches");
        assert!(lenient_json("not json").is_none());
    }
}
