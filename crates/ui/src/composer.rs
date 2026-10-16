//! The composer (app/ui/app.tsx Composer, spec-tasks A): ↑ appends (into context, no turn), → runs (starts a pending
//! task, runs what was appended, continues a stopped turn, resumes a paused one, steers a running turn). Enter = →,
//! ⌘Enter = ↑ (→ while a turn runs), Shift+Enter = newline. Images by picker, paste or drop; dictation (mic button or
//! M outside text fields: tap toggles, hold dictates; Esc stops) writes at the cursor.
use crate::state::{AppState, StateEvent};
use crate::theme::c;
use crate::ui::*;
use gpui::prelude::FluentBuilder;
use gpui::*;
use crate::kit::input::{Input, InputEvent, InputState};
use crate::media::prepare_image;
use optchat_proto::api::ImageIn;
use optchat_proto::types::TaskStatus;
use std::collections::HashMap;
use std::sync::Arc;

actions!(composer, [Run, Append, Newline, PasteSmart, StopDictation]);

pub fn bind(cx: &mut App) {
    cx.bind_keys([
        KeyBinding::new("enter", Run, Some("Composer > Input")),
        KeyBinding::new("secondary-enter", Append, Some("Composer > Input")),
        KeyBinding::new("shift-enter", Newline, Some("Composer > Input")),
        KeyBinding::new("secondary-v", PasteSmart, Some("Composer > Input")),
        KeyBinding::new("escape", StopDictation, Some("Composer")),
    ]);
}

#[derive(Clone)]
pub struct Attachment {
    pub name: String,
    pub media_type: String,
    pub bytes: Arc<Vec<u8>>,
    pub w: u32,
    pub h: u32,
    pub img: Arc<Image>,
}

pub struct Composer {
    /// the phone layout: touch-sized buttons (44 pt)
    pub touch: bool,
    pub state: Entity<AppState>,
    pub input: Entity<InputState>,
    /// drafts by (workspace, task)
    drafts: HashMap<(String, i64), String>,
    /// the workspace the composer writes in
    ws: String,
    pub task: i64,
    pub images: Vec<Attachment>,
    busy: bool,
    drag: bool,
    anchor: Option<(i64, String, String)>, // dictation: task, before, after
    caret: Option<usize>,
    dict_seq: u64,
    pub mic_press: Option<(std::time::Instant, bool)>,
    err: Option<(String, std::time::Instant)>,
    pending_submit: Option<bool>,
    _subs: Vec<Subscription>,
}

impl Composer {
    pub fn new(state: Entity<AppState>, window: &mut Window, cx: &mut Context<Self>) -> Composer {
        let first = state.read(cx).current;
        let who = if first == 0 { "OptChat".to_string() } else { format!("#{first}") };
        let input = cx.new(|cx| InputState::new(window, cx).multi_line(true).auto_grow(1, 10).placeholder(format!("Message {who}…")));
        let s1 = cx.subscribe_in(&input, window, |this: &mut Composer, _, ev: &InputEvent, _, cx| {
            if let InputEvent::Change = ev {
                let v = this.input.read(cx).value().to_string();
                this.drafts.insert((this.ws.clone(), this.task), v);
                cx.notify();
            }
        });
        let s2 = cx.subscribe_in(&state, window, |this: &mut Composer, _, ev: &StateEvent, window, cx| match ev {
            StateEvent::Opened(id) => this.switch(*id, window, cx),
            StateEvent::DictText => this.on_dict_text(window, cx),
            _ => {}
        });
        let s3 = cx.observe(&state, |_, _, cx| cx.notify());
        let task = state.read(cx).current;
        let ws = state.read(cx).active.clone();
        Composer { touch: false, state, input, drafts: HashMap::new(), ws, task, images: vec![], busy: false, drag: false, anchor: None, caret: None, dict_seq: 0, mic_press: None, err: None, pending_submit: None, _subs: vec![s1, s2, s3] }
    }

    fn switch(&mut self, id: i64, window: &mut Window, cx: &mut Context<Self>) {
        if id == self.task {
            return;
        }
        let cur = self.input.read(cx).value().to_string();
        self.drafts.insert((self.ws.clone(), self.task), cur);
        self.task = id;
        let d = self.drafts.get(&(self.ws.clone(), id)).cloned().unwrap_or_default();
        let who = if id == 0 { "OptChat".to_string() } else { format!("#{id}") };
        self.input.update(cx, |i, cx| {
            i.set_value(d, window, cx);
            i.set_placeholder(format!("Message {who}…"), window, cx);
        });
        self.images.clear();
        self.focus(window, cx);
    }

    /// another workspace on screen: keep this draft, show that workspace's draft of its task
    pub fn workspace_switched(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let cur = self.input.read(cx).value().to_string();
        self.drafts.insert((self.ws.clone(), self.task), cur);
        let (ws, id) = {
            let s = self.state.read(cx);
            (s.active.clone(), s.current)
        };
        self.ws = ws;
        self.task = id;
        self.anchor = None;
        self.pending_submit = None;
        self.images.clear();
        let d = self.drafts.get(&(self.ws.clone(), id)).cloned().unwrap_or_default();
        let who = if id == 0 { "OptChat".to_string() } else { format!("#{id}") };
        self.input.update(cx, |i, cx| {
            i.set_value(d, window, cx);
            i.set_placeholder(format!("Message {who}…"), window, cx);
        });
        cx.notify();
    }

    pub fn focus(&self, window: &mut Window, cx: &mut Context<Self>) {
        self.input.update(cx, |i, cx| i.focus(window, cx));
    }

    pub fn running(&self, cx: &App) -> bool {
        let s = self.state.read(cx);
        s.cur.running.is_some() || s.info().map(|i| i.waiting).unwrap_or(false)
    }
    fn runnable(&self, cx: &App) -> bool {
        let s = self.state.read(cx);
        let Some(info) = s.info() else { return false };
        let st = info.task.status;
        let last = s.last_turn();
        let can_continue = s.cur.running.is_none() && last.map(|t| t.status == "cancelled" || t.status == "failed").unwrap_or(false) && !st.terminal() && st != TaskStatus::Paused;
        can_continue || info.task.unrun.is_some() || st == TaskStatus::Pending || st == TaskStatus::Paused
    }
    fn dictating(&self, cx: &App) -> bool {
        let d = &self.state.read(cx).dict;
        d.state != "idle"
    }
    fn disabled(&self, cx: &App) -> bool {
        let s = self.state.read(cx);
        s.hub.is_none() || !s.cur.loaded
    }

    pub fn submit(&mut self, run: bool, window: &mut Window, cx: &mut Context<Self>) {
        if self.dictating(cx) {
            // while dictating: stop, wait for the final text, then send
            self.pending_submit = Some(run);
            self.stop_dictation(cx);
            return;
        }
        if self.disabled(cx) || self.busy {
            return;
        }
        let text = self.input.read(cx).value().trim().to_string();
        let has = !text.is_empty() || !self.images.is_empty();
        if !has && !(run && self.runnable(cx)) {
            return;
        }
        let task = self.task;
        let imgs: Vec<ImageIn> = self.images.drain(..).map(|a| ImageIn { media_type: a.media_type, bytes: (*a.bytes).clone(), name: a.name, w: a.w, h: a.h }).collect();
        self.input.update(cx, |i, cx| i.set_value("", window, cx));
        self.drafts.remove(&(self.ws.clone(), task));
        self.state.update(cx, |s, cx| {
            if run {
                s.follow = true;
            }
            s.command(cx, false, move |h| async move { h.send_message(task, &text, imgs, run) })
        });
        cx.notify();
    }

    fn on_run(&mut self, _: &Run, window: &mut Window, cx: &mut Context<Self>) {
        self.submit(true, window, cx);
    }
    fn on_append(&mut self, _: &Append, window: &mut Window, cx: &mut Context<Self>) {
        // ⌘Enter appends; while a turn runs (↑ hidden) it runs too
        let run = self.running(cx);
        self.submit(run, window, cx);
    }
    fn on_newline(&mut self, _: &Newline, window: &mut Window, cx: &mut Context<Self>) {
        self.input.update(cx, |i, cx| i.insert("\n", window, cx));
    }
    fn on_paste(&mut self, _: &PasteSmart, window: &mut Window, cx: &mut Context<Self>) {
        let Some(item) = cx.read_from_clipboard() else { return };
        self.paste_item(item, window, cx);
    }
    /// ⌘V's handling of what the clipboard holds: images are attached, text is inserted at the cursor
    pub fn paste_item(&mut self, item: ClipboardItem, window: &mut Window, cx: &mut Context<Self>) {
        let mut got_image = false;
        for e in item.entries() {
            if let ClipboardEntry::Image(img) = e {
                let mt = match img.format {
                    ImageFormat::Png => "image/png",
                    ImageFormat::Jpeg => "image/jpeg",
                    ImageFormat::Gif => "image/gif",
                    ImageFormat::Webp => "image/webp",
                    _ => "image/png",
                };
                self.add_bytes("pasted image".into(), mt.into(), img.bytes.clone(), cx);
                got_image = true;
            }
        }
        if !got_image {
            if let Some(t) = item.text() {
                self.input.update(cx, |i, cx| i.insert(&t, window, cx));
            }
        }
    }

    pub fn add_bytes(&mut self, name: String, mt: String, bytes: Vec<u8>, cx: &mut Context<Self>) {
        if self.images.len() >= 20 {
            return;
        }
        match prepare_image(&bytes, &mt) {
            Ok((mt, b, w, h)) => {
                let fmt = match mt.as_str() {
                    "image/png" => ImageFormat::Png,
                    "image/gif" => ImageFormat::Gif,
                    "image/webp" => ImageFormat::Webp,
                    _ => ImageFormat::Jpeg,
                };
                let img = Arc::new(Image::from_bytes(fmt, b.clone()));
                self.images.push(Attachment { name, media_type: mt, bytes: Arc::new(b), w, h, img });
            }
            Err(e) => self.err = Some((format!("image failed: {e}"), std::time::Instant::now())),
        }
        cx.notify();
    }
    pub fn add_paths(&mut self, paths: Vec<std::path::PathBuf>, cx: &mut Context<Self>) {
        for p in paths {
            let ext = p.extension().and_then(|e| e.to_str()).unwrap_or("").to_lowercase();
            let mt = match ext.as_str() {
                "png" => "image/png",
                "jpg" | "jpeg" => "image/jpeg",
                "gif" => "image/gif",
                "webp" => "image/webp",
                "heic" | "tiff" | "tif" | "bmp" => "image/other",
                _ => continue,
            };
            if let Ok(b) = std::fs::read(&p) {
                let name = p.file_name().and_then(|n| n.to_str()).unwrap_or("image").to_string();
                self.add_bytes(name, mt.into(), b, cx);
            }
        }
    }
    fn pick(&mut self, cx: &mut Context<Self>) {
        let rx = cx.prompt_for_paths(PathPromptOptions { files: true, directories: false, multiple: true, prompt: Some("Attach".into()) });
        cx.spawn(async move |this, cx| {
            if let Ok(Ok(Some(paths))) = rx.await {
                let _ = this.update(cx, |c, cx| c.add_paths(paths, cx));
            }
        })
        .detach();
    }

    // ---- dictation ------------------------------------------------------------------------------------------------
    pub fn start_dictation(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.disabled(cx) {
            return;
        }
        let v = self.input.read(cx).value().to_string();
        let focused = self.input.read(cx).focus_handle(cx).is_focused(window);
        // from outside the text field: where the last dictation left the caret, else the end
        let pos = if focused { self.input.read(cx).cursor() } else { self.caret.unwrap_or(v.len()) }.min(v.len());
        let pos = (0..=pos).rev().find(|p| v.is_char_boundary(*p)).unwrap_or(0);
        self.anchor = Some((self.task, v[..pos].to_string(), v[pos..].to_string()));
        self.dict_seq = self.state.read(cx).dict.seq;
        let task = self.task;
        self.state.update(cx, |s, cx| {
            s.dict.state = "connecting".into();
            s.dict.task = task;
            if let Some(h) = s.hub() {
                s.rt.spawn(async move { h.dictate_start(task) });
            }
            cx.notify();
        });
    }
    pub fn stop_dictation(&mut self, cx: &mut Context<Self>) {
        self.state.update(cx, |s, cx| {
            if s.dict.state != "idle" {
                s.dict.state = "finishing".into();
            }
            if let Some(h) = s.hub() {
                s.rt.spawn(async move { h.dictate_stop() });
            }
            cx.notify();
        });
    }
    pub fn mic_down(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.mic_press.is_some() {
            return;
        }
        if self.dictating(cx) {
            self.mic_press = Some((std::time::Instant::now(), false));
            self.stop_dictation(cx);
            return;
        }
        self.mic_press = Some((std::time::Instant::now(), true));
        self.start_dictation(window, cx);
    }
    pub fn mic_up(&mut self, cx: &mut Context<Self>) {
        let p = self.mic_press.take();
        if let Some((at, started)) = p {
            let ms = at.elapsed().as_millis();
            crate::ui::set_state_extra("micRelease", serde_json::json!({ "ms": ms as u64, "started": started, "stops": started && ms > 300 }));
            if started && ms > 300 {
                self.stop_dictation(cx);
            }
        }
    }
    /// Test hook: press the mic button and release it `hold_ms` later (the same mic_down / mic_up as the mouse)
    pub fn test_press(&mut self, hold_ms: u64, window: &mut Window, cx: &mut Context<Self>) {
        self.mic_down(window, cx);
        cx.spawn(async move |this, cx| {
            Timer::after(std::time::Duration::from_millis(hold_ms)).await;
            let _ = this.update(cx, |c, cx| c.mic_up(cx));
        })
        .detach();
    }
    fn on_dict_text(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let d = self.state.read(cx).dict.clone();
        if d.seq == self.dict_seq {
            return;
        }
        self.dict_seq = d.seq;
        let Some((task, before, after)) = self.anchor.clone() else { return };
        if task != self.task {
            return;
        }
        let t = d.text.clone().unwrap_or_default();
        let lead = if !before.is_empty() && !t.is_empty() && !before.ends_with(char::is_whitespace) { " " } else { "" };
        let trail = if !after.is_empty() && !t.is_empty() && !after.starts_with(char::is_whitespace) { " " } else { "" };
        let v = format!("{before}{lead}{t}{trail}{after}");
        let caret = before.len() + lead.len() + t.len();
        self.caret = Some(caret);
        self.input.update(cx, |i, cx| {
            i.set_value(v.clone(), window, cx);
        });
        self.drafts.insert((self.ws.clone(), task), v);
        if d.state == "idle" || d.done.is_some() && d.state == "idle" {
            // finished: caret after the dictated text, focus back
            self.anchor = None;
            self.input.update(cx, |i, cx| i.set_cursor(caret, window, cx));
            self.focus(window, cx);
            if let Some(run) = self.pending_submit.take() {
                self.submit(run, window, cx);
            }
        }
        cx.notify();
    }
}

impl Composer {
    pub fn render_mic(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let k = c(cx).clone();
        let d = self.state.read(cx).dict.clone();
        let st = if d.task == self.task { d.state.clone() } else { "idle".to_string() };
        crate::ui::seen("micSeen", &st);
        let off = self.disabled(cx) && st == "idle";
        let b = div()
            .id("mic")
            .size(px(if self.touch { 44. } else { 30. }))
            .flex()
            .flex_none()
            .items_center()
            .justify_center()
            .rounded_full()
            .border_1()
            .border_color(transparent_black())
            .cursor_pointer()
            .relative()
            .tooltip(tooltip("dictate (M)"))
            .child(probe("composer.mic"));
        let b: AnyElement = match st.as_str() {
            "listening" | "finishing" => {
                let mut bars = div().flex().items_center().gap(px(2.)).h(px(16.));
                for (i, f) in [0.55f32, 1.0, 0.75, 0.45].iter().enumerate() {
                    let h = if st == "finishing" { 0.2 } else { (d.level * f).clamp(0.15, 1.0) };
                    bars = bars.child(div().id(("bar", i)).w(px(2.5)).h(px(16. * h)).rounded(px(2.)).bg(k.red));
                }
                let el = b.bg(k.red_bg).border_color(k.red.opacity(0.35)).child(bars);
                if st == "listening" {
                    el.with_animation("micpulse", Animation::new(std::time::Duration::from_millis(1600)).repeat(), |e, t| {
                        let a = ((t * std::f32::consts::TAU).sin() * 0.5 + 0.5) * 0.12;
                        e.shadow(vec![BoxShadow { color: hsla(0., 1., 0.7, a), offset: point(px(0.), px(0.)), blur_radius: px(0.), spread_radius: px(4.), inset: false }])
                    }).into_any_element()
                } else {
                    el.into_any_element()
                }
            }
            "connecting" => b.child(icon("mic", 16., k.red)).child(div().absolute().inset_0().flex().items_center().justify_center().child(spinner("micspin", 26., k.red))).into_any_element(),
            _ => b.hover(|s| s.bg(k.hover)).child(icon("mic", 16., k.muted)).into_any_element(),
        };
        div()
            .id("micwrap")
            .when(off, |d| d.opacity(0.4))
            .child(b)
            .on_mouse_down(MouseButton::Left, cx.listener(|c, _, w, cx| c.mic_down(w, cx)))
            .on_mouse_up(MouseButton::Left, cx.listener(|c, _, _, cx| c.mic_up(cx)))
            .on_mouse_up_out(MouseButton::Left, cx.listener(|c, _, _, cx| c.mic_up(cx)))
    }
}

impl Render for Composer {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        crate::ui::anim::area(crate::ui::anim::WORKSPACE);
        let k = c(cx).clone();
        let s = self.state.read(cx);
        if crate::ui::probes_on() {
            let fresh = |e: &Option<(String, std::time::Instant)>| e.as_ref().filter(|(_, t)| t.elapsed().as_secs_f32() < 6.0).map(|(e, _)| e.clone());
            let err = fresh(&self.err).or_else(|| fresh(&s.dict.error)).or_else(|| s.errors.last().cloned());
            // UI tests: how long a dictation error stayed in the error line (measured when it went)
            if let Some((m, t)) = &s.dict.error {
                thread_local!(static DONE: std::cell::Cell<Option<std::time::Instant>> = const { std::cell::Cell::new(None) });
                if err.as_deref() != Some(m.as_str()) && DONE.with(|d| d.get()) != Some(*t) {
                    DONE.with(|d| d.set(Some(*t)));
                    crate::ui::set_state_extra("dictErrorGone", serde_json::json!({ "message": m, "lifeMs": t.elapsed().as_millis() as u64 }));
                }
            }
            let disabled = self.disabled(cx);
            if disabled && !s.cur.loaded {
                crate::ui::bump("composerDisabledWhileLoading");
            }
            crate::ui::set_state_extra("composer", serde_json::json!({ "images": self.images.len(), "dims": self.images.iter().map(|a| [a.w, a.h]).collect::<Vec<_>>(), "types": self.images.iter().map(|a| a.media_type.clone()).collect::<Vec<_>>(), "task": self.task, "disabled": disabled, "error": err, "dictating": self.dictating(cx) && s.dict.task == self.task }));
        }
        let running = self.running(cx);
        let stoppable = s.cur.running.is_some();
        let runnable = self.runnable(cx);
        let has = !self.input.read(cx).value().trim().is_empty() || !self.images.is_empty();
        let disabled = self.disabled(cx);
        let dictating = self.dictating(cx) && s.dict.task == self.task;
        let fresh = |e: &Option<(String, std::time::Instant)>| e.as_ref().filter(|(_, t)| t.elapsed().as_secs_f32() < 6.0).map(|(e, _)| e.clone());
        let err = fresh(&self.err).or_else(|| fresh(&s.dict.error)).or_else(|| s.errors.last().cloned());
        let task = self.task;
        let mut thumbs = div().flex().flex_wrap().gap(px(8.)).mb(px(8.));
        for (n, a) in self.images.iter().enumerate() {
            thumbs = thumbs.child(
                div()
                    .id(("thumb", n))
                    .relative()
                    .size(px(64.))
                    .rounded(px(8.))
                    .overflow_hidden()
                    .border_1()
                    .border_color(k.line2)
                    .tooltip(tooltip(format!("{} · {}×{}", a.name, a.w, a.h)))
                    .child(img(a.img.clone()).size_full().object_fit(ObjectFit::Cover))
                    .child(
                        div()
                            .id(("thumbx", n))
                            .absolute()
                            .top(px(2.))
                            .right(px(2.))
                            .size(px(18.))
                            .rounded_full()
                            .bg(hsla(0., 0., 0., 0.65))
                            .flex()
                            .items_center()
                            .justify_center()
                            .cursor_pointer()
                            .child(icon("x", 11., white()))
                            .child(probe(format!("thumbx.{n}")))
                            .on_click(cx.listener(move |c, _, _, cx| {
                                if n < c.images.len() {
                                    c.images.remove(n);
                                }
                                cx.notify();
                            })),
                    ),
            );
        }
        let run_off = (!has && !(runnable && !running)) || disabled || self.busy;
        let border = if dictating { k.red.opacity(0.45) } else if self.drag { k.accent } else { k.line2 };
        div()
            .w_full()
            .border_t_1()
            .border_color(k.line)
            .bg(k.bg)
            .px(px(26.))
            .pt(px(12.))
            .pb(px(16.))
            .child(
                div()
                    .w_full()
                    .flex()
                    .flex_col()
                    .items_center()
                    .when_some(err, |d, e| d.child(div().w_full().max_w(px(860.)).mb(px(8.)).text_size(px(13.)).text_color(k.red).child(e)))
                    .child(
                        div()
                            .id("composer")
                            .key_context("Composer")
                            .on_action(cx.listener(Self::on_run))
                            .on_action(cx.listener(Self::on_append))
                            .on_action(cx.listener(Self::on_newline))
                            .on_action(cx.listener(Self::on_paste))
                            .on_action(cx.listener(|c, _: &StopDictation, _, cx| {
                                if c.dictating(cx) {
                                    c.stop_dictation(cx);
                                } else {
                                    // not dictating: let the workspace close a sheet / overlay
                                    cx.propagate();
                                }
                            }))
                            .on_drop(cx.listener(|c, paths: &ExternalPaths, _, cx| {
                                c.drag = false;
                                c.add_paths(paths.paths().to_vec(), cx);
                            }))
                            .on_drag_move(cx.listener(|c, _: &DragMoveEvent<ExternalPaths>, _, cx| {
                                if !c.drag {
                                    c.drag = true;
                                    cx.notify();
                                }
                            }))
                            .relative()
                            .w_full()
                            .max_w(px(860.))
                            .rounded(px(14.))
                            .border_1()
                            .border_color(border)
                            .bg(k.panel)
                            .pl(px(10.))
                            .pr(px(10.))
                            .pt(px(8.))
                            .pb(px(8.))
                            .child(probe("composer"))
                            .when(!self.images.is_empty(), |d| d.child(thumbs))
                            .child(div().child(Input::new(&self.input).appearance(false).disabled(dictating).text_size(px(14.5))).child(probe("composer.input")))
                            .child(
                                div()
                                    .flex()
                                    .items_center()
                                    .gap(px(8.))
                                    .mt(px(6.))
                                    .child(icon_btn("attach", "image", 16., cx).tooltip(tooltip("attach images (or paste / drop them)")).on_click(cx.listener(|c, _, _, cx| c.pick(cx))))
                                    .child(div().flex_1())
                                    .when(stoppable, |d| {
                                        let st = self.state.clone();
                                        d.child(btn("stop", "Stop", Tone::Danger, cx).child(probe("composer.stop")).on_click(move |_, _, cx| {
                                            st.update(cx, |s, cx| s.command(cx, false, move |h| async move { { h.stop(task); Ok(String::new()) } }))
                                        }))
                                    })
                                    .child(self.render_mic(cx))
                                    .when(!running, |d| {
                                        let off = !has || disabled || self.busy;
                                        d.child(
                                            div()
                                                .id("append")
                                                .size(px(if self.touch { 44. } else { 32. }))
                                                .rounded_full()
                                                .border_1()
                                                .border_color(k.line2)
                                                .flex()
                                                .items_center()
                                                .justify_center()
                                                .child(icon("arrow-up", 16., k.text2))
                                                .child(probe("composer.append"))
                                                .tooltip(tooltip("add to the context (⌘Enter)"))
                                                .when(off, |d| d.opacity(0.4))
                                                .when(!off, |d| d.cursor_pointer().hover(|s| s.border_color(k.muted)).on_click(cx.listener(|c, _, w, cx| c.submit(false, w, cx)))),
                                        )
                                    })
                                    .child(
                                        div()
                                            .id("run")
                                            .size(px(if self.touch { 44. } else { 32. }))
                                            .rounded_full()
                                            .bg(k.accent2)
                                            .flex()
                                            .items_center()
                                            .justify_center()
                                            .child(icon("arrow-right", 16., k.on_accent))
                                            .child(probe("composer.run"))
                                            .tooltip(tooltip("run (Enter)"))
                                            .when(run_off, |d| d.opacity(0.4))
                                            .when(!run_off, |d| d.cursor_pointer().hover(|s| s.bg(k.accent)).on_click(cx.listener(|c, _, w, cx| c.submit(true, w, cx)))),
                                    ),
                            ),
                    ),
            )
    }
}
