//! The status bar (app/ui/tasks.tsx StatusBar): status, mode, compaction jobs, working tasks, queue; cost with the
//! budget bar, active time, deadline, tokens, messages, depth, and the model/effort pickers (they reach a running turn
//! at its next model call on the same backend).
use crate::dialogs;
use crate::fmt;
use crate::state::*;
use crate::theme::c;
use crate::ui::*;
use crate::workspace::Workspace;
use gpui::*;
use crate::state::View;
use gpui::Context;
use crate::kit::button::Button;
use crate::kit::menu::PopupMenuItem;
use optchat_proto::metrics::TOKENS;
use optchat_proto::types::*;

pub fn render(w: &mut Workspace, _window: &mut Window, cx: &mut Context<Workspace>) -> impl IntoElement {
    let k = c(cx).clone();
    let s = w.state.read(cx);
    let narrow = w.narrow;
    let info = s.info().cloned();
    let t = info.as_ref().map(|i| i.task.clone());
    let working = s.tasks.values().filter(|x| x.running).count();
    let offline = s.conn != optchat_proto::api::Conn::Open;
    // like the page: the connection first, then the turn, then the task
    let status = if s.hub.is_none() {
        if s.meta.is_empty() { "no workspace: add an engine and connect one".to_string() } else if s.meta.get(&s.active).is_some_and(|m| m.no_token) { "no token: connect again".into() } else { "starting".into() }
    } else if offline {
        if s.conn == optchat_proto::api::Conn::Closed { "disconnected · reconnecting".into() } else { "connecting".into() }
    } else if let Some(r) = &s.cur.running {
        if r.status == "waiting" { "settling".into() } else { "running".into() }
    } else if info.as_ref().map(|i| i.waiting).unwrap_or(false) {
        "waiting for a slot".into()
    } else {
        t.as_ref().map(|t| t.status.as_str().to_string()).unwrap_or("…".into())
    };
    let running = s.cur.running.is_some();
    let paused = t.as_ref().map(|t| t.status == TaskStatus::Paused).unwrap_or(false);
    let bg = if running { k.status_running } else if paused { k.status_paused } else { k.rail };
    let fg = if running || paused { k.text } else { k.muted };
    let item = || div().flex().items_center().gap(px(5.)).px(px(7.)).h_full().whitespace_nowrap();
    let mut left = div().flex().items_center().h_full().min_w_0().overflow_hidden();
    let sdot: AnyElement = if s.hub.is_none() || s.conn == optchat_proto::api::Conn::Closed {
        dot(k.red, 7.).into_any_element()
    } else if offline {
        pulse("sbdot", dot(k.amber, 7.)).into_any_element()
    } else if running {
        pulse("sbdot", dot(k.green, 7.)).into_any_element()
    } else {
        dot(k.green.opacity(0.8), 7.).into_any_element()
    };
    // the workspace on screen (click: the workspace menu)
    if let Some(m) = s.meta.get(&s.active) {
        let hover = k.hover.opacity(0.5);
        let engine = s.engines.iter().find(|e| e.cfg.endpoint == m.endpoint).map(|e| e.cfg.name.clone()).unwrap_or_default();
        left = left.child(
            item()
                .id("sb-ws")
                .cursor_pointer()
                .hover(move |s| s.bg(hover))
                .child(icon("layers", 13., fg))
                .child(m.ws.clone())
                .children(probe("status.ws"))
                .tooltip(tooltip(format!("workspace {} on {engine} (click to switch)", m.ws)))
                .on_click(cx.listener(|w, _, _, cx| crate::workspaces::open_panel(w, cx))),
        );
    }
    left = left.child(item().child(sdot).child(status.clone()).children(probe("status.text")));
    if let Some(t) = &t {
        if t.parent.is_some() && !narrow {
            left = left.child(item().child(icon("git-fork", 13., fg)).child(format!("#{} {}", t.id, t.mode.as_str())));
        }
    }
    if !s.busy.is_empty() {
        left = left.child(item().child(spinner("sbspin", 12., fg)).child(if narrow { s.busy.len().to_string() } else { format!("summarizing {}", s.busy.len()) }));
    }
    if working > 0 && !narrow {
        left = left.child(item().child(icon("zap", 13., fg)).child(format!("{working} working")));
    }
    if !s.cur.queue.is_empty() {
        left = left.child(item().child(format!("{} queued", s.cur.queue.len())));
    }
    let mut right = div().flex().items_center().h_full().flex_none();
    // unread notices of this workspace, a dot per level (click: the Notifications view)
    {
        let u = s.unread();
        let hover = k.hover.opacity(0.5);
        let mut n = item().id("sb-notices").cursor_pointer().hover(move |s| s.bg(hover)).gap(px(7.)).children(probe("status.notices"));
        let mut any = false;
        clear_probes("status.notice.");
        for (lv, cnt, col) in [("error", u.error, k.red), ("warn", u.warn, k.amber), ("info", u.info, k.muted), ("ok", u.ok, k.green)] {
            if cnt > 0 {
                any = true;
                n = n.child(div().flex().items_center().gap(px(3.)).child(dot(col, 7.)).child(cnt.to_string()).children(probe(format!("status.notice.{lv}"))));
            }
        }
        if !any {
            n = n.child(icon("bell", 13., fg.opacity(0.7)));
        }
        if probes_on() {
            set_state_extra("statusNotices", serde_json::json!(u));
        }
        right = right.child(n.tooltip(tooltip(format!("notifications: {} unread (click to show them)", u.total()))).on_click(cx.listener(|w, _, _, cx| w.pick(View::Notifications, cx))));
    }
    if probes_on() {
        let cost = sub(info.as_ref(), "usd") as f64 / 1e6;
        let b = t.as_ref().and_then(|t| t.budget.usd);
        let pct = b.map(|b| (100.0 * cost / b).min(100.0));
        let col = pct.map(|p| if p >= 100.0 { "red" } else if p > 80.0 { "amber" } else { "accent" });
        let mode = t.as_ref().filter(|t| t.parent.is_some() && !narrow).map(|t| format!("#{} {}", t.id, t.mode.as_str()));
        let j = serde_json::json!({ "status": status, "mode": mode, "summarizing": s.busy.len(), "working": if narrow { 0 } else { working }, "queued": s.cur.queue.len(), "budget": b, "budgetPct": pct, "budgetColor": col, "cost": cost });
        set_state_extra("statusbar", j);
        crate::ui::seen("statusSeen", &status);
        if !s.cur.queue.is_empty() {
            crate::ui::seen("queuedSeen", &format!("{} queued", s.cur.queue.len()));
        }
    }
    if let Some(t) = &t {
        let cost = sub(info.as_ref(), "usd") as f64 / 1e6;
        let pct = t.budget.usd.map(|b| (100.0 * cost / b).min(100.0));
        let hover = k.hover.opacity(0.5);
        let mut costi = item().id("sb-cost").cursor_pointer().hover(move |s| s.bg(hover)).child(icon("coins", 13., fg)).child(fmt::usd(cost)).children(probe("status.cost"));
        if let Some(b) = t.budget.usd {
            let p = pct.unwrap_or(0.0);
            let col = if p >= 100.0 { k.red } else if p > 80.0 { k.amber } else { k.accent };
            costi = costi.child(format!("/ {}", fmt::usd(b))).child(div().w(px(40.)).h(px(4.)).rounded(px(2.)).bg(k.line2).child(div().h_full().rounded(px(2.)).w(px(40. * p as f32 / 100.)).bg(col)).children(probe("status.budgetbar")));
        }
        right = right.child(costi.tooltip(tooltip(format!("cost of this task and its subtree (own {}), API-equivalent. Click for all metrics and the budget.", fmt::usd(own(info.as_ref(), "usd") as f64 / 1e6)))).on_click(cx.listener(|w, _, window, cx| dialogs::open_metrics(w, window, cx))));
        let at = sub(info.as_ref(), "time.active");
        let hover2 = k.hover.opacity(0.5);
        right = right.child(
            item()
                .id("sb-time")
                .cursor_pointer()
                .hover(move |s| s.bg(hover2))
                .child(icon("clock", 13., fg))
                .child(format!("{}{}", fmt::ms(at), t.budget.minutes.map(|m| format!(" / {}", fmt::ms((m * 60_000.0) as i64))).unwrap_or_default()))
                .tooltip(tooltip("active time of this task and its subtree (agent-time: parallel work adds up)"))
                .on_click(cx.listener(|w, _, window, cx| dialogs::open_metrics(w, window, cx))),
        );
        if let Some(d) = &t.budget.deadline {
            let left_min = ((iso_ms(d) - now_ms()) as f64 / 60_000.0).round() as i64;
            right = right.child(item().text_color(if left_min < 0 { k.red } else { fg }).child(if left_min < 0 { "deadline passed".to_string() } else { format!("{left_min} min to deadline") }));
        }
        if !narrow {
            let tok: i64 = TOKENS.iter().map(|m| sub(info.as_ref(), m)).sum();
            right = right.child(item().id("sb-tok").child(format!("{} tok", fmt::tokens(tok))).tooltip(tooltip("tokens of this task and its subtree")));
            let msgs = if s.cur.loaded { s.cur.t.to_string() } else { "—".into() };
            let inh = s.cur.g - s.cur.t;
            right = right.child(item().id("sb-msgs").child(format!("{msgs} msgs")).tooltip(tooltip(format!("messages in this task's log{}", if inh > 0 { format!(" (+{inh} inherited)") } else { String::new() }))));
            right = right.child(item().id("sb-depth").child(format!("depth {}", t.settings.depth)).tooltip(tooltip("levels of child tasks it may create")));
        }
        if !t.status.terminal() {
            if let Some(set) = s.cur.settings.clone() {
                let model_label = s.model(&set.model).map(|m| m.label.clone()).unwrap_or(set.model.clone());
                let models = s.models.clone();
                let task = t.id;
                let st = w.state.clone();
                let st2 = w.state.clone();
                let cur_model = set.model.clone();
                right = right.child(
                    div().h_full().flex().items_center().children(probe("status.model")).child(
                        Button::new("sb-model")
                            .ghost()
                            .xsmall()
                            .icon("cpu")
                            .label(model_label)
                            .dropdown_caret(true)
                            .dropdown_menu(move |mut m, _, _| {
                                for (title, backend) in [("Claude Code", Backend::ClaudeCode), ("Codex", Backend::Codex)] {
                                    m = m.label(title);
                                    for mi in models.iter().filter(|x| x.backend == backend) {
                                        let id = mi.id.clone();
                                        let st = st.clone();
                                        m = m.item(PopupMenuItem::new(mi.label.clone()).checked(mi.id == cur_model).on_click(move |_, _, cx| {
                                            let id = id.clone();
                                            st.update(cx, |s, cx| s.command(cx, false, move |h| async move { h.update_settings(task, SettingsPatch { model: Some(id), ..Default::default() }).await }))
                                        }));
                                    }
                                }
                                m.max_h(px(420.)).scrollable(true)
                            }),
                    ),
                );
                let efforts = s.model(&set.model).map(|m| m.efforts.clone()).unwrap_or_default();
                let cur_e = set.effort.clone();
                right = right.child(
                    div().h_full().flex().items_center().children(probe("status.effort")).child(Button::new("sb-effort").ghost().xsmall().label(set.effort.clone()).dropdown_caret(true).dropdown_menu(move |mut m, _, _| {
                        for e in &efforts {
                            let st = st2.clone();
                            let ev = e.clone();
                            m = m.item(PopupMenuItem::new(e.clone()).checked(*e == cur_e).on_click(move |_, _, cx| {
                                let ev = ev.clone();
                                st.update(cx, |s, cx| s.command(cx, false, move |h| async move { h.update_settings(task, SettingsPatch { effort: Some(ev), ..Default::default() }).await }))
                            }));
                        }
                        m
                    })),
                );
            }
        }
    }
    div()
        .flex()
        .flex_none()
        .items_center()
        .justify_between()
        .h(px(26.))
        .bg(bg)
        .border_t_1()
        .border_color(k.line)
        .text_size(px(11.5))
        .text_color(fg)
        .child(left)
        .child(right)
        .children(probe("statusbar"))
}
