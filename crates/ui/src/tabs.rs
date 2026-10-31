//! Editor tabs with the current task's actions at their right, the thin task bar (breadcrumb, or the reason/result
//! field of an action), and the end bar of a finished task (app/ui/tasks.tsx TabStrip, TaskBar).
use crate::dialogs;
use crate::state::*;
use crate::theme::c;
use crate::ui::*;
use crate::workspace::Workspace;
use gpui::prelude::FluentBuilder;
use gpui::*;
use gpui::Context;
use crate::kit::input::Input;
use optchat_proto::types::*;

pub fn tab_strip(w: &mut Workspace, _window: &mut Window, cx: &mut Context<Workspace>) -> impl IntoElement {
    let k = c(cx).clone();
    let menu_open = w.task_menu_open();
    let s = w.state.read(cx);
    let tabs = s.prefs.tabs.clone();
    let cur = s.current;
    let info = s.info().cloned();
    let st = info.as_ref().map(|i| i.task.status);
    let root = info.as_ref().map(|i| i.task.parent.is_none()).unwrap_or(true);
    // the current tab is scrolled into view when it changes
    if w.tab_scrolled_to != Some(cur) {
        if let Some(ix) = tabs.iter().position(|t| *t == cur) {
            w.tab_scroll.scroll_to_item(ix);
        }
        w.tab_scrolled_to = Some(cur);
    }
    let mut strip = div().id("tabscroll").flex().flex_1().min_w_0().h_full().overflow_x_scroll().track_scroll(&w.tab_scroll);
    for id in tabs {
        let x = s.tasks.get(&id).cloned();
        let on = id == cur;
        let title = x.as_ref().map(|x| task_name(&x.task)).unwrap_or(format!("#{id}"));
        let ended = x.as_ref().map(|x| x.task.status.terminal()).unwrap_or(false);
        let hover = k.hover;
        let lead: AnyElement = if id == 0 {
            icon("message-square", 14., if on { k.text } else { k.muted }).into_any_element()
        } else {
            status_dot(format!("tab{id}"), x.as_ref().map(|x| x.task.status.as_str()).unwrap_or("idle"), 7., cx)
        };
        let tip = x.as_ref().map(|x| format!("{} · #{id} · {}", x.task.title, x.task.status.as_str())).unwrap_or(format!("#{id}"));
        // keyed parent: sibling tabs' context menus (all id "context-menu") stay apart
        strip = strip.child(div().id(SharedString::from(format!("tab-k-{id}"))).flex_none().h_full().child(
            div()
                .id(SharedString::from(format!("tab-{id}")))
                .group("tab")
                .relative()
                .flex()
                .flex_none()
                .items_center()
                .gap(px(7.))
                .h_full()
                .max_w(px(220.))
                .pl(px(12.))
                .pr(px(6.))
                .border_r_1()
                .border_color(k.line)
                .cursor_pointer()
                .text_size(px(12.5))
                .text_color(if on { k.text } else if ended { k.faint } else { k.muted })
                .when(on, |d| d.bg(k.bg).child(div().absolute().top_0().left_0().right_0().h(px(2.)).bg(k.accent)))
                .when(!on, |d| d.bg(k.bg2).hover(move |s| s.bg(hover)))
                .when(!menu_open, |d| d.tooltip(tooltip(tip)))
                .child(probe(format!("tab.{id}")))
                .child(lead)
                .child(div().flex_1().min_w_0().overflow_hidden().line_clamp(1).text_ellipsis().child(title))
                .when(x.as_ref().map(|x| x.running).unwrap_or(false), |d| d.child(spinner(SharedString::from(format!("tabspin{id}")), 12., k.accent)))
                .child(
                    div()
                        .id(SharedString::from(format!("tabx-{id}")))
                        .size(px(18.))
                        .flex()
                        .items_center()
                        .justify_center()
                        .rounded(px(4.))
                        .invisible()
                        .when(on, |d| d.visible())
                        .group_hover("tab", |s| s.visible())
                        .hover(move |s| s.bg(hover))
                        .child(icon("x", 13., k.muted))
                        .on_click(cx.listener(move |w, _, window, cx| {
                            cx.stop_propagation();
                            w.state.update(cx, |s, cx| s.close_tab(id, cx));
                            w.update_title(window, cx);
                        })),
                )
                .on_click(cx.listener(move |w, _, window, cx| w.open_task(id, window, cx)))
                .on_mouse_down(MouseButton::Middle, cx.listener(move |w, _, window, cx| {
                    w.state.update(cx, |s, cx| s.close_tab(id, cx));
                    w.update_title(window, cx);
                }))
                .on_mouse_down(MouseButton::Right, {
                    let status = x.as_ref().map(|x| x.task.status).unwrap_or(TaskStatus::Idle);
                    cx.listener(move |w, e: &MouseDownEvent, window, cx| w.open_task_menu(id, status, e.position, window, cx))
                }),
        ));
    }
    let mut actions = div().flex().flex_none().items_center().gap(px(2.)).px(px(6.)).h_full().border_l_1().border_color(k.line);
    // runs of internal messages in the feed: fold them all into their one-line summaries, or open them all
    let (runs, open) = w.feed.read(cx).fold_state();
    if runs {
        actions = actions.child(
            icon_btn("act-folds", if open { "minimize-2" } else { "maximize-2" }, 14., cx)
                .tooltip(tooltip(if open { "Collapse internal messages" } else { "Expand internal messages" }))
                .child(probe("act.folds"))
                .on_click(cx.listener(|w, _, _, cx| w.feed.update(cx, |f, cx| f.toggle_all(cx)))),
        );
    }
    if let Some(st) = st {
        let term = st.terminal();
        if !term && st != TaskStatus::Paused && st != TaskStatus::Pending {
            actions = actions.child(icon_btn("act-pause", "pause", 15., cx).tooltip(tooltip("Pause this task (and its subtree)")).child(probe("act.pause")).on_click(cx.listener(|w, _, window, cx| w.start_ask("pause", window, cx))));
        }
        if st == TaskStatus::Paused {
            actions = actions.child(icon_btn("act-resume", "play", 15., cx).tooltip(tooltip("Resume")).child(probe("act.resume")).on_click(cx.listener(|w, _, window, cx| w.start_ask("resume", window, cx))));
        }
        if !root && !term {
            actions = actions.child(icon_btn("act-close", "circle-check", 15., cx).tooltip(tooltip("Close as done, with a result")).child(probe("act.close")).on_click(cx.listener(|w, _, window, cx| w.start_ask("close", window, cx))));
            actions = actions.child(icon_btn_c("act-cancel", "ban", 15., k.red, cx).tooltip(tooltip("Cancel this task (and its subtree)")).child(probe("act.cancel")).on_click(cx.listener(|w, _, window, cx| w.start_ask("cancel", window, cx))));
        }
        if !term {
            actions = actions.child(icon_btn_c("act-new", "git-branch-plus", 15., k.accent, cx).tooltip(tooltip("New task under this one (⌘N)")).child(probe("act.new")).on_click(cx.listener(|w, _, window, cx| dialogs::open_new_task(w, None, window, cx))));
        }
    }
    div().flex().flex_none().h(px(36.)).bg(k.bg2).border_b_1().border_color(k.line).child(strip).child(actions).child(probe("tabbar"))
}

pub fn task_bar(w: &mut Workspace, _window: &mut Window, cx: &mut Context<Workspace>) -> impl IntoElement {
    let k = c(cx).clone();
    let s = w.state.read(cx);
    let Some(info) = s.info().cloned() else { return div().h(px(30.)).border_b_1().border_color(k.line).into_any_element() };
    let t = info.task.clone();
    let base = div().flex().flex_none().items_center().gap(px(8.)).h(px(32.)).px(px(14.)).border_b_1().border_color(k.line).bg(k.bg).text_size(px(12.5));
    if let Some(a) = w.ask.clone() {
        let kids_live = s.children_of(t.id).iter().any(|x| !x.task.status.terminal());
        let val = w.ask_input.read(cx).value().to_string();
        let off = a.required && val.trim().is_empty();
        return base
            .bg(k.panel)
            .child(div().font_weight(FontWeight::SEMIBOLD).text_color(k.text).whitespace_nowrap().child(format!("{}{}", a.label, if t.id == 0 { String::new() } else { format!(" #{}", t.id) })))
            .child(div().flex_1().min_w_0().child(Input::new(&w.ask_input).small()).child(probe("ask.input")))
            .when((a.op == "pause" || a.op == "cancel" || a.op == "close") && kids_live, |d| d.child(div().text_color(k.muted).text_size(px(11.5)).whitespace_nowrap().child("and its subtree")))
            .child(disabled(btn_small("ask-go", a.label, if a.op == "cancel" { Tone::Danger } else { Tone::Primary }, cx), off).child(probe("ask.go")).when(!off, |d| d.on_click(cx.listener(|w, _, window, cx| w.confirm_ask(window, cx)))))
            .child(btn_small("ask-esc", "Esc", Tone::Ghost, cx).on_click(cx.listener(|w, _, _, cx| {
                w.ask = None;
                cx.notify();
            })))
            .into_any_element();
    }
    let up = s.ancestors(t.id);
    let mut crumbs = div().flex().items_center().gap(px(4.)).flex_1().min_w_0().overflow_hidden().whitespace_nowrap();
    for x in up {
        let id = x.task.id;
        let label = if id == 0 { "main chat".to_string() } else { format!("#{id} {}", x.task.title) };
        crumbs = crumbs
            .child(div().id(SharedString::from(format!("crumb{id}"))).text_color(k.muted).cursor_pointer().hover(|s| s.text_color(k.text)).max_w(px(240.)).min_w_0().overflow_hidden().line_clamp(1).text_ellipsis().child(label).on_click(cx.listener(move |w, _, window, cx| w.open_task(id, window, cx))))
            .child(icon("chevron-right", 12., k.faint));
    }
    crumbs = crumbs.child(div().flex_1().min_w_0().text_color(k.text).overflow_hidden().line_clamp(1).text_ellipsis().child(if t.id == 0 { "main chat".to_string() } else { format!("#{} {}", t.id, t.title) }));
    if t.mode == TaskMode::Continue {
        if let Some(p) = t.prev {
            crumbs = crumbs.child(div().flex().gap(px(4.)).text_color(k.muted).text_size(px(11.5)).child("· continues").child(div().id("prevlink").text_color(k.accent).cursor_pointer().child(format!("#{p}")).child(probe("taskbar.prev")).on_click(cx.listener(move |w, _, window, cx| w.open_task(p, window, cx)))));
        }
    }
    let mut bar = base.child(crumbs);
    if let Some(r) = &t.result {
        bar = bar.child(div().max_w(relative(0.5)).overflow_hidden().line_clamp(1).text_ellipsis().text_color(k.green).child(format!("→ {r}")).child(probe("taskbar.why")));
    } else if let Some(r) = &t.reason {
        let col = match t.status {
            TaskStatus::Paused => k.amber,
            TaskStatus::Canceled => k.red,
            _ => k.muted,
        };
        bar = bar.child(div().max_w(relative(0.5)).overflow_hidden().line_clamp(1).text_ellipsis().text_color(col).child(format!("{}: {r}", t.status.as_str())).child(probe("taskbar.why")));
    }
    bar.into_any_element()
}

pub fn endbar(w: &mut Workspace, cx: &mut Context<Workspace>) -> impl IntoElement {
    let k = c(cx).clone();
    let s = w.state.read(cx);
    let Some(info) = s.info().cloned() else { return div() };
    let t = info.task.clone();
    let tail = match (&t.result, &t.reason) {
        (Some(r), _) => format!(": {}", r.trim_end_matches(['.', ' '])),
        (None, Some(r)) => format!(" ({})", r.trim_end_matches(['.', ' '])),
        _ => String::new(),
    };
    let id = t.id;
    let parent = t.parent.unwrap_or(0);
    div().border_t_1().border_color(k.line).px(px(26.)).py(px(14.)).child(
        div()
            .max_w(px(860.))
            .mx_auto()
            .flex()
            .items_center()
            .gap(px(10.))
            .px(px(14.))
            .py(px(10.))
            .rounded(px(12.))
            .border_1()
            .border_color(k.line2)
            .bg(k.panel)
            .text_size(px(13.))
            .text_color(k.text2)
            .child(status_dot("end", t.status.as_str(), 9., cx))
            .child(div().flex_1().child(format!("This task is {}{tail}. Its memory stays readable.", t.status.as_str())))
            .child(btn("continue-new", "Continue as a new task", Tone::Primary, cx).child(probe("endbar.continue")).on_click(cx.listener(move |w, _, window, cx| dialogs::open_new_task(w, Some(("continue".into(), Some(id), parent)), window, cx)))),
    )
}
