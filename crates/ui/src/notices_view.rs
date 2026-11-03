//! The Notifications view (rail): the workspace's notices (spec-workspaces §4), newest first: filters by level and
//! unread, a click opens the notice's task and marks it read (on the engine: every client sees it), mark all read.
use crate::fmt;
use crate::state::*;
use crate::theme::c;
use crate::ui::*;
use gpui::prelude::FluentBuilder;
use gpui::*;
use optchat_proto::api::Notice;

pub enum NoticesEvent {
    Open(i64),
}

pub struct NoticesPanel {
    pub state: Entity<AppState>,
    /// all | error | warn | info | ok
    pub level: &'static str,
    pub unread_only: bool,
    loading_for: String,
    _sub: Subscription,
}
impl EventEmitter<NoticesEvent> for NoticesPanel {}

pub fn level_color(level: &str, cx: &App) -> Hsla {
    let k = c(cx);
    match level {
        "error" => k.red,
        "warn" => k.amber,
        "ok" => k.green,
        _ => k.muted,
    }
}

impl NoticesPanel {
    pub fn new(state: Entity<AppState>, cx: &mut Context<Self>) -> NoticesPanel {
        let _sub = cx.observe(&state, |_, _, cx| cx.notify());
        NoticesPanel { state, level: "all", unread_only: false, loading_for: String::new(), _sub }
    }
    fn row(&self, i: usize, n: &Notice, cx: &mut Context<Self>) -> AnyElement {
        let k = c(cx).clone();
        let unread = n.read_at.is_none();
        let col = level_color(&n.level, cx);
        let id = n.id.clone();
        let task = n.task;
        let ago = fmt::ago(&n.at);
        let hover = k.hover;
        div()
            .id(SharedString::from(format!("notice-{i}")))
            .relative()
            .flex()
            .gap(px(9.))
            .px(px(10.))
            .py(px(8.))
            .rounded(px(7.))
            .cursor_pointer()
            .hover(move |s| s.bg(hover))
            .when(unread, |d| d.bg(k.accent_bg.opacity(0.35)))
            .child(div().pt(px(5.)).child(dot(if unread { col } else { col.opacity(0.45) }, 8.)))
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .flex()
                    .flex_col()
                    .gap(px(2.))
                    .child(div().text_size(px(12.5)).line_height(px(17.)).text_color(if unread { k.text } else { k.text2 }).when(unread, |d| d.font_weight(FontWeight::MEDIUM)).child(n.text.clone()))
                    .child(
                        div()
                            .flex()
                            .gap(px(8.))
                            .text_size(px(11.))
                            .text_color(k.muted)
                            .child(n.level.clone())
                            .when_some(task, |d, t| d.child(if t == 0 { "main chat".to_string() } else { format!("#{t}") }))
                            .child(ago)
                            .when(n.count.unwrap_or(1) > 1, |d| d.child(format!("×{}", n.count.unwrap_or(1)))),
                    ),
            )
            .children(probe(format!("notif.row.{i}")))
            .on_click(cx.listener(move |p, _, _, cx| {
                let id = id.clone();
                p.state.update(cx, |s, cx| s.read_notices(Some(vec![id]), None, cx));
                if let Some(t) = task {
                    cx.emit(NoticesEvent::Open(t));
                }
            }))
            .into_any_element()
    }
}

impl Render for NoticesPanel {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let _ = window;
        let k = c(cx).clone();
        let (active, loaded, has_hub) = {
            let s = self.state.read(cx);
            (s.active.clone(), s.notices_loaded, s.hub.is_some())
        };
        if !loaded && has_hub && self.loading_for != active {
            self.loading_for = active.clone();
            self.state.update(cx, |s, cx| s.load_notices(cx));
        }
        let s = self.state.read(cx);
        let u = s.unread();
        let level = self.level;
        let unread_only = self.unread_only;
        let shown: Vec<Notice> = s.notices.iter().filter(|n| (level == "all" || n.level == level) && (!unread_only || n.read_at.is_none())).take(500).cloned().collect();
        set_state_extra("notices", serde_json::json!({ "shown": shown.len(), "level": level, "unreadOnly": unread_only, "loaded": loaded, "unread": u, "first": shown.first().map(|n| serde_json::json!({ "id": n.id, "text": n.text, "level": n.level, "task": n.task, "read": n.read_at.is_some() })) }));
        let filt = |id: &'static str, label: String, on: bool, cx: &mut Context<Self>| {
            let kk = c(cx).clone();
            div()
                .id(SharedString::from(format!("nf-{id}")))
                .px(px(8.))
                .py(px(3.))
                .rounded(px(6.))
                .text_size(px(11.5))
                .cursor_pointer()
                .when(on, |d| d.bg(kk.selected).text_color(kk.text))
                .when(!on, |d| d.text_color(kk.muted).hover(move |s| s.bg(kk.hover)))
                .child(label)
                .children(probe(format!("notif.filter.{id}")))
                .on_click(cx.listener(move |p, _, _, cx| {
                    p.level = id;
                    cx.notify();
                }))
        };
        let count = |n: i64| if n > 0 { format!(" {n}") } else { String::new() };
        let filters = div()
            .flex()
            .flex_wrap()
            .gap(px(3.))
            .px(px(10.))
            .pb(px(8.))
            .child(filt("all", format!("All{}", count(u.total())), level == "all", cx))
            .child(filt("error", format!("Errors{}", count(u.error)), level == "error", cx))
            .child(filt("warn", format!("Warnings{}", count(u.warn)), level == "warn", cx))
            .child(filt("info", format!("Info{}", count(u.info)), level == "info", cx))
            .child(filt("ok", format!("OK{}", count(u.ok)), level == "ok", cx));
        let lv = if level == "all" { None } else { Some(level.to_string()) };
        let tools = div()
            .flex()
            .items_center()
            .gap(px(6.))
            .px(px(10.))
            .pb(px(8.))
            .child(
                div()
                    .id("nf-unread")
                    .flex()
                    .items_center()
                    .gap(px(5.))
                    .text_size(px(11.5))
                    .text_color(if unread_only { k.text } else { k.muted })
                    .cursor_pointer()
                    .child(icon(if unread_only { "eye-off" } else { "eye" }, 13., if unread_only { k.text } else { k.muted }))
                    .child(if unread_only { "unread only" } else { "read and unread" })
                    .children(probe("notif.unreadOnly"))
                    .on_click(cx.listener(|p, _, _, cx| {
                        p.unread_only = !p.unread_only;
                        cx.notify();
                    })),
            )
            .child(div().flex_1())
            .child(
                btn_small("nf-readall", if level == "all" { "Mark all read".to_string() } else { "Mark these read".to_string() }, Tone::Ghost, cx)
                    .children(probe("notif.readAll"))
                    .on_click(cx.listener(move |p, _, _, cx| {
                        let lv = lv.clone();
                        p.state.update(cx, |s, cx| s.read_notices(None, lv, cx));
                    })),
            );
        let mut list = div().flex().flex_col().px(px(4.)).pb(px(16.));
        if shown.is_empty() {
            list = list.child(div().p(px(14.)).text_size(px(12.)).text_color(k.muted).child(if !loaded && has_hub { "loading…" } else if unread_only { "nothing unread" } else { "no notifications" }));
        }
        for (i, n) in shown.iter().enumerate() {
            list = list.child(self.row(i, n, cx));
        }
        let body = div().flex().flex_col().child(filters).child(tools).child(list).children(probe("notifications"));
        crate::ui::scroll_body("NoticesPanel", body.into_any_element())
    }
}
