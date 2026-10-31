//! The narrow / phone layout (iPhone, iPad portrait or split, narrow desktop windows); approved design, variation A
//! with status option 1 (optchat-design: storyboard-bar-A, iphone-*, status-map):
//! - one bottom bar: Tasks · Memory · [chat button] · Alerts · More; panels open as full-screen screens with a large
//!   title and "‹ Chat"; More holds the other views (Context, Log, Models, Settings, Workspaces, Status, New Task).
//! - the chat button: tap → the composer opens; tap outside → it closes (a draft floats above the chat as a card and
//!   the button becomes ^: tap sends and runs); tap the card → edit; long-press → dictation (button ✕, words stream
//!   into the floating composer; ✕ stops → ^ if anything was said); hold-and-release = push-to-talk; double-tap =
//!   send (while dictating: stop and send; a paused task: resume).
//! - header: workspace avatar (→ Workspaces), task title (tap → task switcher; swipe → next / previous open task),
//!   subtitle "Working · 2 jobs" or "Reconnecting…", cost (→ Status, everything from the desktop status bar).
//! - gestures: an edge swipe from the left opens the last panel; from the right, "‹ Chat" or tapping the active tab
//!   again returns to the chat. Mouse drags and trackpad swipes on the desktop, touch on iOS.
use crate::fmt;
use crate::kit::button::Button;
use crate::kit::menu::PopupMenuItem;
use crate::state::*;
use crate::theme::c;
use crate::ui::*;
use crate::workspace::Workspace;
use gpui::prelude::FluentBuilder;
use gpui::*;
use crate::state::View;
use optchat_proto::types::*;
use gpui::Context;
use std::time::{Duration, Instant};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Screen {
    Panel(View),
    More,
    Status,
}
impl Screen {
    pub fn title(&self) -> &'static str {
        match self {
            Screen::Panel(View::Notifications) => "Alerts",
            Screen::Panel(v) => v.label(),
            Screen::More => "More",
            Screen::Status => "Status",
        }
    }
    pub fn key(&self) -> String {
        match self {
            Screen::Panel(v) => v.label().to_lowercase(),
            Screen::More => "more".into(),
            Screen::Status => "status".into(),
        }
    }
}

pub struct Phone {
    /// the screen over the chat (None: the chat)
    pub screen: Option<Screen>,
    /// the last panel shown (a left edge swipe brings it back)
    pub last: Screen,
    /// the composer is open (keyboard up on a phone)
    pub expanded: bool,
    press: Option<Instant>,
    press_seq: u64,
    long: bool,
    dict_started: Option<Instant>,
    last_tap: Option<Instant>,
    /// a drag from a screen edge: (start x, from the left edge)
    edge: Option<(f32, bool)>,
    /// a horizontal trackpad swipe in progress
    swipe: f32,
    title_down: Option<f32>,
    title_pan: f32,
}
impl Default for Phone {
    fn default() -> Self {
        Phone { screen: None, last: Screen::Panel(View::Tasks), expanded: false, press: None, press_seq: 0, long: false, dict_started: None, last_tap: None, edge: None, swipe: 0., title_down: None, title_pan: 0. }
    }
}

const LONG_PRESS: Duration = Duration::from_millis(450);
const PUSH_TO_TALK: Duration = Duration::from_millis(1200);
const DOUBLE_TAP: Duration = Duration::from_millis(320);

fn draft(w: &Workspace, cx: &App) -> String {
    w.composer.read(cx).input.read(cx).value().to_string()
}
fn dictating(w: &Workspace, cx: &App) -> bool {
    w.state.read(cx).dict.state != "idle"
}

impl Workspace {
    pub fn phone_open(&mut self, s: Screen, cx: &mut Context<Self>) {
        if self.phone.screen == Some(s) {
            // the active tab again: back to the chat
            self.phone.screen = None;
        } else {
            self.phone.screen = Some(s);
            if let Screen::Panel(v) = s {
                self.state.update(cx, |st, _| st.prefs.view = v);
            }
            if s != Screen::More {
                self.phone.last = s;
            }
            self.phone.expanded = false;
        }
        cx.notify();
    }
    pub fn phone_back(&mut self, cx: &mut Context<Self>) {
        self.phone.screen = None;
        cx.notify();
    }
    fn phone_expand(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.phone.expanded = true;
        self.phone.screen = None;
        self.composer.update(cx, |c, cx| c.focus(window, cx));
        cx.notify();
    }
    fn phone_collapse(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if !self.phone.expanded {
            return;
        }
        self.phone.expanded = false;
        window.focus(&self.focus, cx);
        window.dismiss_virtual_keyboard();
        cx.notify();
    }
    fn phone_send(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.composer.update(cx, |c, cx| c.submit(true, window, cx));
        self.phone.expanded = false;
        self.phone.screen = None;
        cx.notify();
    }
    fn chat_down(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.phone.press = Some(Instant::now());
        self.phone.press_seq += 1;
        let seq = self.phone.press_seq;
        cx.spawn_in(window, async move |this, cx| {
            Timer::after(LONG_PRESS).await;
            let _ = this.update_in(cx, |w, window, cx| {
                if w.phone.press.is_some() && w.phone.press_seq == seq && !dictating(w, cx) {
                    // long-press: dictation into the floating composer
                    w.chat_long_started(window, cx);
                }
            });
        })
        .detach();
    }
    /// a long press from the platform (iOS: LongPressEvent; the desktop's timer path does the same)
    fn chat_long_started(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.phone.long || dictating(self, cx) {
            return;
        }
        self.phone.long = true;
        self.phone.press.get_or_insert_with(Instant::now);
        self.phone.dict_started = Some(Instant::now());
        // the keyboard goes down; the words stream into the floating card (as with the keyboard down)
        self.phone_collapse(window, cx);
        self.phone.screen = None;
        self.composer.update(cx, |c, cx| c.start_dictation(window, cx));
        cx.notify();
    }
    /// the long press ended: held from the touch's start for ≥ 1.2 s → push-to-talk ends with the release
    fn chat_long_ended(&mut self, touch_began: Instant, cx: &mut Context<Self>) {
        if !self.phone.long {
            return;
        }
        self.phone.long = false;
        self.phone.press = None;
        if touch_began.elapsed() >= PUSH_TO_TALK {
            self.composer.update(cx, |c, cx| c.stop_dictation(cx));
        }
        cx.notify();
    }
    fn chat_up(&mut self, clicks: usize, window: &mut Window, cx: &mut Context<Self>) {
        let Some(_) = self.phone.press.take() else { return };
        if self.phone.long {
            self.phone.long = false;
            // held all along: push-to-talk ends with the release
            if self.phone.dict_started.map(|t| t.elapsed() >= PUSH_TO_TALK).unwrap_or(false) {
                self.composer.update(cx, |c, cx| c.stop_dictation(cx));
            }
            cx.notify();
            return;
        }
        // a double tap: the platform's click count (iOS taps carry 2) or our own timing
        let double = clicks >= 2 || self.phone.last_tap.map(|t| t.elapsed() < DOUBLE_TAP).unwrap_or(false);
        self.phone.last_tap = Some(Instant::now());
        if double {
            // double-tap: send (stops a dictation first; resumes a paused task); the first tap may have sent already
            self.phone.last_tap = None;
            let paused = self.state.read(cx).info().map(|i| i.task.status == TaskStatus::Paused).unwrap_or(false);
            if dictating(self, cx) || !draft(self, cx).trim().is_empty() || paused {
                self.phone_send(window, cx);
            }
            return;
        }
        if dictating(self, cx) {
            // ✕: stop; what was said stays as the draft (^)
            self.composer.update(cx, |c, cx| c.stop_dictation(cx));
        } else if !draft(self, cx).trim().is_empty() && !self.phone.expanded {
            self.phone_send(window, cx);
        } else if self.phone.expanded {
            if draft(self, cx).trim().is_empty() {
                self.phone_collapse(window, cx);
            } else {
                self.phone_send(window, cx);
            }
        } else {
            self.phone_expand(window, cx);
        }
        cx.notify();
    }
    /// a swipe on the title: next (left) or previous (right) open task
    fn title_swipe(&mut self, dx: f32, window: &mut Window, cx: &mut Context<Self>) -> bool {
        if dx.abs() < 40. {
            return false;
        }
        crate::dialogs::cycle_tab(self, if dx < 0. { 1 } else { -1 }, window, cx);
        true
    }
}

/// The whole narrow layout (the root draws the overlays above it).
pub fn render(w: &mut Workspace, window: &mut Window, cx: &mut Context<Workspace>) -> AnyElement {
    let k = c(cx).clone();
    let screen = w.phone.screen;
    let width = f32::from(window.viewport_size().width);
    let dr = draft(w, cx);
    let dict = dictating(w, cx);
    let floating = !w.phone.expanded && (!dr.trim().is_empty() || dict);
    set_state_extra("phone", serde_json::json!({ "screen": screen.map(|s| s.key()), "expanded": w.phone.expanded, "floating": floating, "draft": dr, "dictating": dict, "button": if dict { "stop" } else if floating || (w.phone.expanded && !dr.trim().is_empty()) { "send" } else { "chat" }, "last": w.phone.last.key() }));
    let mut col = div().flex().flex_col().size_full().relative().bg(k.bg);
    col = match screen {
        None => col.child(header(w, window, cx)).child(
            div()
                .flex_1()
                .min_h_0()
                .relative()
                .child(AnyView::from(w.feed.clone()).cached(StyleRefinement::default().size_full()))
                .child(probe("feed"))
                .on_mouse_down(MouseButton::Left, cx.listener(|w, _, window, cx| w.phone_collapse(window, cx))),
        ),
        Some(s) => col.child(screen_view(w, s, window, cx)),
    };
    if w.phone.expanded && screen.is_none() {
        col = col.child(
            div()
                .flex_none()
                .border_t_1()
                .border_color(k.line)
                .bg(k.bg2)
                .child({
                    let (m, e) = model_effort(w, "pc", cx);
                    div().flex().items_center().gap(px(2.)).px(px(8.)).pt(px(6.)).child(m).child(e).child(probe("phone.accessory"))
                })
                .child(w.composer.clone())
                .child(probe("phone.composer"))
                .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation()),
        );
    } else if floating {
        col = col.child(card(w, &dr, dict, cx));
    }
    col = col.child(bar(w, cx));
    // edge swipes (mouse drags from an edge; trackpad swipes) and the root's own drag state
    col.id("phone")
        .on_mouse_down(MouseButton::Left, cx.listener(move |w, e: &MouseDownEvent, _, _| {
            let x = f32::from(e.position.x);
            w.phone.edge = if x < 24. { Some((x, true)) } else if x > width - 24. { Some((x, false)) } else { None };
        }))
        .on_mouse_up(MouseButton::Left, cx.listener(|w, e: &MouseUpEvent, _, cx| {
            if let Some((x0, left)) = w.phone.edge.take() {
                let dx = f32::from(e.position.x) - x0;
                if left && dx > 60. && w.phone.screen.is_none() {
                    let last = w.phone.last;
                    w.phone_open(last, cx);
                } else if !left && dx < -60. && w.phone.screen.is_some() {
                    w.phone_back(cx);
                }
            }
        }))
        .on_scroll_wheel(cx.listener(|w, e: &ScrollWheelEvent, window, cx| {
            let d = e.delta.pixel_delta(window.line_height());
            if f32::from(d.x).abs() <= f32::from(d.y).abs() {
                return;
            }
            w.phone.swipe += f32::from(d.x);
            if w.phone.swipe > 90. && w.phone.screen.is_none() {
                w.phone.swipe = 0.;
                let last = w.phone.last;
                w.phone_open(last, cx);
            } else if w.phone.swipe < -90. && w.phone.screen.is_some() {
                w.phone.swipe = 0.;
                w.phone_back(cx);
            }
            if matches!(e.touch_phase, TouchPhase::Ended) {
                w.phone.swipe = 0.;
            }
        }))
        .into_any_element()
}

fn status_line(s: &AppState) -> (String, bool) {
    if s.hub.is_some() && s.conn != optchat_proto::api::Conn::Open {
        return ("Reconnecting…".into(), true);
    }
    let info = s.info();
    let mut t = if let Some(r) = &s.cur.running {
        if r.status == "waiting" { "Settling".to_string() } else { "Working".to_string() }
    } else if info.map(|i| i.waiting).unwrap_or(false) {
        "Waiting for a slot".to_string()
    } else {
        match info.map(|i| i.task.status) {
            Some(TaskStatus::Paused) => "Paused".into(),
            Some(TaskStatus::Done) => "Done".into(),
            Some(TaskStatus::Canceled) => "Canceled".into(),
            Some(TaskStatus::Pending) => "Pending".into(),
            Some(TaskStatus::Active) => "Working".into(),
            _ => "Idle".into(),
        }
    };
    let jobs = s.busy.len();
    if jobs > 0 {
        t.push_str(&format!(" · {jobs} job{}", if jobs == 1 { "" } else { "s" }));
    }
    (t, false)
}

fn header(w: &mut Workspace, _window: &mut Window, cx: &mut Context<Workspace>) -> impl IntoElement {
    let k = c(cx).clone();
    let s = w.state.read(cx);
    let title = s.info().map(|i| task_name(&i.task)).unwrap_or_else(|| if s.hub.is_none() { "OptChat".into() } else { "…".into() });
    let (sub, offline) = status_line(s);
    let working = s.cur.running.is_some();
    let cost = sub_metric(s);
    let alert = s.background_alert();
    set_state_extra("phoneHeader", serde_json::json!({ "title": title, "subtitle": sub, "cost": cost }));
    let (runs, folds_open) = w.feed.read(cx).fold_state();
    div()
        .flex_none()
        .h(px(56.))
        .px(px(14.))
        .flex()
        .items_center()
        .gap(px(10.))
        .border_b_1()
        .border_color(k.line)
        .child(
            div()
                .id("ph-avatar")
                .relative()
                .cursor_pointer()
                .child(crate::feed::logo(30., cx).rounded_full())
                .when(alert || offline, |d| d.child(div().absolute().top(px(-2.)).right(px(-2.)).child(dot(if offline { k.amber } else { k.red }, 9.).border_2().border_color(k.bg))))
                .child(probe("phone.avatar"))
                .on_click(cx.listener(|w, _, _, cx| w.phone_open(Screen::Panel(View::Workspaces), cx))),
        )
        .child(
            div()
                .id("ph-title")
                .flex_1()
                .min_w_0()
                .flex()
                .flex_col()
                .items_center()
                .cursor_pointer()
                .child(div().flex().items_center().gap(px(4.)).max_w_full().child(div().text_size(px(16.)).font_weight(FontWeight::SEMIBOLD).overflow_hidden().whitespace_nowrap().text_ellipsis().child(title)).child(icon("chevron-down", 12., k.muted)))
                .child(div().flex().items_center().gap(px(5.)).text_size(px(12.)).text_color(if offline { k.amber } else { k.muted }).child(dot(if offline { k.amber } else if working { k.accent } else { k.faint }, 6.)).child(sub).child(probe("phone.subtitle")))
                .child(probe("phone.title"))
                .child(title_pan_area(cx.entity().downgrade()))
                .on_scroll_wheel(cx.listener(|w, e: &ScrollWheelEvent, window, cx| {
                    // iOS pans (and trackpad swipes) arrive as scroll events: switch once past 40 pt, at the end
                    let d = e.delta.pixel_delta(window.line_height());
                    if f32::from(d.x).abs() > f32::from(d.y).abs() {
                        w.phone.title_pan += f32::from(d.x);
                        cx.stop_propagation();
                    }
                    if matches!(e.touch_phase, TouchPhase::Ended) {
                        let dx = std::mem::take(&mut w.phone.title_pan);
                        // content moves with the finger: a pan to the left has a negative delta
                        w.title_swipe(dx, window, cx);
                    }
                }))
                .on_mouse_down(MouseButton::Left, cx.listener(|w, e: &MouseDownEvent, _, _| w.phone.title_down = Some(f32::from(e.position.x))))
                .on_mouse_up(MouseButton::Left, cx.listener(|w, e: &MouseUpEvent, window, cx| {
                    let Some(x0) = w.phone.title_down.take() else { return };
                    let dx = f32::from(e.position.x) - x0;
                    if !w.title_swipe(dx, window, cx) {
                        // a tap: the task switcher
                        crate::dialogs::open_palette(w, true, window, cx);
                    }
                })),
        )
        .when(runs, |d| {
            // runs of internal messages in the chat: fold them all into one line each, or open them all
            d.child(
                icon_btn("ph-folds", if folds_open { "minimize-2" } else { "maximize-2" }, 17., cx)
                    .child(probe("phone.folds"))
                    .on_click(cx.listener(|w, _, _, cx| w.feed.update(cx, |f, cx| f.toggle_all(cx)))),
            )
        })
        .child(
            div()
                .id("ph-cost")
                .cursor_pointer()
                .text_size(px(15.))
                .text_color(k.accent)
                .child(fmt::usd(cost))
                .child(probe("phone.cost"))
                .on_click(cx.listener(|w, _, _, cx| w.phone_open(Screen::Status, cx))),
        )
}

fn sub_metric(s: &AppState) -> f64 {
    crate::state::sub(s.info(), "usd") as f64 / 1e6
}

/// the draft (or the words being dictated) floating above the chat; a tap edits it
fn card(w: &mut Workspace, dr: &str, dict: bool, cx: &mut Context<Workspace>) -> impl IntoElement {
    let k = c(cx).clone();
    let s = w.state.read(cx);
    let to = s.info().map(|i| if i.task.parent.is_none() { "To main chat".to_string() } else { format!("To #{}", i.task.id) }).unwrap_or_default();
    let shown = if dict { s.dict.text.clone().unwrap_or_default() } else { dr.to_string() };
    div()
        .id("ph-card")
        .flex_none()
        .mx(px(12.))
        .mb(px(8.))
        .px(px(14.))
        .py(px(10.))
        .rounded(px(14.))
        .bg(k.panel)
        .border_1()
        .border_color(k.line2)
        .cursor_pointer()
        .child(div().flex().items_center().gap(px(6.)).text_size(px(11.5)).text_color(if dict { k.red } else { k.muted }).when(dict, |d| d.child(pulse("ph-rec", dot(k.red, 7.)))).child(if dict { if s.dict.state == "finishing" { "Finishing…".to_string() } else { "Listening".to_string() } } else { to }))
        .child(div().mt(px(3.)).text_size(px(14.5)).line_height(px(20.)).text_color(k.text).max_h(px(84.)).overflow_hidden().child(if shown.is_empty() { "…".to_string() } else { shown }))
        .child(probe("phone.card"))
        .on_click(cx.listener(|w, _, window, cx| {
            if !dictating(w, cx) {
                w.phone_expand(window, cx)
            }
        }))
}

fn bar(w: &mut Workspace, cx: &mut Context<Workspace>) -> impl IntoElement {
    let k = c(cx).clone();
    let s = w.state.read(cx);
    let u = s.unread();
    let alerts = u.total();
    let screen = w.phone.screen;
    let dr = draft(w, cx);
    let dict = s.dict.state != "idle";
    let send = !dr.trim().is_empty() && !dict;
    let tab = |id: &'static str, label: &'static str, ic: &'static str, target: Screen, badge: i64, cx: &mut Context<Workspace>| {
        let on = screen == Some(target);
        let col = if on { k.accent } else { k.muted };
        div()
            .id(id)
            .relative()
            .flex_1()
            .flex()
            .flex_col()
            .items_center()
            .gap(px(3.))
            .cursor_pointer()
            .child(icon(ic, 21., col))
            .child(div().text_size(px(10.5)).text_color(col).child(label))
            .when(badge > 0, |d| d.child(div().absolute().top(px(-4.)).left(relative(0.55)).min_w(px(17.)).h(px(17.)).px(px(4.)).rounded_full().bg(k.red).text_color(white()).text_size(px(10.)).font_weight(FontWeight::BOLD).flex().items_center().justify_center().child(badge.to_string())))
            .child(probe(format!("phone.tab.{}", target.key())))
            .on_click(cx.listener(move |w, _, _, cx| w.phone_open(target, cx)))
    };
    let (ic, bg) = if dict { ("x", k.red) } else if send { ("arrow-up", k.accent2) } else { ("message-square", k.accent2) };
    div()
        .flex_none()
        .h(px(64.))
        .px(px(8.))
        .flex()
        .items_center()
        .border_t_1()
        .border_color(k.line)
        .bg(k.bg2)
        .child(tab("ph-tasks", "Tasks", "list-tree", Screen::Panel(View::Tasks), 0, cx))
        .child(tab("ph-memory", "Memory", "brain", Screen::Panel(View::Memory), 0, cx))
        .child(
            div().flex_1().flex().justify_center().child(
                div()
                    .id("ph-chat")
                    .size(px(52.))
                    .rounded_full()
                    .bg(bg)
                    .flex()
                    .items_center()
                    .justify_center()
                    .cursor_pointer()
                    .shadow(vec![BoxShadow { color: bg.opacity(0.45), offset: point(px(0.), px(4.)), blur_radius: px(16.), spread_radius: px(0.), inset: false }])
                    .child(icon(ic, 23., k.on_accent))
                    .child(probe("phone.chat"))
                    .child(chat_button_events(cx.entity().downgrade())),
            ),
        )
        .child(tab("ph-alerts", "Alerts", "bell", Screen::Panel(View::Notifications), alerts, cx))
        .child(tab("ph-more", "More", "ellipsis", Screen::More, 0, cx))
}

/// a full-screen panel: "‹ Chat", a large title, the view
fn screen_view(w: &mut Workspace, scr: Screen, window: &mut Window, cx: &mut Context<Workspace>) -> AnyElement {
    let k = c(cx).clone();
    let cached = |v: AnyView| v.cached(StyleRefinement::default().size_full()).into_any_element();
    let body: AnyElement = match scr {
        Screen::Panel(View::Tasks) => cached(w.tasks_panel.clone().into()),
        Screen::Panel(View::Memory) => cached(w.memory.clone().into()),
        Screen::Panel(View::Context) => cached(w.context.clone().into()),
        Screen::Panel(View::Log) => cached(w.log.clone().into()),
        Screen::Panel(View::Models) => cached(w.models.clone().into()),
        Screen::Panel(View::Settings) => cached(w.settings.clone().into()),
        Screen::Panel(View::Notifications) => cached(w.notices_panel.clone().into()),
        Screen::Panel(View::Workspaces) => cached(w.ws_panel.clone().into()),
        Screen::More => more(w, cx).into_any_element(),
        Screen::Status => status(w, window, cx).into_any_element(),
    };
    let s = w.state.read(cx);
    let scope_all = s.prefs.task_scope_all;
    let action: Option<AnyElement> = match scr {
        Screen::Panel(View::Tasks) => Some(icon_btn("ph-new", "plus", 20., cx).child(probe("phone.new")).on_click(cx.listener(|w, _, window, cx| crate::dialogs::open_new_task(w, None, window, cx))).into_any_element()),
        _ => None,
    };
    let seg = |id: &'static str, label: &'static str, on: bool, val: bool, cx: &mut Context<Workspace>| {
        let kk = k.clone();
        div()
            .id(id)
            .flex_1()
            .flex()
            .justify_center()
            .py(px(5.))
            .rounded(px(7.))
            .text_size(px(13.))
            .cursor_pointer()
            .when(on, |d| d.bg(kk.selected).text_color(kk.text))
            .when(!on, |d| d.text_color(kk.muted))
            .child(label)
            .on_click(cx.listener(move |w, _, _, cx| {
                w.state.update(cx, |s, cx| {
                    s.prefs.task_scope_all = val;
                    s.save_prefs();
                    cx.notify();
                })
            }))
    };
    div()
        .flex_1()
        .min_h_0()
        .flex()
        .flex_col()
        .child(
            div()
                .flex_none()
                .flex()
                .items_center()
                .justify_between()
                .px(px(12.))
                .pt(px(10.))
                .child(
                    div()
                        .id("ph-back")
                        .flex()
                        .items_center()
                        .gap(px(2.))
                        .cursor_pointer()
                        .text_size(px(15.))
                        .text_color(k.accent)
                        .child(icon("chevron-left", 18., k.accent))
                        .child("Chat")
                        .child(probe("phone.back"))
                        .on_click(cx.listener(|w, _, _, cx| w.phone_back(cx))),
                )
                .children(action),
        )
        .child(div().flex_none().px(px(16.)).pt(px(4.)).pb(px(8.)).text_size(px(30.)).font_weight(FontWeight::BOLD).child(scr.title()).child(probe("phone.screenTitle")))
        .when(scr == Screen::Panel(View::Tasks), |d| {
            d.child(div().flex_none().mx(px(14.)).mb(px(8.)).p(px(3.)).rounded(px(9.)).bg(k.panel).flex().child(seg("ph-all", "All Tasks", scope_all, true, cx)).child(seg("ph-this", "This Task", !scope_all, false, cx)))
        })
        .child(div().flex_1().min_h_0().child(body))
        .into_any_element()
}

fn group(cx: &App) -> Div {
    let k = c(cx);
    div().mx(px(14.)).mb(px(16.)).rounded(px(12.)).bg(k.panel).border_1().border_color(k.line).overflow_hidden().flex().flex_col()
}
fn group_title(t: &str, cx: &App) -> Div {
    let k = c(cx);
    div().px(px(30.)).pb(px(6.)).text_size(px(11.5)).text_color(k.muted).child(t.to_uppercase())
}
fn row(id: impl Into<ElementId>, ic: &str, tint: Hsla, label: &str, value: String, chev: bool, cx: &App) -> Stateful<Div> {
    let k = c(cx).clone();
    let hover = k.hover;
    div()
        .id(id)
        .flex()
        .items_center()
        .gap(px(12.))
        .px(px(14.))
        .min_h(px(46.))
        .border_b_1()
        .border_color(k.line)
        .hover(move |d| d.bg(hover))
        .child(div().size(px(28.)).rounded(px(7.)).bg(tint).flex().items_center().justify_center().child(icon(ic, 15., white())))
        .child(div().flex_1().min_w_0().text_size(px(15.)).child(label.to_string()))
        .child(div().text_size(px(14.)).text_color(k.muted).child(value))
        .when(chev, |d| d.child(icon("chevron-right", 14., k.faint)))
}

fn more(w: &mut Workspace, cx: &mut Context<Workspace>) -> impl IntoElement {
    let k = c(cx).clone();
    let ws_name = w.state.read(cx).meta.get(&w.state.read(cx).active).map(|m| m.ws.clone()).unwrap_or_default();
    let item = |id: &'static str, ic: &str, tint: Hsla, label: &str, val: String, target: Option<Screen>, cx: &mut Context<Workspace>| {
        row(id, ic, tint, label, val, true, cx).cursor_pointer().child(probe(format!("phone.more.{}", label.to_lowercase().replace(' ', "")))).on_click(cx.listener(move |w, _, window, cx| match target {
            Some(t) => w.phone_open(t, cx),
            None => crate::dialogs::open_new_task(w, None, window, cx),
        }))
    };
    div()
        .id("ph-more-list")
        .size_full()
        .overflow_y_scroll()
        .pt(px(4.))
        .child(
            group(cx)
                .child(item("pm-ctx", "scan-text", k.cyan.opacity(0.8), "Context", String::new(), Some(Screen::Panel(View::Context)), cx))
                .child(item("pm-log", "scroll-text", k.muted, "Log", String::new(), Some(Screen::Panel(View::Log)), cx))
                .child(item("pm-models", "cpu", k.violet, "Models", String::new(), Some(Screen::Panel(View::Models)), cx))
                .child(item("pm-settings", "settings", k.muted, "Settings", String::new(), Some(Screen::Panel(View::Settings)), cx)),
        )
        .child(group(cx).child(item("pm-ws", "layers", k.accent2, "Workspaces", ws_name, Some(Screen::Panel(View::Workspaces)), cx)).child(item("pm-status", "zap", k.green, "Status", String::new(), Some(Screen::Status), cx)))
        .child(group(cx).child(item("pm-new", "message-square", k.accent2, "New Task", String::new(), None, cx)))
}

/// everything the desktop status bar shows, as a screen
fn status(w: &mut Workspace, _window: &mut Window, cx: &mut Context<Workspace>) -> impl IntoElement {
    let k = c(cx).clone();
    let s = w.state.read(cx);
    let info = s.info().cloned();
    let t = info.as_ref().map(|i| i.task.clone());
    let (st, _) = status_line(s);
    let cost = sub(info.as_ref(), "usd") as f64 / 1e6;
    let chat_usd = sub_k(info.as_ref(), "usd", "chat") as f64 / 1e6;
    let mem_usd = sub_k(info.as_ref(), "usd", "compact") as f64 / 1e6;
    let tok: i64 = optchat_proto::metrics::TOKENS.iter().map(|m| sub(info.as_ref(), m)).sum();
    let active = sub(info.as_ref(), "time.active");
    let working = s.tasks.values().filter(|x| x.running).count();
    let ws = s.meta.get(&s.active).cloned();
    let engine = ws.as_ref().and_then(|m| s.engines.iter().find(|e| e.cfg.endpoint == m.endpoint)).map(|e| e.cfg.name.clone()).unwrap_or_default();
    let conn = format!("{:?}", s.conn).to_lowercase();
    let head = t.as_ref().map(|t| format!("#{} · {}", t.id, task_name(t))).unwrap_or_default();
    let mode = t.as_ref().map(|t| t.mode.as_str().to_string()).unwrap_or_default();
    let depth = t.as_ref().map(|t| format!("{} level{}", t.settings.depth, if t.settings.depth == 1 { "" } else { "s" })).unwrap_or_default();
    let budget = t.as_ref().and_then(|t| t.budget.usd).map(|b| format!(" / {}", fmt::usd(b))).unwrap_or_default();
    let msgs = s.cur.t.to_string();
    let jobs = s.busy.len();
    let (model_btn, effort_btn) = model_effort(w, "ps", cx);
    set_state_extra("phoneStatus", serde_json::json!({ "status": st, "cost": cost, "tokens": tok, "messages": msgs, "connection": conn }));
    div()
        .id("ph-status")
        .size_full()
        .overflow_y_scroll()
        .pt(px(4.))
        .child(group_title(&head, cx))
        .child(
            group(cx)
                .child(row("pst-status", "zap", k.accent2, "Status", st, false, cx))
                .child(row("pst-mode", "git-fork", k.muted, "Mode", mode, false, cx))
                .child(row("pst-msgs", "hash", k.muted, "Messages", msgs, false, cx))
                .child(row("pst-depth", "layers", k.muted, "Depth", depth, false, cx))
                .child(row("pst-jobs", "brain", k.muted, "Compaction jobs", jobs.to_string(), false, cx))
                .child(row("pst-working", "list-tree", k.muted, "Working tasks", working.to_string(), false, cx)),
        )
        .child(group_title("spend", cx))
        .child(
            group(cx)
                .child(row("pst-cost", "coins", k.green, "Cost", format!("{}{budget}", fmt::usd(cost)), false, cx))
                .child(row("pst-active", "clock", k.green, "Active time", fmt::ms(active), false, cx))
                .child(row("pst-tok", "hash", k.green, "Tokens", fmt::tokens(tok), false, cx))
                .child(row("pst-split", "coins", k.muted, "Chat · memory", format!("{} · {}", fmt::usd(chat_usd), fmt::usd(mem_usd)), false, cx)),
        )
        .child(group_title("next turn", cx))
        .child(
            group(cx)
                .child(row("pst-model", "cpu", k.violet, "Model", String::new(), false, cx).child(model_btn).child(probe("phone.status.model")))
                .child(row("pst-effort", "sliders-horizontal", k.violet, "Effort", String::new(), false, cx).child(effort_btn)),
        )
        .child(group_title("workspace", cx))
        .child(
            group(cx)
                .child(row("pst-ws", "layers", k.accent2, "Workspace", ws.map(|m| m.ws).unwrap_or_default(), false, cx))
                .child(row("pst-engine", "server", k.muted, "Engine", engine, false, cx))
                .child(row("pst-conn", "plug", if conn == "open" { k.green } else { k.amber }, "Connection", if conn == "open" { "connected".into() } else { "reconnecting…".into() }, false, cx)),
        )
}

/// The chat button's presses, taps and long presses, taken in the CAPTURE phase over its bounds: nothing under or
/// around it (the focused composer's text input, which iOS hands touches and long presses to while the keyboard is
/// up) can swallow them. Mouse down/up drive taps, double taps (click_count or timing) and the desktop's long-press
/// timer; iOS's LongPressEvent (Started after ~500 ms, Ended on release) drives dictation and hold-to-talk.
fn chat_button_events(ws: WeakEntity<Workspace>) -> impl IntoElement {
    canvas(
        |b, _, _| b,
        move |bounds, _, window, _| {
            let free = |w: &Workspace, cx: &App| w.overlay.is_none() && !crate::kit::menu::is_open(cx);
            let ws1 = ws.clone();
            window.on_mouse_event::<MouseDownEvent>(move |e, phase, window, cx| {
                if phase != DispatchPhase::Capture || e.button != MouseButton::Left || !bounds.contains(&e.position) {
                    return;
                }
                let _ = ws1.update(cx, |w, cx| {
                    if free(w, cx) {
                        cx.stop_propagation();
                        w.chat_down(window, cx);
                    }
                });
            });
            let ws2 = ws.clone();
            window.on_mouse_event::<MouseUpEvent>(move |e, phase, window, cx| {
                if phase != DispatchPhase::Capture || e.button != MouseButton::Left {
                    return;
                }
                let _ = ws2.update(cx, |w, cx| {
                    // a release anywhere ends a press that began on the button
                    if w.phone.press.is_some() && (bounds.contains(&e.position) || w.phone.long) {
                        cx.stop_propagation();
                        w.chat_up(e.click_count, window, cx);
                    }
                });
            });
            let ws3 = ws.clone();
            let began = std::rc::Rc::new(std::cell::Cell::new(None::<Instant>));
            window.on_mouse_event::<LongPressEvent>(move |e, phase, window, cx| {
                if phase != DispatchPhase::Capture || !bounds.contains(&e.start_position) {
                    return;
                }
                cx.stop_propagation();
                match e.phase {
                    TouchPhase::Started => {
                        // the touch began ~500 ms before the long press was recognised
                        began.set(Some(Instant::now() - Duration::from_millis(500)));
                        let _ = ws3.update(cx, |w, cx| w.chat_long_started(window, cx));
                    }
                    TouchPhase::Ended | TouchPhase::Cancelled => {
                        let t = began.take().unwrap_or_else(Instant::now);
                        let _ = ws3.update(cx, |w, cx| w.chat_long_ended(t, cx));
                    }
                    _ => {}
                }
            });
        },
    )
    .absolute()
    .size_full()
    .top_0()
    .left_0()
}

/// iOS: a horizontal touch drag over the title switches to the next / previous open task when it ends
fn title_pan_area(ws: WeakEntity<Workspace>) -> impl IntoElement {
    canvas(
        |b, _, _| b,
        move |bounds, _, window, _| {
            let ws = ws.clone();
            window.on_mouse_event::<TouchDragEvent>(move |e, phase, window, cx| {
                if phase != DispatchPhase::Bubble || !bounds.contains(&e.start_position) || !matches!(e.phase, TouchPhase::Ended) {
                    return;
                }
                let dx = f32::from(e.position.x - e.start_position.x);
                let _ = ws.update(cx, |w, cx| {
                    w.title_swipe(dx, window, cx);
                });
            });
        },
    )
    .absolute()
    .size_full()
    .top_0()
    .left_0()
}

/// the next turn's model and effort as menus (the composer's accessory row, the Status screen)
fn model_effort(w: &Workspace, id: &str, cx: &mut Context<Workspace>) -> (Button, Button) {
    let s = w.state.read(cx);
    let task_id = s.current;
    let set = s.cur.settings.clone();
    let models = s.models.clone();
    let model_label = set.as_ref().map(|x| s.model(&x.model).map(|m| m.label.clone()).unwrap_or(x.model.clone())).unwrap_or_default();
    let efforts = set.as_ref().and_then(|x| s.model(&x.model)).map(|m| m.efforts.clone()).unwrap_or_default();
    let st_model = w.state.clone();
    let st_effort = w.state.clone();
    let cur_model = set.as_ref().map(|x| x.model.clone()).unwrap_or_default();
    let cur_effort = set.as_ref().map(|x| x.effort.clone()).unwrap_or_default();
    let model_btn = Button::new(SharedString::from(format!("{id}-model"))).ghost().label(model_label).dropdown_caret(true).dropdown_menu(move |mut m, _, _| {
        for (title, backend) in [("Claude Code", Backend::ClaudeCode), ("Codex", Backend::Codex)] {
            m = m.label(title);
            for mi in models.iter().filter(|x| x.backend == backend) {
                let id = mi.id.clone();
                let st = st_model.clone();
                m = m.item(PopupMenuItem::new(mi.label.clone()).checked(mi.id == cur_model).on_click(move |_, _, cx| {
                    let id = id.clone();
                    st.update(cx, |s, cx| s.command(cx, false, move |h| async move { h.update_settings(task_id, SettingsPatch { model: Some(id), ..Default::default() }).await }))
                }));
            }
        }
        m.max_h(px(420.))
    });
    let effort_btn = Button::new(SharedString::from(format!("{id}-effort"))).ghost().label(cur_effort.clone()).dropdown_caret(true).dropdown_menu(move |mut m, _, _| {
        for e in &efforts {
            let st = st_effort.clone();
            let ev = e.clone();
            m = m.item(PopupMenuItem::new(e.clone()).checked(*e == cur_effort).on_click(move |_, _, cx| {
                let ev = ev.clone();
                st.update(cx, |s, cx| s.command(cx, false, move |h| async move { h.update_settings(task_id, SettingsPatch { effort: Some(ev), ..Default::default() }).await }))
            }));
        }
        m
    });
    (model_btn, effort_btn)
}
