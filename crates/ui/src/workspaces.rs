//! Engines and workspaces in the UI (spec-workspaces §4, §6): the Workspaces panel, a rail view opened by the top-left
//! icon (and ⇧⌘O): engines → their connected workspaces (a check on the one on screen, a dot for unread
//! errors/warnings, an offline engine greyed and retried). Add engine asks only for a token (pasted, masked; on phones
//! and tablets also scanned from the admin page's QR) and an optional name: the token carries the engine's addresses,
//! and every workspace in its scope connects by itself. No workspace name is special.
use crate::client::ws_key;
use crate::state::*;
use crate::theme::c;
use crate::ui::*;
use crate::workspace::Workspace;
use gpui::prelude::FluentBuilder;
use gpui::*;
use crate::state::View;
use crate::kit::input::{Input, InputEvent, InputState};
use optchat_proto::api::Conn;

/// the panel on screen (from the status bar's workspace chip, the empty state, Settings › App): never toggles it off
pub fn open_panel(w: &mut Workspace, cx: &mut Context<Workspace>) {
    w.state.update(cx, |s, _| {
        s.prefs.view = View::Workspaces;
        s.save_prefs();
    });
    w.set_side_open(true, cx);
}
/// the panel with its Add engine form open and focused
pub fn open_add_engine(w: &mut Workspace, window: &mut Window, cx: &mut Context<Workspace>) {
    open_panel(w, cx);
    w.ws_panel.update(cx, |p, cx| p.start_add(window, cx));
}

pub struct WorkspacesPanel {
    pub state: Entity<AppState>,
    /// engines whose disabled workspaces are expanded
    disabled_open: std::collections::HashSet<String>,
    pub adding: bool,
    /// the engine token (oc1…), masked
    pub token: Entity<InputState>,
    /// a name for the engine (optional: else the host it connected through)
    pub name: Entity<InputState>,
    /// an Add in flight (the token's addresses being tried)
    pending: bool,
    _subs: Vec<Subscription>,
}

impl WorkspacesPanel {
    pub fn new(state: Entity<AppState>, window: &mut Window, cx: &mut Context<Self>) -> WorkspacesPanel {
        let token = cx.new(|cx| InputState::new(window, cx).masked(true).placeholder("engine token (oc1…)"));
        let name = cx.new(|cx| InputState::new(window, cx).placeholder("name (optional)"));
        let mut subs = vec![cx.observe(&state, |p: &mut WorkspacesPanel, s, cx| {
            // an Add finished: close the form when it worked
            if p.pending && !s.read(cx).adding_engine {
                p.pending = false;
                if s.read(cx).add_engine_error.is_none() {
                    p.adding = false;
                }
            }
            cx.notify()
        })];
        for i in [&token, &name] {
            subs.push(cx.subscribe(i, |p: &mut WorkspacesPanel, _, e: &InputEvent, cx| {
                if let InputEvent::PressEnter { .. } = e {
                    p.add(cx);
                }
            }));
        }
        WorkspacesPanel { state, disabled_open: Default::default(), adding: false, token, name, pending: false, _subs: subs }
    }
    pub fn start_add(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.adding = true;
        self.state.update(cx, |s, _| s.add_engine_error = None);
        self.token.update(cx, |i, cx| i.focus(window, cx));
        cx.notify();
    }
    pub fn set_add_fields(&mut self, token: &str, name: &str, window: &mut Window, cx: &mut Context<Self>) {
        let (t, n) = (token.to_string(), name.to_string());
        self.token.update(cx, |i, cx| i.set_value(t, window, cx));
        self.name.update(cx, |i, cx| i.set_value(n, window, cx));
    }
    pub fn add(&mut self, cx: &mut Context<Self>) {
        let token = self.token.read(cx).value().to_string();
        let name = self.name.read(cx).value().to_string();
        self.pending = true;
        self.state.update(cx, |s, cx| s.add_engine_token(&name, &token, cx));
        cx.notify();
    }
    pub fn cancel(&mut self, cx: &mut Context<Self>) {
        self.adding = false;
        self.pending = false;
        self.state.update(cx, |s, _| s.add_engine_error = None);
        cx.notify();
    }
    fn scan(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let me = cx.entity().downgrade();
        crate::platform::scan_qr(
            window,
            cx,
            Box::new(move |text, window, cx| {
                // the scanner may answer at once (from inside this panel's own update): set the field afterwards
                if let Some(t) = text {
                    window.defer(cx, move |window, cx| {
                        let _ = me.update(cx, |p, cx| {
                            p.token.update(cx, |i, cx| i.set_value(t.trim().to_string(), window, cx));
                            cx.notify();
                        });
                    });
                }
            }),
        );
    }

    fn row_base(id: SharedString, cx: &App) -> Stateful<Div> {
        let k = c(cx).clone();
        let hover = k.hover;
        div().id(id).flex().items_center().gap(px(8.)).mx(px(6.)).pl(px(10.)).pr(px(6.)).min_h(px(30.)).rounded(px(6.)).hover(move |d| d.bg(hover))
    }
}

impl Render for WorkspacesPanel {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let k = c(cx).clone();
        let s = self.state.read(cx);
        let engines = s.engines.clone();
        let meta: Vec<WsMeta> = s.meta.values().cloned().collect();
        let active = s.active.clone();
        let (busy, add_err) = (s.adding_engine, s.add_engine_error.clone());
        let mut col = div().flex().flex_col().pb(px(16.));
        let mut rows_state = vec![];
        if engines.is_empty() && !self.adding {
            col = col.child(div().px(px(14.)).py(px(8.)).text_size(px(12.)).text_color(k.muted).child("No engine yet. Add one with a token from its admin page: its workspaces connect by themselves."));
        }
        for (ei, e) in engines.iter().enumerate() {
            let online = e.online != Some(false);
            let fg = if online { k.text } else { k.faint };
            let ep = e.cfg.endpoint.clone();
            let st = self.state.clone();
            col = col.child(
                div()
                    .flex()
                    .items_center()
                    .gap(px(7.))
                    .px(px(14.))
                    .pt(px(if ei == 0 { 4. } else { 14. }))
                    .pb(px(4.))
                    .child(icon("server", 13., if online { k.muted } else { k.faint }))
                    .child(div().text_size(px(12.5)).font_weight(FontWeight::SEMIBOLD).text_color(fg).child(e.cfg.name.clone()))
                    .child(div().flex_1())
                    .child(div().text_size(px(11.)).text_color(if online { k.faint } else { k.amber }).child(match e.online {
                        Some(false) => "offline · retrying",
                        None => "…",
                        _ => "",
                    }))
                    .child(
                        icon_btn(SharedString::from(format!("wsp-rm-{ei}")), "x", 12., cx)
                            .tooltip(tooltip(format!("Remove {}", e.cfg.name)))
                            .children(probe(format!("wsp.remove.{ei}")))
                            .on_click(move |_, _, cx| {
                                let ep = ep.clone();
                                st.update(cx, |s, cx| s.remove_engine(&ep, cx))
                            }),
                    )
                    .children(probe(format!("wsp.engine.{ei}"))),
            );
            let connected: Vec<&WsMeta> = meta.iter().filter(|m| m.endpoint == e.cfg.endpoint).collect();
            let is_default = |n: &str| e.workspaces.iter().any(|w| w.name == n && w.default);
            for m in &connected {
                let key = ws_key(&m.endpoint, &m.ws);
                let on = key == active;
                // the one on screen has its notices in the status bar; the others get the dot
                let alert = !on && (m.unread.error > 0 || m.unread.warn > 0);
                let acol = if m.unread.error > 0 { k.red } else { k.amber };
                let state = if m.disabled.is_some() {
                    Some("disabled")
                } else if m.refused.is_some() {
                    Some("token refused")
                } else if m.no_token {
                    Some("no token")
                } else if !online || m.conn == Conn::Closed {
                    Some("offline")
                } else if m.conn == Conn::Connecting {
                    Some("connecting")
                } else {
                    None
                };
                // a connection of its own (an older per-workspace token) can be disconnected alone
                let own = crate::tokens::get(&key).is_some() || !e.cfg.auto;
                rows_state.push(serde_json::json!({ "ws": m.ws, "engine": ei, "kind": "connected", "on": on, "alert": alert, "state": state }));
                let (key2, key3, name) = (key.clone(), key.clone(), m.ws.clone());
                col = col.child(
                    Self::row_base(SharedString::from(format!("wsp-{key}")), cx)
                        .cursor_pointer()
                        .when(on, |d| d.bg(k.selected))
                        .child(div().w(px(14.)).child(if on { icon("check", 13., k.accent).into_any_element() } else { div().into_any_element() }))
                        .child(div().flex_1().min_w_0().flex().items_center().gap(px(6.)).text_size(px(13.)).text_color(if state.is_some() { k.muted } else { fg }).child(m.ws.clone()).when(is_default(&m.ws), |d| d.child(div().text_size(px(10.5)).text_color(k.faint).child("default"))))
                        .when_some(state, |d, st| d.child(div().text_size(px(11.)).text_color(k.faint).child(st)))
                        .when(alert, |d| d.child(dot(acol, 7.)))
                        .when(own, |d| {
                            d.child(
                                icon_btn(SharedString::from(format!("wsp-x-{key}")), "x", 12., cx)
                                    .tooltip(tooltip(format!("Disconnect {name}")))
                                    .children(probe(format!("wsp.disc.{}", m.ws)))
                                    .on_click(cx.listener(move |p, _, _, cx| {
                                        cx.stop_propagation();
                                        let key = key3.clone();
                                        p.state.update(cx, |s, cx| s.disconnect_ws(&key, cx));
                                    })),
                            )
                        })
                        .children(probe(format!("wsp.ws.{}", m.ws)))
                        .on_click(cx.listener(move |p, _, _, cx| {
                            let key = key2.clone();
                            p.state.update(cx, |s, cx| s.switch_ws(&key, cx));
                        })),
                );
            }
            if connected.is_empty() && online && e.online.is_some() {
                col = col.child(div().pl(px(38.)).py(px(4.)).text_size(px(12.)).text_color(k.faint).child(if e.cfg.auto { "no workspace in this token's scope" } else { "no workspace connected" }));
            }
            // a token's engine shows only the token's scope: disabled workspaces only where the scope is everything
            // (an engine by address, or the local one through its local token)
            let scoped = e.cfg.auto && crate::tokens::get(&crate::state::engine_token_key(&e.cfg.endpoint)).is_some();
            let disabled: Vec<String> = if scoped { vec![] } else { e.workspaces.iter().filter(|w| !w.running && !connected.iter().any(|m| m.ws == w.name)).map(|w| w.name.clone()).collect() };
            if !disabled.is_empty() {
                let open = self.disabled_open.contains(&e.cfg.endpoint);
                let ep = e.cfg.endpoint.clone();
                col = col.child(
                    Self::row_base(SharedString::from(format!("wsp-d-{ei}")), cx)
                        .cursor_pointer()
                        .min_h(px(26.))
                        .child(chevron(format!("wsp-ch-{ei}"), 12., k.faint, open))
                        .child(div().text_size(px(11.5)).text_color(k.faint).child(format!("{} disabled", disabled.len())))
                        .children(probe(format!("wsp.disabled.{ei}")))
                        .on_click(cx.listener(move |p, _, _, cx| {
                            if !p.disabled_open.remove(&ep) {
                                p.disabled_open.insert(ep.clone());
                            }
                            cx.notify();
                        })),
                );
                if open {
                    for n in &disabled {
                        rows_state.push(serde_json::json!({ "ws": n, "engine": ei, "kind": "disabled" }));
                        col = col.child(div().pl(px(44.)).h(px(24.)).flex().items_center().text_size(px(12.)).text_color(k.faint).child(n.clone()));
                    }
                }
            }
        }
        // Add engine: a token (pasted or scanned) and an optional name
        col = col.child(div().h(px(1.)).bg(k.line).mx(px(14.)).my(px(10.)));
        let can_scan = crate::platform::can_scan_qr(cx);
        if self.adding {
            col = col.child(
                div()
                    .px(px(14.))
                    .flex()
                    .flex_col()
                    .gap(px(6.))
                    .child(div().text_size(px(12.)).font_weight(FontWeight::SEMIBOLD).child("Add engine"))
                    .child(div().text_size(px(11.)).text_color(k.faint).child("Paste a token from the engine's admin page (Copy token)."))
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .gap(px(6.))
                            .child(div().flex_1().min_w_0().child(Input::new(&self.token).small()).children(probe("wsp.token")))
                            .when(can_scan, |d| {
                                d.child(
                                    div()
                                        .id("wsp-scan")
                                        .flex_none()
                                        .size(px(44.))
                                        .rounded(px(8.))
                                        .bg(k.panel2)
                                        .flex()
                                        .items_center()
                                        .justify_center()
                                        .cursor_pointer()
                                        .child(icon("scan-text", 18., k.text2))
                                        .children(probe("wsp.scan"))
                                        .on_click(cx.listener(|p, _, window, cx| p.scan(window, cx))),
                                )
                            }),
                    )
                    .child(div().child(Input::new(&self.name).small()).children(probe("wsp.name")))
                    .when_some(add_err.clone(), |d, e| d.child(div().text_size(px(11.5)).text_color(k.red).child(e)))
                    .child(
                        div()
                            .flex()
                            .gap(px(6.))
                            .child(btn_small("wsp-add-go", if busy { "Adding…" } else { "Add" }, Tone::Primary, cx).children(probe("wsp.add.go")).when(!busy, |d| d.on_click(cx.listener(|p, _, _, cx| p.add(cx)))))
                            .child(btn_small("wsp-add-cancel", "Cancel", Tone::Ghost, cx).on_click(cx.listener(|p, _, _, cx| p.cancel(cx)))),
                    ),
            );
        } else {
            col = col.child(
                Self::row_base("wsp-add".into(), cx)
                    .cursor_pointer()
                    .child(icon("plus", 13., k.muted))
                    .child(div().text_size(px(13.)).child("Add engine…"))
                    .children(probe("wsp.add"))
                    .on_click(cx.listener(|p, _, window, cx| p.start_add(window, cx))),
            );
        }
        set_state_extra(
            "wsPanel",
            serde_json::json!({ "rows": rows_state, "hasToken": !self.token.read(cx).value().is_empty(), "adding": self.adding, "busy": busy, "err": add_err, "canScan": can_scan,
                "engines": engines.iter().map(|e| serde_json::json!({ "name": e.cfg.name, "endpoint": e.cfg.endpoint, "online": e.online, "auto": e.cfg.auto, "workspaces": e.workspaces.len() })).collect::<Vec<_>>() }),
        );
        crate::ui::scroll_body("WorkspacesPanel", col.children(probe("wspanel")).into_any_element())
    }
}
