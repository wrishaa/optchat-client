//! Sidebar views (app/ui/panels.tsx, models.tsx): the memory tree, the context debugger, the request log with the
//! ledger breakdown, the models view with rates, and settings (Global and the task's).
use crate::fmt;
use crate::state::*;
use crate::theme::{c, Themes};
use crate::ui::*;
use gpui::prelude::FluentBuilder;
use gpui::*;
use crate::state::View;
use optchat_proto::types::Role;
use gpui::Context;
use crate::kit::button::Button;
use crate::kit::input::{Input, InputEvent, InputState};
use crate::kit::menu::PopupMenuItem;
use crate::kit::button::Switch;
use optchat_proto::api::*;
use optchat_proto::rates::Per1M;
use optchat_proto::types::*;
use std::collections::{HashMap, HashSet};
use std::sync::Arc;

use crate::client::Client;
/// Run `f` on the client's runtime and hand the result to `done` on this entity.
pub fn fetch<E: 'static, R: Send + 'static, F: std::future::Future<Output = R> + Send + 'static>(state: &Entity<AppState>, cx: &mut Context<E>, f: impl FnOnce(Arc<Client>) -> F + Send + 'static, done: impl FnOnce(&mut E, R, &mut Context<E>) + 'static) {
    let s = state.read(cx);
    let Some(hub) = s.hub() else { return };
    let (tx, rx) = tokio::sync::oneshot::channel();
    s.rt.spawn(async move {
        let _ = tx.send(f(hub).await);
    });
    cx.spawn(async move |this, cx| {
        if let Ok(r) = rx.await {
            let _ = this.update(cx, |e, cx| {
                done(e, r, cx);
                cx.notify();
            });
        }
    })
    .detach();
}

fn pad(d: Div) -> Div {
    d.px(px(12.)).pb(px(14.))
}
fn section(cx: &App) -> Div {
    let _ = cx;
    div().mb(px(16.))
}
fn seg(id: impl Into<SharedString>, label: impl Into<SharedString>, on: bool, cx: &App) -> Stateful<Div> {
    let k = c(cx);
    let hover = k.hover;
    div()
        .id(ElementId::Name(id.into()))
        .px(px(9.))
        .py(px(3.))
        .rounded(px(6.))
        .text_size(px(12.))
        .cursor_pointer()
        .text_color(if on { k.text } else { k.muted })
        .when(on, |d| d.bg(k.hover).border_1().border_color(k.line2))
        .when(!on, |d| d.hover(move |s| s.bg(hover)))
        .child(label.into())
}
fn bar(parts: &[(f32, Hsla)], cx: &App) -> Div {
    let k = c(cx);
    let mut d = div().flex().h(px(6.)).w_full().rounded(px(3.)).overflow_hidden().bg(k.line).my(px(8.));
    for (f, col) in parts {
        d = d.child(div().h_full().w(relative(f.clamp(0.0, 1.0))).bg(*col));
    }
    d
}

// =====================================================================================================================
// Memory
// =====================================================================================================================
pub struct MemoryPanel {
    state: Entity<AppState>,
    texts: HashMap<(i64, u32, i64), String>,
    cview: Vec<ViewPart>,
    open: HashSet<(i64, u32, i64)>,
    kids: HashMap<(i64, u32, i64), (Option<Arc<LogEntry>>, Vec<NodeKid>)>,
    hit: Option<(i64, u32, i64)>,
    which_c: bool,
    rendered: Option<Rendered>,
    search: Entity<InputState>,
    hits: Option<Vec<SearchHit>>,
    seen: (i64, u64, i64, usize, u64),
    scroll: ScrollHandle,
    /// set by reveal: the highlighted row scrolls itself into view once it is laid out
    want_hit: std::rc::Rc<std::cell::Cell<bool>>,
    _subs: Vec<Subscription>,
}
impl MemoryPanel {
    pub fn new(state: Entity<AppState>, window: &mut Window, cx: &mut Context<Self>) -> MemoryPanel {
        let search = cx.new(|cx| InputState::new(window, cx).placeholder("search this task's memory…"));
        let s1 = cx.subscribe(&search, |m: &mut MemoryPanel, _, e: &InputEvent, cx| {
            if let InputEvent::PressEnter { .. } = e {
                m.do_search(cx);
            }
        });
        let s2 = cx.observe(&state, |m: &mut MemoryPanel, _, cx| { m.maybe_refresh(cx); cx.notify(); });
        MemoryPanel { state, texts: HashMap::new(), cview: vec![], open: HashSet::new(), kids: HashMap::new(), hit: None, which_c: false, rendered: None, search, hits: None, seen: (-1, 0, -1, 0, 0), scroll: ScrollHandle::new(), want_hit: Default::default(), _subs: vec![s1, s2] }
    }
    fn maybe_refresh(&mut self, cx: &mut Context<Self>) {
        let s = self.state.read(cx);
        if s.prefs.view != View::Memory || !s.prefs.side_open || !s.cur.loaded {
            return;
        }
        let key = (s.current, s.node_tick, s.cur.view.as_ref().map(|v| v.t).unwrap_or(0), s.cur.view.as_ref().map(|v| v.parts.len()).unwrap_or(0), s.open_seq);
        if key == self.seen {
            return;
        }
        if key.0 != self.seen.0 || key.4 != self.seen.4 {
            self.open.clear();
            self.kids.clear();
            self.hits = None;
            self.hit = None;
            self.rendered = None;
        }
        self.seen = key;
        let id = key.0;
        fetch(&self.state, cx, move |h| async move { (h.view_parts(id).await, h.cview_parts(id).await) }, |m, (v, cv), _| {
            m.texts = v.into_iter().map(|p| ((p.t, p.l, p.i), p.text.unwrap_or_default())).collect();
            m.cview = cv;
        });
        // opened rows whose children were not built yet refresh when nodes arrive
        let stale: Vec<(i64, u32, i64)> = self.open.iter().filter(|k| self.kids.get(k).map(|(_, ch)| ch.iter().any(|c| c.node.is_none())).unwrap_or(false)).cloned().collect();
        for k in stale {
            self.fetch_kids(k, cx);
        }
    }
    fn fetch_kids(&mut self, k: (i64, u32, i64), cx: &mut Context<Self>) {
        fetch(&self.state, cx, move |h| async move { h.node_children(k.0, k.1, k.2).await }, move |m, r, _| {
            m.kids.insert(k, r);
        });
    }
    fn toggle(&mut self, k: (i64, u32, i64), cx: &mut Context<Self>) {
        if !self.open.remove(&k) {
            self.open.insert(k);
            if !self.kids.contains_key(&k) {
                self.fetch_kids(k, cx);
            }
        }
        cx.notify();
    }
    fn do_search(&mut self, cx: &mut Context<Self>) {
        let q = self.search.read(cx).value().trim().to_string();
        if q.is_empty() {
            self.hits = None;
            cx.notify();
            return;
        }
        let id = self.state.read(cx).current;
        fetch(&self.state, cx, move |h| async move { h.search(id, &q).await }, |m, r, _| m.hits = Some(r));
    }
    /// open the path from the view's line down to message (t, i)
    fn reveal(&mut self, t: i64, i: i64, cx: &mut Context<Self>) {
        self.which_c = false;
        let s = self.state.read(cx);
        let Some(view) = s.cur.view.clone() else { return };
        let Some(top) = view.parts.iter().find(|p| p.t == t && p.i * (1i64 << p.l) <= i && i < (p.i + 1) * (1i64 << p.l)).cloned() else { return };
        let mut path = vec![];
        for l in (0..=top.l).rev() {
            path.push((t, l, i >> l));
        }
        for k in &path {
            self.open.insert(*k);
            if !self.kids.contains_key(k) {
                self.fetch_kids(*k, cx);
            }
        }
        self.hit = path.last().cloned();
        self.want_hit.set(true);
        cx.notify();
    }
    fn row(&self, t: i64, l: u32, i: i64, text: String, built: bool, depth: usize, cx: &mut Context<Self>) -> AnyElement {
        let k = c(cx).clone();
        let s = self.state.read(cx);
        let key = (t, l, i);
        let n = 1i64 << l;
        let start = i * n;
        let open = self.open.contains(&key);
        let foreign = t != s.current;
        let kk = format!("{t}:{l}:{i}");
        let busy = s.busy.contains(&kk);
        let failed = s.failed.get(&kk).cloned();
        let hover = k.hover;
        let mut out = div().flex().flex_col();
        out = out.child(
            div()
                .id(SharedString::from(format!("m-{kk}")))
                .flex()
                .items_start()
                .gap(px(6.))
                .py(px(3.))
                .pl(px(4. + depth as f32 * 12.))
                .pr(px(4.))
                .rounded(px(5.))
                .cursor_pointer()
                .text_size(px(12.))
                .line_height(px(17.))
                .when(self.hit == Some(key), |d| {
                    let (want, sh) = (self.want_hit.clone(), self.scroll.clone());
                    d.bg(k.accent_bg).child(
                        canvas(
                            move |b, window, _| {
                                if want.replace(false) {
                                    window.refresh();
                                    // put the row about a fifth of the way down the panel
                                    let top = sh.bounds().origin.y + sh.bounds().size.height * 0.2;
                                    let mut o = sh.offset();
                                    o.y -= b.origin.y - top;
                                    sh.set_offset(o);
                                }
                            },
                            |_, _, _, _| {},
                        )
                        .absolute()
                        .size_full(),
                    )
                })
                .when(open, |d| d.bg(k.panel))
                .hover(move |s| s.bg(hover))
                .tooltip(tooltip(if l == 0 { "open the message".to_string() } else { format!("zoom: open into two lines of {}", n / 2) }))
                .child(probe(format!("mem.{kk}")))
                .child(div().mt(px(3.)).child(chevron(format!("mem{kk}"), 10., k.faint, open)))
                .child(div().w(px(3.)).h(px(14.)).mt(px(1.)).rounded(px(2.)).flex_none().bg(k.levels[l as usize % 8]))
                .child(
                    div()
                        .flex_none()
                        .flex()
                        .gap(px(3.))
                        .font_family(crate::ui::MONO)
                        .text_size(px(10.5))
                        .text_color(k.muted)
                        .child(format!("{start}+{n}"))
                        .when(foreign, |d| d.child(div().px(px(3.)).rounded(px(3.)).bg(k.violet.opacity(0.15)).text_color(k.violet).child(format!("@{t}")))),
                )
                .child(div().flex_1().min_w_0().text_color(if built { k.text2 } else { k.faint }).child(text))
                .when(busy, |d| d.child(spinner(SharedString::from(format!("ms{kk}")), 11., k.accent)))
                .when_some(failed, |d, f| d.child(chip("retry", ChipTone::Red, cx).id(SharedString::from(format!("mf{kk}"))).tooltip(tooltip(f))))
                .on_click(cx.listener(move |m, _, _, cx| m.toggle(key, cx))),
        );
        if open {
            match self.kids.get(&key) {
                None => out = out.child(div().pl(px(20. + depth as f32 * 12.)).child(skeleton_line(Some(120.), cx))),
                Some((msg, kids)) => {
                    if let Some(m) = msg {
                        out = out.child(
                            div()
                                .ml(px(18. + depth as f32 * 12.))
                                .my(px(4.))
                                .p(px(8.))
                                .rounded(px(6.))
                                .bg(k.bg)
                                .border_1()
                                .border_color(k.line)
                                .text_size(px(12.))
                                .line_height(px(18.))
                                .child(div().font_family(crate::ui::MONO).text_size(px(10.5)).text_color(k.muted).mb(px(4.)).child(format!("{}message {} · {} · {}", if foreign { format!("#{t} · ") } else { String::new() }, m.i, m.kind.as_str(), fmt::datetime(&m.date))))
                                .child(div().text_color(k.text2).child(m.text.clone()))
                                .when_some(m.image.clone(), |d, r| d.child(div().mt(px(6.)).child(crate::ui::blob_thumb(&self.state, &r.blob, 260., 200., r.w.zip(r.h), cx)))),
                        );
                    }
                    let kids = kids.clone();
                    for x in kids {
                        let text = x.node.as_ref().map(|n| n.text.clone()).unwrap_or_else(|| match &x.message {
                            Some(m) => format!("{}: {} (not summarized yet)", m.kind.as_str(), m.text.chars().take(300).collect::<String>()),
                            None => "(not built yet)".into(),
                        });
                        out = out.child(self.row(x.t, x.l, x.i, text, x.node.is_some(), depth + 1, cx));
                    }
                }
            }
        }
        out.into_any_element()
    }
}
impl Render for MemoryPanel {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        // a cached view is laid out at the size its parent gives it, so the panel scrolls itself
        crate::ui::anim::area(crate::ui::anim::PANEL);
        let body = self.body(window, cx).into_any_element();
        div().id("scroll-MemoryPanel").size_full().overflow_y_scroll().track_scroll(&self.scroll).child(body)
    }
}
impl MemoryPanel {
    fn body(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        crate::ui::set_state_extra("memory", serde_json::json!({ "open": self.open.iter().map(|(t, l, i)| format!("{t}:{l}:{i}")).collect::<Vec<_>>(), "hit": self.hit.map(|(t, l, i)| format!("{t}:{l}:{i}")), "kids": self.kids.len() }));
        self.maybe_refresh(cx);
        let k = c(cx).clone();
        let s = self.state.read(cx);
        let Some(view) = s.cur.view.clone() else { return pad(div()).child(skeleton_line(None, cx)) };
        let task = s.current;
        let segs: Vec<i64> = s.tasks.get(&task).map(|t| t.task.segments.iter().map(|x| x.task).collect()).unwrap_or_default();
        let budget = view.high.max(1);
        let low = view.low;
        let mut levels: std::collections::BTreeMap<u32, usize> = Default::default();
        for p in &view.parts {
            *levels.entry(p.l).or_default() += 1;
        }
        let inherited = view.parts.iter().filter(|p| p.t != task).count();
        let in_timeline = |k: &String| {
            let t: i64 = k.split(':').next().and_then(|x| x.parse().ok()).unwrap_or(-1);
            t == task || segs.contains(&t)
        };
        let busy_all: Vec<String> = s.busy.iter().filter(|b| in_timeline(b)).cloned().collect();
        let failed: Vec<(String, String)> = s.failed.iter().filter(|(b, _)| in_timeline(b)).map(|(a, b)| (a.clone(), b.clone())).collect();
        let other = s.busy.len() - busy_all.len();
        let mut legend = div().flex().flex_wrap().gap(px(10.)).text_size(px(11.)).text_color(k.muted);
        for (l, n) in &levels {
            legend = legend.child(div().flex().items_center().gap(px(4.)).child(div().size(px(8.)).rounded(px(2.)).bg(k.levels[*l as usize % 8])).child(format!("{n} × {} msg", 1i64 << l)));
        }
        let summary = card(cx)
            .child(kv_row("messages", format!("{}{}", s.cur.t, if view.g > s.cur.t { format!(" own + {} inherited", view.g - s.cur.t) } else { String::new() }), cx))
            .child(kv_row("tree nodes built", s.cur.nodes.to_string(), cx))
            .child(kv_row("view", format!("{} lines{} · {}", view.parts.len(), if inherited > 0 { format!(" ({inherited} inherited)") } else { String::new() }, fmt::bytes(view.bytes)), cx))
            .child(kv_row("batches", format!("at {} → {}", fmt::bytes(budget), fmt::bytes(low)), cx))
            .child(kv_row("compactions' view", format!("{} lines · {} (16–32 KB)", view.compaction_lines, fmt::bytes(view.compaction_bytes)), cx))
            .child(div().relative().child(bar(&[(view.bytes as f32 / budget as f32, if view.batching { k.amber } else { k.accent })], cx)).child(div().absolute().left(relative(0.5)).top(px(6.)).h(px(10.)).w(px(1.)).bg(k.line2)))
            .when(view.batching, |d| d.child(div().text_size(px(11.5)).text_color(k.muted).child(format!("batch pending: merging toward {} as parents get built", fmt::bytes(low)))))
            .child(legend);
        if crate::ui::probes_on() {
            let built = if self.which_c { self.cview.iter().filter(|p| p.built).count() } else { view.parts.iter().filter(|p| p.built && self.texts.get(&(p.t, p.l, p.i)).map(|t| !t.is_empty()).unwrap_or(false)).count() };
            crate::ui::set_state_extra("memSummary", serde_json::json!({
                "messages": s.cur.t, "inherited": (view.g - s.cur.t).max(0), "nodes": s.cur.nodes, "lines": view.parts.len(), "inheritedLines": inherited, "bytes": view.bytes,
                "high": budget, "low": low, "batching": view.batching, "cviewLines": view.compaction_lines, "cviewBytes": view.compaction_bytes,
                "legend": levels.iter().map(|(l, n)| [*l as usize, *n]).collect::<Vec<_>>(),
                "running": busy_all.len(), "failed": failed.len(), "other": other, "whichC": self.which_c, "rows": if self.which_c { self.cview.len() } else { view.parts.len() },
                "builtRows": built, "raw": self.rendered.as_ref().map(|r| serde_json::json!({ "chars": js_len(&r.text), "blocks": r.blocks, "mark": r.mark_index, "marked": r.marked_chars, "header": r.header.len() })),
            }));
        }
        let mut jobs = div().flex().flex_wrap().gap(px(6.));
        if busy_all.is_empty() && failed.is_empty() {
            jobs = jobs.child(div().text_size(px(12.)).text_color(k.muted).child("nothing being summarized in this timeline"));
        }
        for b in &busy_all {
            let v: Vec<i64> = b.split(':').map(|x| x.parse().unwrap_or(0)).collect();
            let (t, l, i) = (v[0], v[1] as u32, v[2]);
            jobs = jobs.child(
                div()
                    .id(SharedString::from(format!("job{b}")))
                    .flex()
                    .items_center()
                    .gap(px(5.))
                    .px(px(7.))
                    .py(px(2.))
                    .rounded(px(6.))
                    .border_1()
                    .border_color(k.line2)
                    .text_size(px(11.5))
                    .cursor_pointer()
                    .child(spinner(SharedString::from(format!("js{b}")), 11., k.accent))
                    .child(probe(format!("job.{b}")))
                    .child(if l == 0 { format!("message {i}") } else { format!("{}+{}", i << l, 1i64 << l) })
                    .when(t != task, |d| d.child(div().text_color(k.violet).child(format!("@{t}"))))
                    .on_click(cx.listener(move |m, _, _, cx| m.reveal(t, i << l, cx))),
            );
        }
        for (b, e) in &failed {
            jobs = jobs.child(chip(format!("{b} failed · retrying"), ChipTone::Red, cx).id(SharedString::from(format!("jf{b}"))).child(probe(format!("jobf.{b}"))).tooltip(tooltip(e.clone())));
        }
        let mut hits_el = div().flex().flex_col();
        if let Some(h) = &self.hits {
            if h.is_empty() {
                hits_el = hits_el.child(div().text_size(px(12.)).text_color(k.muted).child("no matches"));
            }
            for x in h.iter().rev().take(60) {
                let (t, i) = (x.t, x.i);
                hits_el = hits_el.child(
                    div()
                        .id(SharedString::from(format!("hit{t}:{i}")))
                        .flex()
                        .gap(px(6.))
                        .py(px(3.))
                        .px(px(4.))
                        .rounded(px(5.))
                        .text_size(px(12.))
                        .cursor_pointer()
                        .hover(|s| s.bg(k.hover))
                        .child(probe(format!("hit.{t}.{i}")))
                        .child(div().flex_none().font_family(crate::ui::MONO).text_color(k.muted).child(format!("{i}{}", if t != task { format!("@{t}") } else { String::new() })))
                        .child(div().flex_none().px(px(4.)).rounded(px(3.)).bg(k.panel2).text_size(px(10.5)).text_color(k.muted).child(x.kind.clone()))
                        .child(div().flex_1().min_w_0().overflow_hidden().line_clamp(1).text_ellipsis().text_color(k.text2).child(x.text.replace('\n', " ").chars().take(160).collect::<String>()))
                        .on_click(cx.listener(move |m, _, _, cx| m.reveal(t, i, cx))),
                );
            }
        }
        let rows: Vec<(i64, u32, i64, String, bool)> = if self.which_c {
            self.cview.iter().map(|p| (p.t, p.l, p.i, p.text.clone().unwrap_or_default(), p.built)).collect()
        } else {
            view.parts.iter().map(|p| (p.t, p.l, p.i, self.texts.get(&(p.t, p.l, p.i)).cloned().unwrap_or_else(|| if p.built { "…".into() } else { "(not summarized yet)".into() }), p.built)).collect()
        };
        let mut tree = div().flex().flex_col();
        if rows.is_empty() {
            tree = tree.child(div().text_size(px(12.)).text_color(k.muted).child(if self.which_c { "loading…" } else { "empty: no messages yet" }));
        }
        for (t, l, i, text, built) in rows {
            tree = tree.child(self.row(t, l, i, text, built, 0, cx));
        }
        let which_c = self.which_c;
        pad(div())
            .child(probe("memory"))
            .child(section(cx).child(section_title(format!("memory of {}", if task == 0 { "the main chat".to_string() } else { format!("#{task}") }), cx)).child(summary))
            .child(section(cx).child(section_title(format!("compaction jobs · {}{}", if busy_all.is_empty() { "idle".to_string() } else { format!("{} running", busy_all.len()) }, if other > 0 { format!(" · {other} in other tasks") } else { String::new() }), cx)).child(jobs))
            .child(section(cx).child(div().flex().gap(px(6.)).child(div().flex_1().child(Input::new(&self.search).small()).child(probe("mem.search"))).child(btn_small("msearch", "Search", Tone::Plain, cx).on_click(cx.listener(|m, _, _, cx| m.do_search(cx))))).child(div().mt(px(6.)).child(hits_el)))
            .child(
                section(cx)
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .gap(px(4.))
                            .mb(px(8.))
                            .child(seg("seg-view", "the view", !which_c, cx).child(probe("mem.view")).on_click(cx.listener(|m, _, _, cx| { m.which_c = false; cx.notify(); })))
                            .child(seg("seg-cview", "compactions' view", which_c, cx).child(probe("mem.cview")).on_click(cx.listener(|m, _, _, cx| {
                                m.which_c = true;
                                let id = m.state.read(cx).current;
                                fetch(&m.state, cx, move |h| async move { h.cview_parts(id).await }, |m, cv, _| m.cview = cv);
                                cx.notify();
                            })))
                            .child(div().flex_1())
                            .child(btn_small("mraw", if self.rendered.is_some() { "hide raw" } else { "raw" }, Tone::Ghost, cx).child(probe("mem.raw")).on_click(cx.listener(|m, _, _, cx| {
                                if m.rendered.is_some() {
                                    m.rendered = None;
                                    cx.notify();
                                } else {
                                    let id = m.state.read(cx).current;
                                    fetch(&m.state, cx, move |h| async move { h.render(id).await }, |m, r, _| m.rendered = r);
                                }
                            }))),
                    )
                    .when_some(self.rendered.clone(), |d, r| {
                        d.child(
                            div()
                                .id("rawview")
                                .mb(px(10.))
                                .p(px(10.))
                                .rounded(px(8.))
                                .bg(k.code_bg)
                                .max_h(px(360.))
                                .overflow_y_scroll()
                                .font_family(crate::ui::MONO)
                                .text_size(px(11.))
                                .text_color(k.text2)
                                .child(format!("{} chars · {} blocks of 4 lines · cache mark on block {} (first {} chars)\n\n{}\n\n{}", js_len(&r.text), r.blocks, r.mark_index + 1, r.marked_chars, r.text, r.header)),
                        )
                    })
                    .child(tree),
            )
    }
}

// =====================================================================================================================
// Context debugger
// =====================================================================================================================
pub struct ContextPanel {
    state: Entity<AppState>,
    detail: Option<RequestDetail>,
    loading: Option<(String, String)>,
    err: Option<String>,
    open: HashSet<String>,
    wire: bool,
    _subs: Vec<Subscription>,
}
impl ContextPanel {
    pub fn new(state: Entity<AppState>, cx: &mut Context<Self>) -> ContextPanel {
        let s = cx.observe(&state, |p: &mut ContextPanel, _, cx| { p.maybe(cx); cx.notify(); });
        ContextPanel { state, detail: None, loading: None, err: None, open: HashSet::new(), wire: false, _subs: vec![s] }
    }
    fn maybe(&mut self, cx: &mut Context<Self>) {
        let s = self.state.read(cx);
        if s.prefs.view != View::Context {
            return;
        }
        let Some(id) = s.shown_request() else {
            self.detail = None;
            return;
        };
        let status = s.cur.requests.get(&id).map(|r| r.status.clone()).unwrap_or_default();
        let key = (id.clone(), status);
        if self.loading.as_ref() == Some(&key) {
            return;
        }
        self.loading = Some(key);
        fetch(&self.state, cx, move |h| async move { h.request_detail(&id).await }, |p, d, _| {
            p.err = if d.is_none() { Some("no such request".into()) } else { None };
            p.detail = d;
        });
    }
    fn msg_row(&self, m: &Message, indent: bool, cx: &mut Context<Self>) -> AnyElement {
        let k = c(cx).clone();
        let open = self.open.contains(&m.id);
        let is_img = matches!(m.body, Body::Image { .. });
        let id = m.id.clone();
        let (kind, prev, full) = msg_texts(m);
        let rcol = match m.role {
            Role::System => k.amber,
            Role::Developer => k.amber,
            Role::User => k.accent,
            Role::Assistant => k.green,
        };
        div()
            .flex()
            .flex_col()
            .when(indent, |d| d.ml(px(14.)))
            .child(
                div()
                    .id(SharedString::from(format!("cm{}", m.id)))
                    .flex()
                    .items_center()
                    .gap(px(6.))
                    .py(px(3.))
                    .px(px(4.))
                    .rounded(px(5.))
                    .cursor_pointer()
                    .text_size(px(12.))
                    .hover(|s| s.bg(k.hover))
                    .child(chevron(format!("cm{}", m.id), 10., k.faint, open))
                    .child(div().flex_none().w(px(62.)).text_color(rcol).child(format!("{:?}", m.role).to_lowercase()))
                    .child(div().flex_none().w(px(70.)).text_color(k.muted).text_size(px(11.)).child(kind))
                    .child(div().flex_1().min_w_0().overflow_hidden().line_clamp(1).text_ellipsis().text_color(k.text2).child(prev))
                    .child(div().flex_none().text_size(px(10.5)).text_color(k.faint).child(format!("{}{}", fmt::bytes(m.bytes as usize), if m.raw.is_some() { " · raw" } else { "" })))
                    .when(is_img, |d| d.child(probe(format!("cmimg.{}", m.id))))
                    .on_click(cx.listener(move |p, _, _, cx| {
                        if !p.open.remove(&id) {
                            p.open.insert(id.clone());
                        }
                        cx.notify();
                    })),
            )
            .when(open, |d| {
                let image = match &m.body {
                    Body::Image { blob: Some(b), .. } => Some(crate::ui::blob_thumb(&self.state, b, 260., 200., None, cx)),
                    _ => None,
                };
                d.child(div().ml(px(18.)).mb(px(6.)).p(px(8.)).rounded(px(6.)).bg(k.code_bg).font_family(crate::ui::MONO).text_size(px(11.5)).line_height(px(17.)).text_color(k.text2).child(full).when_some(image, |d, i| d.child(div().mt(px(6.)).child(i).child(probe("ctx.thumb")))))
            })
            .into_any_element()
    }
}
fn msg_texts(m: &Message) -> (String, String, String) {
    let one = |s: &str| s.chars().take(160).collect::<String>().split_whitespace().collect::<Vec<_>>().join(" ");
    match &m.body {
        Body::Text { text } => ("text".into(), one(text), text.chars().take(20000).collect()),
        Body::Reasoning { text, sealed } => {
            let t = text.clone().unwrap_or_default();
            let p = if !t.is_empty() { t.clone() } else if sealed.as_ref().and_then(|s| s.encrypted.as_ref()).is_some() { "[encrypted reasoning]".into() } else { "[signed thinking]".into() };
            let mut f = vec![t];
            if let Some(s) = sealed {
                if let Some(sig) = &s.signature {
                    f.push(format!("signature: {} chars", sig.len()));
                }
                if let Some(e) = &s.encrypted {
                    f.push(format!("encrypted: {} chars", e.len()));
                }
            }
            ("reasoning".into(), one(&p), f.into_iter().filter(|x| !x.is_empty()).collect::<Vec<_>>().join("\n\n"))
        }
        Body::ToolCall { call_id, name, input, .. } => {
            let s = match input {
                serde_json::Value::String(s) => s.clone(),
                v => v.to_string(),
            };
            ("tool_call".into(), one(&format!("{name} {s}")), format!("{name} ({call_id})\n{}", serde_json::to_string_pretty(input).unwrap_or_default()))
        }
        Body::ToolResult { call_id, content, is_error, .. } => {
            let t = optchat_proto::api::result_text(content);
            ("tool_result".into(), format!("{}{}", if *is_error { "ERROR " } else { "" }, one(&t)), format!("{call_id}{}\n{t}", if *is_error { " (error)" } else { "" }))
        }
        Body::Image { media_type, .. } => ("image".into(), format!("[image {media_type}]"), format!("[image {media_type}]")),
        Body::File { name, .. } => ("file".into(), format!("[file {name}]"), name.clone()),
        Body::Envelope { children } => ("envelope".into(), format!("{} items", children.len()), String::new()),
    }
}
impl Render for ContextPanel {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        // a cached view is laid out at the size its parent gives it, so the panel scrolls itself
        crate::ui::anim::area(crate::ui::anim::PANEL);
        let body = self.body(window, cx).into_any_element();
        crate::ui::scroll_body("ContextPanel", body)
    }
}
impl ContextPanel {
    fn body(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        self.maybe(cx);
        let k = c(cx).clone();
        let s = self.state.read(cx);
        let shown = s.shown_request();
        if crate::ui::probes_on() {
            let running = shown.as_ref().and_then(|r| s.cur.requests.get(r)).map(|r| r.status == "running").unwrap_or(false);
            let d = self.detail.as_ref().filter(|d| Some(&d.request.id) == shown.as_ref());
            let all: Vec<&Message> = d.map(|d| d.context.iter().flat_map(|x| match &x.3 { Some(ch) => ch.iter().collect::<Vec<_>>(), None => vec![&x.2] }).collect()).unwrap_or_default();
            let imgs = all.iter().filter(|m| matches!(m.body, Body::Image { .. })).count();
            let mut open: Vec<&String> = self.open.iter().collect();
            open.sort();
            crate::ui::set_state_extra("context", serde_json::json!({ "wire": self.wire, "pinned": s.pinned_request, "shown": shown, "stillRunning": running && d.is_none(), "rows": all.len(), "imageRows": imgs, "open": open }));
            if running && d.is_none() {
                crate::ui::seen("contextSeen", "stillRunning");
            }
        }
        let Some(rid) = shown else { return pad(div()).child(div().text_size(px(12.)).text_color(k.muted).child("Pick a model call (the “context ↗” or “steps” chips under a turn, or the Log view) to see exactly what was sent.")) };
        let status = s.cur.requests.get(&rid).map(|r| r.status.clone());
        let Some(d) = self.detail.clone().filter(|d| d.request.id == rid) else {
            if status.as_deref() == Some("running") {
                return pad(div()).child(div().text_size(px(12.)).text_color(k.muted).child("This model call is still running: its context shows here once its response is in."));
            }
            if let Some(e) = &self.err {
                return pad(div()).child(div().text_color(k.red).child(e.clone()));
            }
            return pad(div()).child(div().text_size(px(12.)).text_color(k.muted).child("loading…"));
        };
        let r = d.request.clone();
        let u = r.usage.clone().unwrap_or_default();
        let total = u.input.max(0);
        let uncached = (total - u.cache_read - u.cache_write).max(0);
        let f = |x: i64| if total > 0 { x as f32 / total as f32 } else { 0.0 };
        let ctx_bytes: u64 = d.context.iter().map(|x| x.2.bytes + x.3.as_ref().map(|c| c.iter().map(|m| m.bytes).sum::<u64>()).unwrap_or(0)).sum();
        let cost = r.cost.clone();
        let cs = |v: f64| fmt::cost(Some(&Cost { total: v, priced: cost.as_ref().map(|c| c.priced).unwrap_or(true), ..Default::default() }));
        let mut head = div().flex().flex_wrap().gap(px(6.)).mb(px(8.));
        head = head
            .child(chip(format!("{}{}", r.model, r.effort.as_ref().map(|e| format!(" · {e}")).unwrap_or_default()), if r.provider == Provider::Anthropic { ChipTone::Acc } else { ChipTone::Green }, cx))
            .child(chip(r.backend.as_str(), ChipTone::Plain, cx))
            .child(chip(format!("{}{}", r.status, r.stop.as_ref().map(|s| format!(" · {s}")).unwrap_or_default()), if r.status == "running" { ChipTone::Amber } else if r.status == "failed" { ChipTone::Red } else { ChipTone::Plain }, cx))
            .child(chip(fmt::cost(r.cost.as_ref()), ChipTone::Plain, cx));
        let legend = div()
            .flex()
            .flex_wrap()
            .gap(px(10.))
            .text_size(px(11.))
            .text_color(k.muted)
            .child(div().flex().gap(px(4.)).items_center().child(div().size(px(8.)).rounded(px(2.)).bg(k.green)).child(format!("cache read {}", fmt::tokens(u.cache_read))))
            .child(div().flex().gap(px(4.)).items_center().child(div().size(px(8.)).rounded(px(2.)).bg(k.amber)).child(format!("cache write {}", fmt::tokens(u.cache_write))))
            .child(div().flex().gap(px(4.)).items_center().child(div().size(px(8.)).rounded(px(2.)).bg(k.accent)).child(format!("uncached {}", fmt::tokens(uncached))))
            .child(format!("output {}{}", fmt::tokens(u.output), u.reasoning.filter(|x| *x > 0).map(|x| format!(" (reasoning {})", fmt::tokens(x))).unwrap_or_default()));
        let summary = card(cx)
            .child(head)
            .child(bar(&[(f(u.cache_read), k.green), (f(u.cache_write), k.amber), (f(uncached), k.accent)], cx))
            .child(legend)
            .child(
                div()
                    .mt(px(10.))
                    .child(kv_row("input tokens", format!("{} · {}% cached", fmt::tokens(total), fmt::cache_pct(Some(&u))), cx))
                    .child(kv_row("cost", match &cost { Some(c) => format!("in {} · read {} · write {} · out {}", cs(c.input), cs(c.cache_read), cs(c.cache_write), cs(c.output)), None => "—".into() }, cx))
                    .child(kv_row("first token", Some(fmt::dur(Some(&r.started_at), r.first_token_at.as_deref())).filter(|s| !s.is_empty()).unwrap_or("—".into()), cx))
                    .child(kv_row("total time", Some(fmt::dur(Some(&r.started_at), r.done_at.as_deref())).filter(|s| !s.is_empty()).unwrap_or("—".into()), cx))
                    .child(kv_row("started", fmt::time(&r.started_at), cx))
                    .child(kv_row("canonical context", format!("{} items · {}", d.context.len(), fmt::bytes(ctx_bytes as usize)), cx)),
            );
        let mut ctx = div().flex().flex_col();
        let mut prev_ctx: Option<String> = None;
        for (cid, mark, m, children) in d.context.iter() {
            if let Some(p) = &prev_ctx {
                if p != cid {
                    ctx = ctx.child(div().my(px(4.)).text_size(px(10.5)).text_color(k.faint).font_family(crate::ui::MONO).child(format!("context {}", &cid[..6.min(cid.len())])));
                }
            }
            prev_ctx = Some(cid.clone());
            match children {
                Some(ch) => {
                    ctx = ctx.child(div().my(px(4.)).text_size(px(11.5)).text_color(k.muted).child(format!("model output ({} items)", ch.len())));
                    for c2 in ch {
                        ctx = ctx.child(self.msg_row(c2, true, cx));
                    }
                }
                None => ctx = ctx.child(self.msg_row(m, false, cx)),
            }
            if *mark {
                ctx = ctx.child(div().my(px(3.)).border_t_1().border_dashed().border_color(k.amber).pt(px(2.)).text_size(px(10.5)).text_color(k.amber).child("cache breakpoint"));
            }
        }
        let mut out = div().flex().flex_col();
        if d.output.is_empty() {
            out = out.child(div().text_size(px(12.)).text_color(k.muted).child(if r.status == "running" { "streaming…" } else { "no output" }));
        }
        for m in d.output.iter() {
            out = out.child(self.msg_row(m, false, cx));
        }
        let wire_txt = d.wire.as_ref().map(|w| serde_json::to_string_pretty(w).unwrap_or_default());
        pad(div())
            .child(probe("context"))
            .child(section(cx).child(section_title(format!("{} {}", if r.purpose == "compact" { "compactor call" } else { "model call" }, &r.id[..8.min(r.id.len())]), cx)).child(summary))
            .child(section(cx).child(section_title("context sent (canonical, oldest first)", cx)).child(ctx))
            .child(section(cx).child(section_title("output", cx)).child(out))
            .child(
                section(cx)
                    .child(div().flex().items_center().gap(px(8.)).child(section_title("wire request", cx)).child(btn_small("wire", if self.wire { "hide" } else { "show" }, Tone::Ghost, cx).child(probe("ctx.wire")).on_click(cx.listener(|p, _, _, cx| { p.wire = !p.wire; cx.notify(); }))))
                    .child(if self.wire {
                        match wire_txt {
                            Some(w) => div().p(px(8.)).rounded(px(6.)).bg(k.code_bg).font_family(crate::ui::MONO).text_size(px(11.)).text_color(k.text2).child(w.chars().take(200_000).collect::<String>()),
                            None => div().text_size(px(12.)).text_color(k.muted).child("not captured for this call"),
                        }
                    } else {
                        div().text_size(px(12.)).text_color(k.muted).child(if d.wire.is_some() { "Exact body the local agent sent upstream (Claude: after OptChat's cache-marker layout; Codex: thread items)." } else { "not captured" })
                    }),
            )
    }
}

// =====================================================================================================================
// Log: breakdown + requests
// =====================================================================================================================
pub struct LogPanel {
    state: Entity<AppState>,
    subtree: bool,
    filter: &'static str,
    sub: Option<Vec<RequestRecord>>,
    bd: Option<Breakdown>,
    seen: (i64, bool, usize, u64),
    _subs: Vec<Subscription>,
}
impl LogPanel {
    pub fn new(state: Entity<AppState>, cx: &mut Context<Self>) -> LogPanel {
        let s = cx.observe(&state, |p: &mut LogPanel, _, cx| { p.maybe(cx); cx.notify(); });
        LogPanel { state, subtree: false, filter: "all", sub: None, bd: None, seen: (-1, false, 0, 0), _subs: vec![s] }
    }
    fn maybe(&mut self, cx: &mut Context<Self>) {
        let s = self.state.read(cx);
        if s.prefs.view != View::Log {
            return;
        }
        let key = (s.current, self.subtree, s.cur.requests.len(), s.open_seq);
        if key == self.seen {
            return;
        }
        self.seen = key;
        let (id, sub) = (key.0, key.1);
        fetch(&self.state, cx, move |h| async move { (h.breakdown(id, sub).await, if sub { Some(h.requests_recent(id, true, 400).await) } else { None }) }, |p, (b, r), _| {
            p.bd = Some(b);
            p.sub = r;
        });
    }
    fn table(&self, title: &str, g: &[(String, HashMap<String, i64>)], label: &dyn Fn(&str) -> String, cx: &App) -> Div {
        let k = c(cx);
        let total: i64 = g.iter().map(|(_, v)| *v.get("usd").unwrap_or(&0)).sum::<i64>().max(1);
        let mut t = div().flex().flex_col().text_size(px(12.)).mb(px(8.));
        t = t.child(div().text_size(px(10.5)).text_color(k.muted).mb(px(4.)).child(title.to_uppercase()));
        t = t.child(div().flex().text_size(px(10.5)).text_color(k.faint).child(div().flex_1()).child(div().w(px(70.)).flex().justify_end().child("cost")).child(div().w(px(64.)).flex().justify_end().child("tokens")).child(div().w(px(64.)).flex().justify_end().child("time")));
        for (key, v) in g {
            let usd = *v.get("usd").unwrap_or(&0);
            let tok: i64 = ["tokens.in", "tokens.cache_read", "tokens.cache_write", "tokens.out"].iter().map(|m| *v.get(*m).unwrap_or(&0)).sum();
            t = t.child(
                div()
                    .flex()
                    .items_center()
                    .py(px(3.))
                    .child(div().flex_1().min_w_0().relative().child(div().absolute().left_0().top_0().bottom_0().w(relative((usd as f32 / total as f32).max(0.02))).bg(k.accent_bg).rounded(px(3.))).child(div().relative().px(px(4.)).overflow_hidden().line_clamp(1).text_ellipsis().text_color(k.text2).child(label(key))))
                    .child(div().w(px(70.)).flex().justify_end().child(fmt::usd(usd as f64 / 1e6)))
                    .child(div().w(px(64.)).flex().justify_end().text_color(k.muted).child(fmt::tokens(tok)))
                    .child(div().w(px(64.)).flex().justify_end().text_color(k.muted).child(fmt::ms(*v.get("time.model").unwrap_or(&0)))),
            );
        }
        t
    }
}
impl Render for LogPanel {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        // a cached view is laid out at the size its parent gives it, so the panel scrolls itself
        crate::ui::anim::area(crate::ui::anim::PANEL);
        let body = self.body(window, cx).into_any_element();
        crate::ui::scroll_body("LogPanel", body)
    }
}
impl LogPanel {
    fn body(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        self.maybe(cx);
        let k = c(cx).clone();
        let s = self.state.read(cx);
        let info = s.info().cloned();
        let all: Vec<RequestRecord> = if self.subtree { self.sub.clone().unwrap_or_default() } else { s.cur.requests.values().cloned().collect() };
        let mut rows: Vec<RequestRecord> = all.iter().filter(|r| self.filter == "all" || r.purpose == self.filter).cloned().collect();
        rows.sort_by(|a, b| b.started_at.cmp(&a.started_at));
        rows.truncate(300);
        if crate::ui::probes_on() {
            let mut purposes: Vec<&str> = rows.iter().map(|r| r.purpose.as_str()).collect();
            purposes.sort();
            purposes.dedup();
            let bd = self.bd.as_ref().map(|b| serde_json::json!({ "byKind": b.by_kind.iter().map(|x| x.0.clone()).collect::<Vec<_>>(), "byModel": b.by_model.len(), "bySource": b.by_source.iter().map(|x| x.0.clone()).collect::<Vec<_>>(), "events": b.events }));
            crate::ui::set_state_extra("log", serde_json::json!({ "subtree": self.subtree, "filter": self.filter, "rows": rows.len(), "purposes": purposes, "breakdown": bd, "first": rows.first().map(|r| r.id.clone()) }));
        }
        let (mut chat, mut comp, mut inp, mut read, mut out) = (0.0, 0.0, 0i64, 0i64, 0i64);
        for r in &all {
            let ct = r.cost.as_ref().map(|c| c.total).unwrap_or(0.0);
            if r.purpose == "chat" { chat += ct } else if r.purpose == "compact" { comp += ct }
            if let Some(u) = &r.usage {
                inp += u.input;
                read += u.cache_read;
                out += u.output;
            }
        }
        let selected = s.shown_request();
        let bd_el = match &self.bd {
            None => div().child(skeleton_line(None, cx)).child(skeleton_line(Some(200.), cx)),
            Some(b) => div()
                .child(section_title(format!("breakdown · {} ledger events", b.events), cx))
                .child(self.table("by kind", &b.by_kind, &|k| match k { "compact" => "memory (compactions)".into(), "chat" => "chat (turns)".into(), x => x.into() }, cx))
                .child(self.table("by model", &b.by_model, &|k| k.replace("claude-", ""), cx))
                .child(self.table("by source", &b.by_source, &|k| k.to_string(), cx)),
        };
        let usd_ledger = if self.subtree { sub(info.as_ref(), "usd") } else { own(info.as_ref(), "usd") };
        let totals = card(cx)
            .child(kv_row("cost (ledger)", fmt::usd(usd_ledger as f64 / 1e6), cx))
            .child(kv_row("turns (loaded)", fmt::usd(chat), cx))
            .child(kv_row("compactions (loaded)", fmt::usd(comp), cx))
            .child(kv_row("input tokens", format!("{} · {}% cached", fmt::tokens(inp), if inp > 0 { ((100 * read) as f64 / inp as f64).round() as i64 } else { 0 }), cx))
            .child(kv_row("output tokens", fmt::tokens(out), cx))
            .child(div().mt(px(8.)).text_size(px(11.)).text_color(k.muted).child("API-equivalent estimates; local agents run on your subscription. Claude prices are calibrated from Claude Code's own cost report."));
        let mut table = div().flex().flex_col().text_size(px(12.));
        table = table.child(div().flex().text_size(px(10.5)).text_color(k.faint).py(px(3.)).child(div().w(px(64.)).child("when")).when(self.subtree, |d| d.child(div().w(px(36.)).child("task"))).child(div().flex_1().child("model")).child(div().w(px(48.)).flex().justify_end().child("in")).child(div().w(px(40.)).flex().justify_end().child("cache")).child(div().w(px(44.)).flex().justify_end().child("out")).child(div().w(px(58.)).flex().justify_end().child("cost")));
        for (n, r) in rows.into_iter().enumerate() {
            let rid = r.id.clone();
            let sel = selected.as_deref() == Some(&r.id);
            let st = self.state.clone();
            table = table.child(
                div()
                    .id(SharedString::from(format!("lr{}", r.id)))
                    .flex()
                    .items_center()
                    .py(px(3.))
                    .rounded(px(4.))
                    .cursor_pointer()
                    .when(sel, |d| d.bg(k.accent_bg))
                    .hover(|s| s.bg(k.hover))
                    .text_color(k.text2)
                    .child(div().w(px(64.)).child(format!("{}{}", fmt::time(&r.started_at), if r.purpose == "compact" { " · c" } else { "" })))
                    .when(self.subtree, |d| d.child(div().w(px(36.)).font_family(crate::ui::MONO).child(format!("#{}", r.task.unwrap_or(0)))))
                    .child(div().flex_1().min_w_0().overflow_hidden().line_clamp(1).text_ellipsis().child(format!("{}{}", r.model.replace("claude-", ""), if r.status == "running" { " …" } else { "" })))
                    .child(div().w(px(48.)).flex().justify_end().child(fmt::tokens(r.usage.as_ref().map(|u| u.input).unwrap_or(0))))
                    .child(div().w(px(40.)).flex().justify_end().child(format!("{}%", fmt::cache_pct(r.usage.as_ref()))))
                    .child(div().w(px(44.)).flex().justify_end().child(fmt::tokens(r.usage.as_ref().map(|u| u.output).unwrap_or(0))))
                    .child(div().w(px(58.)).flex().justify_end().child(fmt::cost(r.cost.as_ref())))
                    .when(n < 5, |d| d.child(probe(format!("logrow.{n}"))))
                    .on_click(move |_, _, cx| {
                        let rid = rid.clone();
                        st.update(cx, |s, cx| {
                            s.selected_request = Some(rid);
                            s.pinned_request = true;
                            s.prefs.view = View::Context;
                            s.prefs.side_open = true;
                            cx.notify();
                        })
                    }),
            );
        }
        let filters = ["all", "chat", "compact"];
        let mut fl = div().flex().gap(px(4.)).mb(px(8.));
        for f in filters {
            fl = fl.child(seg(format!("lf{f}"), f, self.filter == f, cx).child(probe(format!("log.f.{f}"))).on_click(cx.listener(move |p, _, _, cx| {
                p.filter = f;
                cx.notify();
            })));
        }
        pad(div())
            .child(probe("log"))
            .child(div().flex().gap(px(4.)).mb(px(10.)).child(seg("ls-task", "this task", !self.subtree, cx).on_click(cx.listener(|p, _, _, cx| { p.subtree = false; cx.notify(); }))).child(seg("ls-sub", "with its subtree", self.subtree, cx).child(probe("log.subtree")).on_click(cx.listener(|p, _, _, cx| { p.subtree = true; cx.notify(); }))))
            .child(section(cx).child(bd_el))
            .child(section(cx).child(totals))
            .child(fl)
            .child(table)
    }
}

// =====================================================================================================================
// Models
// =====================================================================================================================
pub struct ModelsPanel {
    state: Entity<AppState>,
    rows: Option<Vec<ModelRow>>,
    show_hidden: bool,
    edit: Option<(String, [Entity<InputState>; 6])>,
    hist: HashSet<String>,
    seen: (usize, u64),
    reloads: u64,
    _subs: Vec<Subscription>,
}
impl ModelsPanel {
    pub fn new(state: Entity<AppState>, _window: &mut Window, cx: &mut Context<Self>) -> ModelsPanel {
        let s = cx.observe(&state, |p: &mut ModelsPanel, _, cx| { p.maybe(cx); cx.notify(); });
        ModelsPanel { state, rows: None, show_hidden: false, edit: None, hist: HashSet::new(), seen: (usize::MAX, 0), reloads: 0, _subs: vec![s] }
    }
    fn maybe(&mut self, cx: &mut Context<Self>) {
        let s = self.state.read(cx);
        if s.prefs.view != View::Models {
            return;
        }
        let key = (s.models.len() * 1000 + s.models.iter().filter(|m| m.priced).count(), s.notices.len() as u64 + s.server_log.len() as u64);
        if key == self.seen {
            return;
        }
        self.seen = key;
        self.reload(cx);
    }
    fn reload(&mut self, cx: &mut Context<Self>) {
        fetch(&self.state, cx, |h| async move { h.models_list().await }, |p, r, _| {
            p.rows = Some(r);
            p.reloads += 1;
        });
    }
    fn start_edit(&mut self, r: &ModelRow, window: &mut Window, cx: &mut Context<Self>) {
        let p = r.rate.as_ref().map(|r| r.per1m.clone());
        let v = |x: Option<f64>| x.map(|n| format!("{n}")).unwrap_or_default();
        let vals = [v(p.as_ref().map(|p| p.input)), v(p.as_ref().map(|p| p.cache_read)), v(p.as_ref().map(|p| p.cache_write)), v(p.as_ref().and_then(|p| p.cache_write_1h)), v(p.as_ref().map(|p| p.out)), String::new()];
        let anthropic = r.cat.provider == "anthropic";
        let inputs: [Entity<InputState>; 6] = std::array::from_fn(|i| {
            let val = vals[i].clone();
            cx.new(|cx| InputState::new(window, cx).default_value(val).placeholder(if i == 5 { "note (where the price comes from)" } else { "" }))
        });
        if anthropic {
            // Anthropic's fixed ratios follow the base input price
            let deps = inputs.clone();
            let s = cx.subscribe_in(&inputs[0], window, move |_: &mut ModelsPanel, inp, e: &InputEvent, window, cx| {
                if let InputEvent::Change = e {
                    if let Ok(b) = inp.read(cx).value().trim().parse::<f64>() {
                        if b > 0.0 {
                            let set = |i: usize, v: f64, window: &mut Window, cx: &mut Context<ModelsPanel>| deps[i].update(cx, |x, cx| x.set_value(format!("{}", (v * 1e6).round() / 1e6), window, cx));
                            set(1, b * 0.1, window, cx);
                            set(2, b * 1.25, window, cx);
                            set(3, b * 2.0, window, cx);
                            set(4, b * 5.0, window, cx);
                        }
                    }
                }
            });
            self._subs.push(s);
        }
        self.edit = Some((r.cat.id.clone(), inputs));
        cx.notify();
    }
    fn save_edit(&mut self, cx: &mut Context<Self>) {
        let Some((model, inputs)) = self.edit.take() else { return };
        let g = |i: usize, cx: &App| inputs[i].read(cx).value().trim().to_string();
        let num = |s: String| s.parse::<f64>().unwrap_or(f64::NAN);
        let per = Per1M { input: num(g(0, cx)), cache_read: num(g(1, cx)), cache_write: num(g(2, cx)), cache_write_1h: Some(g(3, cx)).filter(|s| !s.is_empty()).map(num), out: num(g(4, cx)) };
        let note = g(5, cx);
        self.state.update(cx, |s, cx| s.command(cx, true, move |h| async move { h.set_rate(&model, per, &note).await }));
        cx.notify();
    }
}
impl Render for ModelsPanel {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        // a cached view is laid out at the size its parent gives it, so the panel scrolls itself
        crate::ui::anim::area(crate::ui::anim::PANEL);
        let body = self.body(window, cx).into_any_element();
        crate::ui::scroll_body("ModelsPanel", body)
    }
}
impl ModelsPanel {
    fn body(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        self.maybe(cx);
        let k = c(cx).clone();
        let Some(rows) = self.rows.clone() else { return pad(div()).child(skeleton_line(None, cx)).child(skeleton_line(Some(200.), cx)) };
        let hidden = rows.iter().filter(|r| r.cat.hidden.unwrap_or(false)).count();
        let runnable = rows.iter().filter(|r| r.available && !r.cat.hidden.unwrap_or(false)).count();
        if crate::ui::probes_on() {
            let mut providers: Vec<String> = rows.iter().map(|r| r.cat.provider.clone()).collect();
            providers.dedup();
            let shown: Vec<String> = rows.iter().filter(|r| self.show_hidden || !r.cat.hidden.unwrap_or(false)).map(|r| r.cat.id.clone()).collect();
            let haiku = rows.iter().find(|r| r.cat.id == "claude-haiku-5-5").map(|r| serde_json::json!({ "rate": r.rate.as_ref().map(|x| x.per1m.input), "source": r.rate.as_ref().map(|x| x.source.clone()), "spendCalls": r.spend_calls, "history": r.history.len(), "efforts": r.cat.efforts.clone() }));
            let mut hist: Vec<&String> = self.hist.iter().collect();
            hist.sort();
            crate::ui::set_state_extra("models", serde_json::json!({ "providers": providers, "runnable": runnable, "hidden": hidden, "showHidden": self.show_hidden, "shown": shown, "haiku": haiku, "hist": hist, "reloads": self.reloads, "histCounts": rows.iter().filter(|r| r.history.len() > 1).map(|r| (r.cat.id.clone(), r.history.len())).collect::<std::collections::BTreeMap<_, _>>() }));
        }
        let mut out = pad(div()).child(probe("models")).child(
            div()
                .flex()
                .items_center()
                .gap(px(6.))
                .mb(px(10.))
                .child(div().flex_1().min_w_0().overflow_hidden().line_clamp(1).text_ellipsis().text_size(px(12.)).text_color(k.muted).child(format!("{runnable} models you can run · prices per 1M tokens")))
                .when(hidden > 0, |d| d.child(btn_small("showhidden", format!("{} {hidden} hidden", if self.show_hidden { "hide" } else { "show" }), Tone::Ghost, cx).flex_none().child(probe("models.showhidden")).on_click(cx.listener(|p, _, _, cx| { p.show_hidden = !p.show_hidden; cx.notify(); }))))
                .child(icon_btn("mrefresh", "refresh-cw", 14., cx).child(probe("models.refresh")).tooltip(tooltip("read the providers' model lists again")).on_click(cx.listener(|p, _, _, cx| {
                    fetch(&p.state, cx, |h| async move { let _ = h.refresh_catalog().await; }, |p, _, cx| p.reload(cx));
                }))),
        );
        let mut providers: Vec<String> = rows.iter().map(|r| r.cat.provider.clone()).collect();
        providers.dedup();
        for p in providers {
            let mut sec = section(cx).child(section_title(match p.as_str() { "anthropic" => "Anthropic · Claude Code".to_string(), "openai" => "OpenAI · Codex".to_string(), x => x.to_string() }, cx));
            for r in rows.iter().filter(|r| r.cat.provider == p && (self.show_hidden || !r.cat.hidden.unwrap_or(false))) {
                sec = sec.child(self.card_for(r, cx));
            }
            out = out.child(sec);
        }
        out
    }
}
impl ModelsPanel {
    fn card_for(&self, r: &ModelRow, cx: &mut Context<Self>) -> AnyElement {
        let k = c(cx).clone();
        let id = r.cat.id.clone();
        let hidden = r.cat.hidden.unwrap_or(false);
        let p = r.rate.as_ref().map(|r| r.per1m.clone());
        let editing = self.edit.as_ref().map(|e| e.0 == id).unwrap_or(false);
        let hist_open = self.hist.contains(&id);
        let mut tags = div().flex().flex_wrap().items_center().gap(px(6.)).mt(px(4.));
        if !r.available {
            tags = tags.child(chip("not offered now", ChipTone::Amber, cx));
        }
        if hidden {
            tags = tags.child(chip("hidden", ChipTone::Plain, cx));
        }
        if r.cat.computer_use {
            tags = tags.child(chip("computer use", ChipTone::Plain, cx));
        }
        tags = tags.child(div().text_size(px(11.)).text_color(k.muted).child(format!("efforts {}", r.cat.efforts.join(" · "))));
        let price = |label: &str, v: Option<f64>| div().flex().flex_col().child(div().text_size(px(10.)).text_color(k.muted).italic().child(label.to_string())).child(div().text_size(px(12.5)).child(fmt::price(v)));
        let rate_el = match &p {
            Some(p) => div().flex().flex_wrap().gap(px(14.)).mt(px(8.)).child(price("in", Some(p.input))).child(price("cache read", Some(p.cache_read))).child(price("cache write", Some(p.cache_write))).when(p.cache_write_1h.is_some(), |d| d.child(price("write 1 h", p.cache_write_1h))).child(price("out", Some(p.out))),
            None => div().mt(px(8.)).text_size(px(12.)).text_color(k.muted).child("No price yet. Its calls are recorded at $0 and marked unpriced; once you set a rate, those calls are priced at it."),
        };
        let st = self.state.clone();
        let mut cardv = card(cx)
            .mb(px(8.))
            .when(hidden, |d| d.opacity(0.6))
            .child(probe(format!("model.{id}")))
            .child(
                div()
                    .flex()
                    .items_start()
                    .child(div().flex_1().flex().flex_col().child(div().font_weight(FontWeight::SEMIBOLD).child(r.cat.label.clone())).child(div().font_family(crate::ui::MONO).text_size(px(11.)).text_color(k.muted).child(id.clone())))
                    .when(r.spend_calls > 0, |d| d.child(div().id(SharedString::from(format!("spend{id}"))).font_weight(FontWeight::SEMIBOLD).child(fmt::usd(r.spend_usd as f64 / 1e6)).tooltip(tooltip(format!("{} calls · {} tokens, from the ledger", r.spend_calls, fmt::tokens(r.spend_tokens)))))),
            )
            .child(tags)
            .child(rate_el);
        if let Some(l) = r.rate.as_ref().and_then(|r| r.long.clone()) {
            cardv = cardv.child(div().mt(px(4.)).text_size(px(11.)).text_color(k.muted).child(format!("input above {}: in {} · out {}", fmt::tokens(l.above), fmt::price(Some(l.per1m.input)), fmt::price(Some(l.per1m.out)))));
        }
        if let Some(rate) = &r.rate {
            cardv = cardv.child(
                div()
                    .flex()
                    .flex_wrap()
                    .items_center()
                    .gap(px(6.))
                    .mt(px(6.))
                    .text_size(px(11.5))
                    .text_color(k.text2)
                    .child(chip(rate.source.clone(), match rate.source.as_str() { "calibrated" => ChipTone::Acc, "manual" => ChipTone::Amber, _ => ChipTone::Plain }, cx))
                    .child(format!("since {}", fmt::day(&rate.from)))
                    ,
            );
            if let Some(n) = rate.note.clone() {
                cardv = cardv.child(div().mt(px(2.)).text_size(px(11.)).line_height(px(16.)).text_color(k.muted).child(n));
            }
        }
        let id2 = id.clone();
        let id3 = id.clone();
        let r2 = r.clone();
        cardv = cardv.child(
            div()
                .flex()
                .gap(px(12.))
                .mt(px(8.))
                .text_size(px(12.))
                .text_color(k.accent)
                .child(div().id(SharedString::from(format!("edit{id}"))).cursor_pointer().child(if editing { "cancel" } else if p.is_some() { "edit rate" } else { "set a rate" }).child(probe(format!("model.edit.{id}"))).on_click(cx.listener(move |m, _, window, cx| {
                    if m.edit.as_ref().map(|e| e.0 == r2.cat.id).unwrap_or(false) {
                        m.edit = None;
                        cx.notify();
                    } else {
                        m.start_edit(&r2, window, cx);
                    }
                })))
                .when(r.history.len() > 1, |d| d.child(div().id(SharedString::from(format!("hist{id}"))).cursor_pointer().child(format!("history ({})", r.history.len())).child(probe(format!("model.hist.{id}"))).on_click(cx.listener(move |m, _, _, cx| {
                    if !m.hist.remove(&id2) {
                        m.hist.insert(id2.clone());
                    }
                    cx.notify();
                }))))
                .child(div().id(SharedString::from(format!("hide{id}"))).cursor_pointer().child(if hidden { "show in pickers" } else { "hide from pickers" }).child(probe(format!("model.hide.{id}"))).on_click(move |_, _, cx| {
                    let id = id3.clone();
                    st.update(cx, |s, cx| s.command(cx, true, move |h| async move { h.hide_model(&id, !hidden).await }))
                })),
        );
        if editing {
            let (_, inputs) = self.edit.clone().unwrap();
            let labels = ["in", "cache read", "cache write", "write 1 h", "out"];
            let mut grid = div().flex().flex_wrap().gap(px(8.)).mt(px(8.));
            for (i, l) in labels.iter().enumerate() {
                grid = grid.child(div().w(px(76.)).flex().flex_col().gap(px(2.)).text_size(px(10.5)).text_color(k.muted).child(l.to_string()).child(Input::new(&inputs[i]).small()).child(probe(format!("rate.{i}"))));
            }
            cardv = cardv
                .child(grid)
                .child(div().mt(px(6.)).child(Input::new(&inputs[5]).small()))
                .child(div().flex().items_center().mt(px(6.)).gap(px(8.)).child(div().flex_1().min_w_0().text_size(px(11.)).text_color(k.muted).child("A new rate applies from now; past calls keep the rate they were priced at.")).child(btn_small("saverate", "Save rate", Tone::Primary, cx).child(probe("rate.save")).on_click(cx.listener(|m, _, _, cx| m.save_edit(cx)))));
        }
        if hist_open {
            let mut h = div().flex().flex_col().mt(px(8.)).text_size(px(11.5));
            for x in r.history.iter().rev() {
                h = h.child(div().flex().gap(px(8.)).py(px(2.)).child(div().w(px(90.)).child(fmt::day(&x.from))).child(div().w(px(70.)).text_color(k.muted).child(x.source.clone())).child(div().w(px(90.)).child(format!("{} / {}", fmt::price(Some(x.per1m.input)), fmt::price(Some(x.per1m.out))))).child(div().flex_1().min_w_0().overflow_hidden().line_clamp(1).text_ellipsis().text_color(k.muted).child(x.note.clone().unwrap_or_default())));
            }
            cardv = cardv.child(h);
        }
        cardv.into_any_element()
    }
}

// =====================================================================================================================
// Settings
// =====================================================================================================================
pub struct SettingsPanel {
    state: Entity<AppState>,
    instr: Entity<InputState>,
    cwd: Entity<InputState>,
    budget: Entity<InputState>,
    task: i64,
    _subs: Vec<Subscription>,
}
impl SettingsPanel {
    pub fn new(state: Entity<AppState>, window: &mut Window, cx: &mut Context<Self>) -> SettingsPanel {
        let instr = cx.new(|cx| InputState::new(window, cx).multi_line(true).auto_grow(4, 14).placeholder("who you are, how your files are organized, how you want work done (sent in each turn's header)"));
        let cwd = cx.new(|cx| InputState::new(window, cx));
        let budget = cx.new(|cx| InputState::new(window, cx));
        let mut subs = vec![];
        // saved on blur, like the web
        for (n, i) in [&instr, &cwd, &budget].into_iter().enumerate() {
            subs.push(cx.subscribe(i, move |p: &mut SettingsPanel, inp, e: &InputEvent, cx| {
                if matches!(e, InputEvent::Blur) || (n > 0 && matches!(e, InputEvent::PressEnter { .. })) {
                    let v = inp.read(cx).value().to_string();
                    let task = p.task;
                    let cur = p.state.read(cx).cur.settings.clone();
                    let Some(cur) = cur else { return };
                    let patch = match n {
                        0 if v != cur.instructions => SettingsPatch { instructions: Some(v), ..Default::default() },
                        1 if v != cur.cwd => SettingsPatch { cwd: Some(v), ..Default::default() },
                        2 => match v.trim().parse::<u64>() {
                            Ok(b) if b >= 2000 && b != cur.view_budget => SettingsPatch { view_budget: Some(b), ..Default::default() },
                            _ => return,
                        },
                        _ => return,
                    };
                    p.state.update(cx, |s, cx| s.command(cx, false, move |h| async move { h.update_settings(task, patch).await }));
                }
            }));
        }
        subs.push(cx.observe(&state, |_, _, cx| cx.notify()));
        SettingsPanel { state, instr, cwd, budget, task: -1, _subs: subs }
    }
    fn sync_inputs(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let s = self.state.read(cx);
        if s.current == self.task || s.cur.settings.is_none() {
            return;
        }
        self.task = s.current;
        let st = s.cur.settings.clone().unwrap();
        self.instr.update(cx, |i, cx| i.set_value(st.instructions.clone(), window, cx));
        self.cwd.update(cx, |i, cx| i.set_value(st.cwd.clone(), window, cx));
        self.budget.update(cx, |i, cx| i.set_value(st.view_budget.to_string(), window, cx));
    }
    fn patch(&self, p: SettingsPatch, cx: &mut Context<Self>) {
        let task = self.task;
        self.state.update(cx, |s, cx| s.command(cx, false, move |h| async move { h.update_settings(task, p).await }));
    }
}
fn picker(id: &str, label: String, items: Vec<(String, String, bool)>, on: impl Fn(String, &mut App) + 'static) -> impl IntoElement {
    let on = std::rc::Rc::new(on);
    div().flex().child(picker_btn(id, label, items, on)).child(probe(format!("picker.{id}")))
}
fn picker_btn(id: &str, label: String, items: Vec<(String, String, bool)>, on: std::rc::Rc<dyn Fn(String, &mut App)>) -> impl IntoElement {
    Button::new(SharedString::from(id.to_string())).outline().small().label(label).dropdown_caret(true).dropdown_menu(move |mut m, _, _| {
        for (v, l, checked) in &items {
            let on = on.clone();
            let v = v.clone();
            if v.starts_with("__label:") {
                m = m.label(l.clone());
                continue;
            }
            m = m.item(PopupMenuItem::new(l.clone()).checked(*checked).on_click(move |_, _, cx| on(v.clone(), cx)));
        }
        m.max_h(px(400.)).scrollable(true)
    })
}
impl Render for SettingsPanel {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        // a cached view is laid out at the size its parent gives it, so the panel scrolls itself
        crate::ui::anim::area(crate::ui::anim::PANEL);
        let body = self.body(window, cx).into_any_element();
        crate::ui::scroll_body("SettingsPanel", body)
    }
}
impl SettingsPanel {
    fn body(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        self.sync_inputs(window, cx);
        let k = c(cx).clone();
        let s = self.state.read(cx);
        let global_tab = s.prefs.settings_tab_global;
        let task = s.current;
        let st_e = self.state.clone();
        let tabs = div()
            .flex()
            .gap(px(4.))
            .mb(px(12.))
            .child(seg("tab-task", "Workspace", !global_tab, cx).child(probe("settings.task")).child(probe("settings.workspace")).on_click({
                let st = st_e.clone();
                move |_, _, cx| st.update(cx, |s, cx| { s.prefs.settings_tab_global = false; s.save_prefs(); cx.notify(); })
            }))
            .child(seg("tab-global", "App", global_tab, cx).child(probe("settings.global")).child(probe("settings.app")).on_click({
                let st = st_e.clone();
                move |_, _, cx| st.update(cx, |s, cx| { s.prefs.settings_tab_global = true; s.save_prefs(); cx.notify(); })
            }));
        let field = |label: &str| div().text_size(px(11.5)).text_color(k.muted).mb(px(4.)).child(label.to_string());
        if global_tab {
            let engines = self.engines_section(cx).into_any_element();
            let s = self.state.read(cx);
            let g = s.global.clone();
            let font_size = s.prefs.font_size;
            let themes: Vec<(String, String, bool)> = {
                let th = cx.global::<Themes>();
                th.all.iter().map(|t| (t.id.clone(), t.name.clone(), t.id == th.active)).collect()
            };
            let (active_name, follow) = {
                let th = cx.global::<Themes>();
                (th.all.iter().find(|t| t.id == th.active).map(|t| t.name.clone()).unwrap_or_default(), th.follow_system)
            };
            let st1 = st_e.clone();
            let st2 = st_e.clone();
            let st3 = st_e.clone();
            let st4 = st_e.clone();
            let log: Vec<String> = s.server_log.iter().rev().take(80).rev().cloned().collect();
            let stats = s.stats.clone();
            let _ = (g, st1, st2, st3, log);
            let updates = if cfg!(target_os = "windows") {
                Some(section(cx)
                    .child(section_title("updates", cx))
                    .child(div().text_size(px(12.)).text_color(k.muted).child("Download the latest Windows installer. Close OptChat before installing it; your settings and connections are kept."))
                    .child(btn_small("download-updates", "Download updates", Tone::Plain, cx)
                        .child(probe("settings.updates"))
                        .on_click(|_, _, cx| cx.open_url("https://github.com/fabihatasneem/optchat-client/releases"))))
            } else { None };
            return pad(div())
                .child(probe("settings"))
                .child(tabs)
                .child(engines)
                .children(updates)
                .child(
                    section(cx)
                        .child(section_title("appearance", cx))
                        .child(field("theme"))
                        .child(div().child(picker("theme", active_name, themes, move |v, cx| {
                            crate::theme::set(cx, &v, false);
                            st4.update(cx, |s, cx| {
                                s.prefs.theme = Some(v.clone());
                                s.prefs.follow_system = false;
                                s.save_prefs();
                                cx.notify();
                            });
                        })).child(probe("settings.theme")))
                        .child(
                            div()
                                .flex()
                                .items_center()
                                .justify_between()
                                .mt(px(10.))
                                .child(div().text_size(px(13.)).child("Follow the system appearance").child(div().text_size(px(11.)).text_color(k.muted).child("OptChat Dark / OptChat Light")))
                                .child(Switch::new("follow").checked(follow).on_click({
                                    let st = st_e.clone();
                                    move |on: &bool, window, cx| {
                                        let a = window.appearance();
                                        cx.global_mut::<Themes>().follow_system = *on;
                                        if *on {
                                            crate::theme::on_appearance(cx, a);
                                        }
                                        let on = *on;
                                        st.update(cx, |s, cx| {
                                            s.prefs.follow_system = on;
                                            s.save_prefs();
                                            cx.notify();
                                        });
                                    }
                                })),
                        )
                        .child(
                            div()
                                .flex()
                                .items_center()
                                .justify_between()
                                .mt(px(10.))
                                .child(div().text_size(px(13.)).child("Chat text size").child(div().text_size(px(11.)).text_color(k.muted).child(crate::ui::shortcut_label("⌘= bigger · ⌘- smaller · ⌘⇧0 actual size"))))
                                .child(
                                    div()
                                        .flex()
                                        .items_center()
                                        .gap(px(4.))
                                        .child(btn_small("fs-minus", "−", Tone::Plain, cx).child(probe("settings.font.minus")).on_click(|_, window, cx| window.dispatch_action(Box::new(crate::workspace::ZoomOut), cx)))
                                        .child(div().w(px(34.)).text_center().text_size(px(12.5)).child(format!("{font_size:.0}")))
                                        .child(btn_small("fs-plus", "+", Tone::Plain, cx).child(probe("settings.font.plus")).on_click(|_, window, cx| window.dispatch_action(Box::new(crate::workspace::ZoomIn), cx)))
                                        .child(btn_small("fs-reset", "reset", Tone::Ghost, cx).on_click(|_, window, cx| window.dispatch_action(Box::new(crate::workspace::ZoomReset), cx))),
                                ),
                        ),
                )
                .child(section(cx).child(section_title("performance", cx)).child(card(cx).child(kv_row("first frame", stats.first_frame_ms.map(|m| format!("{m:.0} ms after launch")).unwrap_or("—".into()), cx)).child(kv_row("chat loaded", stats.loaded_ms.map(|m| format!("{m:.0} ms after launch")).unwrap_or("—".into()), cx)).child(kv_row("events applied", stats.events.to_string(), cx)).child(kv_row("workspace switches", s.ws_switches.last().map(|m| format!("{} · last {m:.2} ms", s.ws_switches.len())).unwrap_or("—".into()), cx))))
                .into_any_element();
        }
        let g = s.global.clone();
        let st1 = st_e.clone();
        let st2 = st_e.clone();
        let st3 = st_e.clone();
        let log: Vec<String> = s.server_log.iter().rev().take(80).rev().cloned().collect();
        let srvlog = section(cx).child(section_title(format!("server log · engine {} · build {} · {}", s.version, s.build, s.hub().map(|h| h.label()).unwrap_or_default()), cx)).child(div().id("srvlog").max_h(px(300.)).overflow_y_scroll().p(px(8.)).rounded(px(6.)).bg(k.code_bg).font_family(crate::ui::MONO).text_size(px(11.)).line_height(px(16.)).text_color(k.text2).child(if log.is_empty() { "—".to_string() } else { log.join("\n") }));
        let ws_name = s.meta.get(&s.active).map(|m| m.ws.clone()).unwrap_or_default();
        let ws_head = div()
            .text_size(px(12.))
            .text_color(k.muted)
            .mb(px(12.))
            .child(format!("Settings of the workspace “{ws_name}” on its engine: every client sees them. The engine's own settings are on its admin page."));
        let ws_global = section(cx)
            .child(section_title("turns", cx))
            .child(field("turns at once, across all tasks"))
            .child(picker("maxturns", g.max_turns.to_string(), [1, 2, 3, 4, 6, 8, 12, 16].iter().map(|n| (n.to_string(), n.to_string(), *n == g.max_turns)).collect(), move |v, cx| {
                let n: u32 = v.parse().unwrap_or(4);
                st1.update(cx, |s, cx| s.command(cx, false, move |h| async move { { h.update_global(Some(n), None); Ok(String::new()) } }))
            }))
            .child(div().mt(px(10.)).child(field("dictation: speech to text")))
            .child(
                div()
                    .flex()
                    .gap(px(4.))
                    .child(seg("dict-claude", "Claude", g.dictation == DictationService::Claude, cx).child(probe("settings.dict.claude")).on_click(move |_, _, cx| st2.update(cx, |s, cx| s.command(cx, false, |h| async move { { h.update_global(None, Some(DictationService::Claude)); Ok(String::new()) } }))))
                    .child(seg("dict-codex", "Codex", g.dictation == DictationService::Codex, cx).child(probe("settings.dict.codex")).on_click(move |_, _, cx| st3.update(cx, |s, cx| s.command(cx, false, |h| async move { { h.update_global(None, Some(DictationService::Codex)); Ok(String::new()) } })))),
            );
        let Some(st) = s.cur.settings.clone() else { return pad(div()).child(probe("settings")).child(tabs).child(ws_head).child(ws_global).into_any_element() };
        let info = s.info().cloned();
        let parent = info.as_ref().and_then(|i| i.task.parent).and_then(|p| s.tasks.get(&p)).map(|p| p.task.settings.depth);
        let max_depth = parent.map(|d| (d - 1).max(0)).unwrap_or(8);
        let cm_efforts = s.model(&st.compactor_model).map(|m| m.efforts.clone()).unwrap_or_default();
        let model_comp = s.model(&st.model).map(|m| m.computer_use).unwrap_or(false);
        let cua = s.computer_available;
        let mut groups: Vec<(String, Vec<String>)> = vec![];
        for t in &s.cur.tool_list {
            match groups.iter_mut().find(|g| g.0 == t.group) {
                Some(g) => g.1.push(t.name.clone()),
                None => groups.push((t.group.clone(), vec![t.name.clone()])),
            }
        }
        let models_items: Vec<(String, String, bool)> = {
            let mut v = vec![("__label:c".to_string(), "Claude Code (Anthropic)".to_string(), false)];
            v.extend(s.models.iter().filter(|m| m.backend == Backend::ClaudeCode).map(|m| (m.id.clone(), m.label.clone(), m.id == st.compactor_model)));
            v.push(("__label:o".into(), "Codex (OpenAI)".into(), false));
            v.extend(s.models.iter().filter(|m| m.backend == Backend::Codex).map(|m| (m.id.clone(), format!("{}{}", m.label, if m.priced { "" } else { " (unpriced)" }), m.id == st.compactor_model)));
            v
        };
        let cm_label = s.model(&st.compactor_model).map(|m| m.label.clone()).unwrap_or(st.compactor_model.clone());
        let me = cx.entity().downgrade();
        let me2 = me.clone();
        let me3 = me.clone();
        let me4 = me.clone();
        let toggle = |label: &str, sub: String, on: bool, id: &'static str, f: Box<dyn Fn(bool, &mut App)>| {
            div()
                .flex()
                .items_center()
                .justify_between()
                .gap(px(10.))
                .py(px(6.))
                .child(div().flex_1().min_w_0().flex().flex_col().text_size(px(13.)).child(label.to_string()).child(div().text_size(px(11.)).text_color(k.muted).child(sub)))
                .child(Switch::new(id).checked(on).on_click(move |v: &bool, _, cx| f(*v, cx)))
        };
        let mk = |me: WeakEntity<SettingsPanel>, which: &'static str| -> Box<dyn Fn(bool, &mut App)> {
            Box::new(move |v, cx| {
                let _ = me.update(cx, |p, cx| {
                    let t = match which {
                        "core" => ToolsPatch { core: Some(v), ..Default::default() },
                        "computer" => ToolsPatch { computer: Some(v), ..Default::default() },
                        _ => ToolsPatch { mcp: Some(v), ..Default::default() },
                    };
                    p.patch(SettingsPatch { tools: Some(t), ..Default::default() }, cx)
                });
            })
        };
        pad(div())
            .child(probe("settings"))
            .child(tabs)
            .child(ws_head)
            .child(ws_global)
            .child(div().text_size(px(13.)).font_weight(FontWeight::SEMIBOLD).mt(px(6.)).mb(px(4.)).child(if task == 0 { "Main chat".to_string() } else { format!("Task #{task}") }))
            .child(div().text_size(px(12.)).text_color(k.muted).mb(px(12.)).child("New child tasks start with a copy of these (a parent may override parts)."))
            .child(section(cx).child(section_title("instructions", cx)).child(div().child(Input::new(&self.instr)).child(probe("settings.instructions"))))
            .child(section(cx).child(section_title("tasks", cx)).child(field("depth: how many levels of child tasks it may create (0 = none; a child gets at most its parent's − 1)")).child(picker("depth", format!("{}{}", st.depth, if st.depth == 0 { " (no task tools)" } else { "" }), (0..=max_depth).map(|d| (d.to_string(), format!("{d}{}", if d == 0 { " (no task tools)" } else { "" }), d == st.depth)).collect(), move |v, cx| {
                let d: i64 = v.parse().unwrap_or(0);
                let _ = me.update(cx, |p, cx| p.patch(SettingsPatch { depth: Some(d), ..Default::default() }, cx));
            })))
            .child(
                section(cx)
                    .child(section_title("compactor (summary tree)", cx))
                    .child(field("model"))
                    .child(picker("cmodel", cm_label, models_items, move |v, cx| {
                        let _ = me2.update(cx, |p, cx| p.patch(SettingsPatch { compactor_model: Some(v), ..Default::default() }, cx));
                    }))
                    .child(div().mt(px(8.)).child(field("effort")))
                    .child(picker("ceffort", st.compactor_effort.clone(), cm_efforts.iter().map(|e| (e.clone(), e.clone(), *e == st.compactor_effort)).collect(), move |v, cx| {
                        let _ = me3.update(cx, |p, cx| p.patch(SettingsPatch { compactor_effort: Some(v), ..Default::default() }, cx));
                    })),
            )
            .child(
                section(cx)
                    .child(section_title("tools", cx))
                    .child(toggle("Local operations", "bash, read/write/edit files, glob, grep, web fetch".into(), st.tools.core, "t-core", mk(cx.entity().downgrade(), "core")))
                    .child(toggle("Computer & browser use", if cua { format!("Codex Computer Use (cua) — {}; one task drives it at a time", if model_comp { "supported by this model" } else { "not offered to this model" }) } else { "needs ChatGPT.app (cua_repl) installed".into() }, st.tools.computer, "t-comp", mk(cx.entity().downgrade(), "computer")))
                    .child(toggle("reframe MCP", "source.*, yan.*, eval.* on mcp.reframe.so".into(), st.tools.mcp, "t-mcp", mk(cx.entity().downgrade(), "mcp")))
                    .child(div().mt(px(10.)).child(field("computer use approvals")).child(picker("approval", if st.computer_approval == "always" { "always allow".into() } else { "ask me for each app".into() }, vec![("ask".into(), "ask me for each app".into(), st.computer_approval == "ask"), ("always".into(), "always allow".into(), st.computer_approval == "always")], move |v, cx| {
                        let _ = me4.update(cx, |p, cx| p.patch(SettingsPatch { computer_approval: Some(v), ..Default::default() }, cx));
                    })))
                    .child({
                        let mut g = div().mt(px(8.)).text_size(px(11.5)).line_height(px(18.)).text_color(k.muted);
                        for (name, tools) in groups {
                            g = g.child(div().child(format!("{name}: {}", tools.join(", "))));
                        }
                        g
                    }),
            )
            .child(section(cx).child(section_title("files and view", cx)).child(field("working directory for tools")).child(Input::new(&self.cwd).small()).child(div().mt(px(8.)).child(field("view budget (bytes): the view grows to this size, then one batch merges it down to half"))).child(Input::new(&self.budget).small()))
            .child(srvlog)
            .into_any_element()
    }

    /// App settings: the engines and the workspaces connected on each (tokens are in the Keychain)
    fn engines_section(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let k = c(cx).clone();
        let s = self.state.read(cx);
        let mut sec = section(cx).child(section_title("engines and workspaces", cx)).child(probe("settings.engines"));
        if s.engines.is_empty() {
            sec = sec.child(div().text_size(px(12.)).text_color(k.muted).mb(px(8.)).child("No engine yet. Ask your engine administrator for a client token, then choose Add engine and paste it. Your workspaces connect automatically."));
        }
        for (i, e) in s.engines.iter().enumerate() {
            let online = e.online.unwrap_or(false);
            let ep = e.cfg.endpoint.clone();
            let st = self.state.clone();
            let mut card_ = card(cx).child(
                div()
                    .flex()
                    .items_center()
                    .gap(px(8.))
                    .child(dot(if online { k.green } else if e.online.is_none() { k.muted } else { k.red }, 7.))
                    .child(div().flex_1().min_w_0().flex().flex_col().child(div().text_size(px(13.)).font_weight(FontWeight::MEDIUM).child(e.cfg.name.clone())).child(div().text_size(px(11.)).text_color(k.muted).child(format!("{}{}", e.cfg.endpoint, if online { String::new() } else if let Some(er) = &e.error { format!(" · offline: {er}") } else { " · offline".into() }))))
                    .child(btn_small(SharedString::from(format!("eng-rm-{i}")), "remove", Tone::Ghost, cx).on_click(move |_, _, cx| {
                        let ep = ep.clone();
                        st.update(cx, |s, cx| s.remove_engine(&ep, cx))
                    })),
            );
            for m in s.meta.values().filter(|m| m.endpoint == e.cfg.endpoint) {
                let key = crate::client::ws_key(&m.endpoint, &m.ws);
                let st = self.state.clone();
                let state = if m.disabled.is_some() { "disabled" } else if m.no_token { "no token" } else { match m.conn { optchat_proto::api::Conn::Open => "connected", optchat_proto::api::Conn::Connecting => "connecting", optchat_proto::api::Conn::Closed => "offline" } };
                card_ = card_.child(
                    div()
                        .flex()
                        .items_center()
                        .gap(px(8.))
                        .pl(px(15.))
                        .pt(px(6.))
                        .child(div().flex_1().text_size(px(12.5)).child(format!("{}{}", m.ws, if key == s.active { "  ✓" } else { "" })))
                        .child(div().text_size(px(11.)).text_color(k.muted).child(state))
                        .child(btn_small(SharedString::from(format!("ws-dc-{key}")), "disconnect", Tone::Ghost, cx).on_click(move |_, _, cx| {
                            let key = key.clone();
                            st.update(cx, |s, cx| s.disconnect_ws(&key, cx))
                        })),
                );
            }
            sec = sec.child(div().mb(px(8.)).child(card_));
        }
        sec.child(div().flex().gap(px(6.)).child(btn_small("eng-add", "Add engine…", Tone::Plain, cx).child(probe("settings.addEngine")).on_click(|_, window, cx| window.dispatch_action(Box::new(crate::workspace::AddEngine), cx))).child(btn_small("ws-menu", "Workspaces…", Tone::Ghost, cx).on_click(|_, window, cx| window.dispatch_action(Box::new(crate::workspace::WorkspaceMenu), cx))))
    }
}
