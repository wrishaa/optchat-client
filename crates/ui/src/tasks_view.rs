//! The Tasks view (sidebar): the current task's subtree with its ancestors above it, or every task.
use crate::fmt;
use crate::state::*;
use crate::theme::c;
use crate::ui::*;
use gpui::prelude::FluentBuilder;
use gpui::*;
use optchat_proto::api::TaskInfo;

pub enum TasksEvent {
    Open(i64),
    Act(i64, &'static str),
    Menu(i64, optchat_proto::types::TaskStatus, Point<Pixels>),
}
/// The Tasks view as its own (cached) view: it re-renders only when the state changes, never while the feed scrolls.
pub struct TasksPanel {
    pub state: Entity<AppState>,
    pub collapsed: std::collections::HashSet<i64>,
    /// a task menu is open: no row tooltips (they are drawn above everything and would cover the menu)
    pub menu_open: bool,
    /// the phone layout: taller rows, larger text, separators
    pub large: bool,
    _sub: Subscription,
}
impl EventEmitter<TasksEvent> for TasksPanel {}
impl TasksPanel {
    pub fn new(state: Entity<AppState>, cx: &mut Context<Self>) -> TasksPanel {
        let _sub = cx.observe(&state, |_, _, cx| cx.notify());
        TasksPanel { state, collapsed: Default::default(), menu_open: false, large: false, _sub }
    }
}
impl Render for TasksPanel {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        // a cached view is laid out at the size its parent gives it, so the panel scrolls itself
        crate::ui::anim::area(crate::ui::anim::TASKS);
        let body = self.body(window, cx).into_any_element();
        crate::ui::scroll_body("TasksPanel", body)
    }
}
impl TasksPanel {
    fn body(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        render(self, window, cx)
    }
}

/// TV: only tasks that are running (a turn going, or status active) or paused show, plus their ancestors back to the
/// root so the tree still reads, and the open task's path. None off the TV. Recomputed per render, so it stays live.
fn tv_live(s: &AppState) -> Option<std::collections::HashSet<i64>> {
    use optchat_proto::types::TaskStatus;
    if !crate::tv::enabled() {
        return None;
    }
    let mut keep = std::collections::HashSet::new();
    let mut mark = |mut id: Option<i64>| {
        while let Some(i) = id {
            if !keep.insert(i) {
                break;
            }
            id = s.tasks.get(&i).and_then(|x| x.task.parent);
        }
    };
    for x in s.tasks.values() {
        if x.running || matches!(x.task.status, TaskStatus::Active | TaskStatus::Paused) {
            mark(Some(x.task.id));
        }
    }
    mark(Some(s.current));
    Some(keep)
}

/// the rows the Tasks view shows, top to bottom: (task, depth, on the path above the current task). Finished tasks are
/// hidden unless "show ended" is on (always hidden on the TV), except the current one.
pub fn visible_rows(s: &AppState, collapsed: &std::collections::HashSet<i64>) -> Vec<(TaskInfo, usize, bool)> {
    let mut rows: Vec<(TaskInfo, usize, bool)> = vec![];
    let show_ended = s.prefs.show_ended && !crate::tv::enabled();
    let cur = s.current;
    let live = tv_live(s);
    let live = &live;
    fn walk(s: &AppState, id: i64, depth: usize, closed: &std::collections::HashSet<i64>, show_ended: bool, cur: i64, live: &Option<std::collections::HashSet<i64>>, out: &mut Vec<(TaskInfo, usize, bool)>) {
        let Some(x) = s.tasks.get(&id) else { return };
        out.push((x.clone(), depth, false));
        if closed.contains(&id) {
            return;
        }
        for c in s.children_of(id) {
            if let Some(l) = live {
                if l.contains(&c.task.id) {
                    walk(s, c.task.id, depth + 1, closed, show_ended, cur, live, out);
                }
                continue;
            }
            if show_ended || !c.task.status.terminal() || c.task.id == cur || s.ancestors(cur).iter().any(|a| a.task.id == c.task.id) {
                walk(s, c.task.id, depth + 1, closed, show_ended, cur, live, out);
            }
        }
    }
    if s.prefs.task_scope_all || crate::tv::enabled() {
        walk(s, 0, 0, collapsed, show_ended, cur, live, &mut rows);
    } else {
        let up = s.ancestors(cur);
        let n = up.len();
        for (d, x) in up.into_iter().enumerate() {
            rows.push((x.clone(), d, true));
        }
        walk(s, cur, n, collapsed, show_ended, cur, live, &mut rows);
    }
    rows
}

pub fn render(w: &mut TasksPanel, _window: &mut Window, cx: &mut Context<TasksPanel>) -> impl IntoElement {
    let k = c(cx).clone();
    let s = w.state.read(cx);
    let mut list = div().flex().flex_col().pb(px(16.)).px(px(6.));
    if s.tasks.get(&s.current).is_none() || s.tasks.get(&0).is_none() {
        return list.child(skeleton_line(None, cx)).child(skeleton_line(Some(160.), cx)).child(skeleton_line(None, cx));
    }
    let cur = s.current;
    let rows = visible_rows(s, &w.collapsed);
    let no_kids = s.children_of(cur).is_empty();
    let up_n = if s.prefs.task_scope_all { 0 } else { s.ancestors(cur).len() };
    let scope_all = s.prefs.task_scope_all;
    set_state_extra("tasksPanel", serde_json::json!({ "rows": rows.len(), "noKids": no_kids && !scope_all, "collapsed": w.collapsed.iter().collect::<Vec<_>>() }));
    for (x, depth, path) in rows {
        list = list.child(row(w, &x, depth, path, cx));
    }
    if no_kids && !scope_all {
        list = list.child(div().pl(px(40. + up_n as f32 * 14.)).py(px(6.)).text_size(px(12.)).text_color(k.faint).child("no subtasks yet").child(probe("tasks.nokids")));
    }
    list
}

fn row(w: &TasksPanel, x: &TaskInfo, depth: usize, path: bool, cx: &mut Context<TasksPanel>) -> impl IntoElement {
    let k = c(cx).clone();
    let s = w.state.read(cx);
    let t = &x.task;
    let id = t.id;
    let is_cur = id == s.current;
    let ended = t.status.terminal();
    let kids = if path { 0 } else { s.children_of(id).iter().filter(|c| (s.prefs.show_ended && !crate::tv::enabled()) || !c.task.status.terminal() || c.task.id == s.current).count() };
    let open = !w.collapsed.contains(&id);
    let usd = sub(Some(x), "usd");
    let active = sub(Some(x), "time.active");
    let tip = format!(
        "{}\n#{} · {} · {}{}{}",
        t.title,
        id,
        t.mode.as_str(),
        t.status.as_str(),
        t.reason.as_ref().map(|r| format!(" ({r})")).unwrap_or_default(),
        t.result.as_ref().map(|r| format!("\n→ {r}")).unwrap_or_default()
    );
    let status_word = if x.running { "working".to_string() } else if x.waiting { "waiting".into() } else { t.status.as_str().into() };
    let dot_tip = format!("{tip}\n{status_word}");
    let st_col = match t.status.as_str() {
        "paused" => k.amber,
        "canceled" => k.red,
        "done" => k.green,
        "active" => k.accent,
        _ => k.muted,
    };
    let hover = k.hover;
    let large = w.large;
    let mut meta = div().flex().gap(px(7.)).text_size(px(if large { 12.5 } else { 11. })).text_color(k.faint).whitespace_nowrap().child(div().font_family(crate::ui::MONO).child(format!("#{id}")));
    if usd > 0 {
        meta = meta.child(fmt::usd(usd as f64 / 1e6));
    }
    if active >= 1000 {
        meta = meta.child(fmt::ms(active));
    }
    if t.parent.is_some() {
        meta = meta.child(t.mode.as_str().to_string());
    }
    if t.status.as_str() != "idle" {
        meta = meta.child(div().text_color(st_col).child(status_word));
    }
    let me = cx.entity().downgrade();
    let status = t.status;
    // w_full + min_w_0 down the chain: a long title truncates with "…" instead of widening the row
    div().id(SharedString::from(format!("trow-k-{id}"))).w_full().min_w_0().child(div()
        .id(SharedString::from(format!("trow-{id}")))
        .w_full()
        .min_w_0()
        .flex()
        .items_center()
        .gap(px(7.))
        .h(px(if large { 58. } else { 40. }))
        .pl(px(if large { 10. } else { 6. } + depth as f32 * if large { 18. } else { 14. }))
        .pr(px(if large { 12. } else { 8. }))
        .when(!large, |d| d.rounded(px(7.)))
        .when(large, |d| d.border_b_1().border_color(k.line))
        .cursor_pointer()
        .text_color(if path { k.muted } else { k.text2 })
        .when(is_cur, |d| d.bg(k.accent_bg).text_color(k.text).child(div().absolute().left_0().top(px(6.)).bottom(px(6.)).w(px(2.)).bg(k.accent)))
        .when(!is_cur, |d| d.hover(move |s| s.bg(hover)))
        .relative()
        .child(probe(format!("trow.{id}")))
        .child(
            div()
                .id(SharedString::from(format!("tw-{id}")))
                .w(px(12.))
                .flex_none()
                .when(kids > 0, |d| {
                    d.child(chevron(format!("trow{id}"), 13., k.faint, open)).child(probe(format!("trow.chev.{id}"))).on_click(cx.listener(move |w, _, _, cx| {
                        cx.stop_propagation();
                        if !w.collapsed.remove(&id) {
                            w.collapsed.insert(id);
                        }
                        cx.notify();
                    }))
                }),
        )
        // the dot has its own tooltip: title, mode, status (with the reason) and the result
        .child(div().id(SharedString::from(format!("trdot-{id}"))).flex_none().p(px(2.)).child(status_dot(format!("tr{id}"), t.status.as_str(), 8., cx)).child(probe(format!("trow.dot.{id}"))).when(!w.menu_open, |d| d.tooltip(tooltip(dot_tip)))) 
        .child(
            div()
                .id(SharedString::from(format!("trtext-{id}")))
                .flex_1()
                .min_w_0()
                .flex()
                .flex_col()
                .gap(px(1.))
                // the details as a tooltip over the title (the dot has its own)
                .when(!w.menu_open, |d| d.tooltip(tooltip(tip)))
                .child(div().text_size(px(if large { 15.5 } else { 13. })).overflow_hidden().line_clamp(1).text_ellipsis().when(ended, |d| d.text_color(k.muted)).child(task_name(t)))
                .child(meta),
        )
        .when(x.running, |d| d.child(spinner(SharedString::from(format!("trs{id}")), 13., k.accent)))
        .on_click(cx.listener(move |_, _, _, cx| cx.emit(TasksEvent::Open(id))))
        // the menu itself is drawn by the workspace (see Workspace::open_task_menu)
        .on_mouse_down(MouseButton::Right, move |e: &MouseDownEvent, _, cx| {
            let pos = e.position;
            let _ = me.update(cx, |_, cx| cx.emit(TasksEvent::Menu(id, status, pos)));
        }))
}
