//! The TV layout's remote control (samsung tizen tv; `OPTCHAT_TV=1` on a Mac for fast loops): the same desktop
//! workspace, driven by a remote's keys instead of a pointer and a keyboard, like the web tv shell (0.2.3):
//! - up / down: the previous / next task in the sidebar's tree (opens it); finished tasks are hidden
//! - left / right: scroll the chat up / down
//! - OK (enter, `Confirm`) and green: send the draft (nothing when it's empty)
//! - red: delete the draft's last word
//! - mic (XF86BTVoice, held): dictate into the draft while held, through the engine's dictation
//! - back: dismiss
//! The text field never takes focus (so no on-screen keyboard); text comes from dictation only. No hint labels.
use crate::workspace::Workspace;
use gpui::*;
use std::sync::atomic::{AtomicBool, Ordering};

actions!(tv, [Up, Down, Left, Right, Confirm, Red, Green, Mic, Back]);

static TV: AtomicBool = AtomicBool::new(false);
/// whether the TV layout is on (the shell turns it on before opening the window)
pub fn enabled() -> bool {
    TV.load(Ordering::Relaxed)
}
pub fn enable() {
    TV.store(true, Ordering::Relaxed);
    // engine traffic (tickers, streaming text of running tasks) redraws at most ~3×/s on the tv
    optchat_client::state::set_engine_redraw_gap(ENGINE_REDRAW_GAP_MS);
}

/// see `enable`; dev builds can override it (`OC_ENGINE_REDRAW_GAP_MS`) for A/B
pub const ENGINE_REDRAW_GAP_MS: u64 = 300;

/// the remote's keys, in the workspace (they win over the desktop bindings: bound after them)
pub fn bind(cx: &mut App) {
    let ctx = Some("Workspace");
    cx.bind_keys([
        KeyBinding::new("up", Up, ctx),
        KeyBinding::new("down", Down, ctx),
        KeyBinding::new("left", Left, ctx),
        KeyBinding::new("right", Right, ctx),
        KeyBinding::new("enter", Confirm, ctx),
        KeyBinding::new("xf86red", Red, ctx),
        KeyBinding::new("xf86green", Green, ctx),
        KeyBinding::new("xf86btvoice", Mic, ctx),
        KeyBinding::new("xf86back", Back, ctx),
        // a Mac standing in for the remote: F1 red, F2 green, F3 mic (held), escape back
        KeyBinding::new("f1", Red, ctx),
        KeyBinding::new("f2", Green, ctx),
        KeyBinding::new("f3", Mic, ctx),
        KeyBinding::new("escape", Back, ctx),
    ]);
}

/// the draft without its last word (trailing spaces go with it)
pub fn drop_last_word(s: &str) -> String {
    let t = s.trim_end();
    match t.rfind(char::is_whitespace) {
        Some(i) => t[..i].trim_end().to_string(),
        None => String::new(),
    }
}

/// whether a key-up ends a held mic press
pub fn is_mic_key(key: &str) -> bool {
    matches!(key, "xf86btvoice" | "f3")
}

impl Workspace {
    fn tv_step(&mut self, d: i64, window: &mut Window, cx: &mut Context<Self>) {
        let s = self.state.read(cx);
        let rows = crate::tasks_view::visible_rows(s, &self.tasks_panel.read(cx).collapsed);
        let ids: Vec<i64> = rows.iter().map(|r| r.0.task.id).collect();
        let Some(i) = ids.iter().position(|id| *id == s.current) else { return };
        let j = (i as i64 + d).clamp(0, ids.len() as i64 - 1) as usize;
        if j != i {
            self.open_task(ids[j], window, cx);
        }
    }
    fn tv_send(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let dictating = self.state.read(cx).dict.state != "idle";
        let empty = self.composer.read(cx).input.read(cx).value().trim().is_empty();
        if dictating || !empty {
            self.composer.update(cx, |c, cx| c.submit(true, window, cx));
        }
    }
    fn tv_scroll(&mut self, d: f32, cx: &mut Context<Self>) {
        self.feed.update(cx, |f, cx| {
            // at the bottom the list's logical top is past the last item: a step up starts a screen higher
            let at_end = f.list.logical_scroll_top().item_ix >= f.list.item_count();
            let vh = f32::from(f.list.viewport_bounds().size.height);
            f.list.scroll_by(px(if d < 0. && at_end { d - vh } else { d }));
            if d < 0. {
                f.scrolled_up = true;
            }
            let top = f.list.logical_scroll_top();
            crate::ui::set_state_extra("tvScroll", serde_json::json!({ "item": top.item_ix, "offset": f32::from(top.offset_in_item), "rows": f.list.item_count() }));
            cx.notify()
        });
        if d < 0. {
            self.state.update(cx, |s, _| s.follow = false);
        }
    }
    /// the remote's actions, on the workspace's root element
    pub fn tv_actions<E: InteractiveElement>(&mut self, d: E, cx: &mut Context<Self>) -> E {
        use crate::ui::anim::input;
        d.on_action(cx.listener(|w, _: &Up, window, cx| { input(); w.tv_step(-1, window, cx) }))
            .on_action(cx.listener(|w, _: &Down, window, cx| { input(); w.tv_step(1, window, cx) }))
            .on_action(cx.listener(|w, _: &Left, _, cx| { input(); w.tv_scroll(-360., cx) }))
            .on_action(cx.listener(|w, _: &Right, _, cx| { input(); w.tv_scroll(360., cx) }))
            .on_action(cx.listener(|w, _: &Confirm, window, cx| w.tv_send(window, cx)))
            .on_action(cx.listener(|w, _: &Green, window, cx| w.tv_send(window, cx)))
            .on_action(cx.listener(|w, _: &Red, window, cx| {
                w.composer.update(cx, |c, cx| {
                    let v = drop_last_word(&c.input.read(cx).value());
                    c.input.update(cx, |i, cx| i.set_value(v, window, cx));
                    cx.notify();
                });
            }))
            .on_action(cx.listener(|w, _: &Mic, window, cx| {
                if w.m_press.is_some() {
                    return; // the remote repeats a held key
                }
                w.m_press = Some(std::time::Instant::now());
                w.composer.update(cx, |c, cx| c.mic_down(window, cx));
            }))
            .on_action(cx.listener(|w, _: &Back, window, cx| {
                if w.overlay.is_some() {
                    w.close_overlay(window, cx);
                }
                window.focus(&w.focus, cx);
                cx.notify();
            }))
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn last_word() {
        assert_eq!(super::drop_last_word("alpha beta gamma"), "alpha beta");
        assert_eq!(super::drop_last_word("alpha beta  "), "alpha");
        assert_eq!(super::drop_last_word("alpha"), "");
        assert_eq!(super::drop_last_word(""), "");
    }
}

// ---- temporary theme preview (to pick the tv's theme): every bundled theme in turn, 15 s each, its name on top.
// the shell turns it on (tizen `theme-preview` feature); off = a normal build.
static PREVIEW: AtomicBool = AtomicBool::new(false);
pub const PREVIEW_SECS: u64 = 15;
pub fn preview_on() -> bool {
    PREVIEW.load(Ordering::Relaxed)
}
/// start cycling through `optchat_theme::builtin()` (the same list the settings picker shows), from the active one
pub fn start_theme_preview(cx: &mut App) {
    PREVIEW.store(true, Ordering::Relaxed);
    cx.spawn(async move |cx: &mut AsyncApp| loop {
        cx.background_executor().timer(std::time::Duration::from_secs(PREVIEW_SECS)).await;
        let _ = cx.update(|cx| next_theme(cx));
    })
    .detach();
}
pub fn next_theme(cx: &mut App) {
    let th = cx.global::<crate::theme::Themes>();
    let i = th.all.iter().position(|t| t.id == th.active).map_or(0, |i| (i + 1) % th.all.len());
    let id = th.all[i].id.clone();
    crate::theme::set(cx, &id, false);
}
/// the active theme's name, centered at the top, above everything
pub fn preview_label(cx: &App) -> Option<AnyElement> {
    if !preview_on() {
        return None;
    }
    let th = cx.global::<crate::theme::Themes>();
    let i = th.all.iter().position(|t| t.id == th.active).unwrap_or(0);
    let name = th.all.get(i).map_or(th.active.clone(), |t| t.name.clone());
    let c = &th.c;
    Some(
        div()
            .absolute()
            .top(px(14.))
            .left_0()
            .right_0()
            .flex()
            .justify_center()
            .child(
                div()
                    .px(px(22.))
                    .py(px(8.))
                    .rounded(px(10.))
                    .bg(c.panel2)
                    .border_2()
                    .border_color(c.accent)
                    .text_color(c.text)
                    .text_size(px(26.))
                    .child(format!("{name}   {}/{}", i + 1, th.all.len())),
            )
            .into_any_element(),
    )
}
