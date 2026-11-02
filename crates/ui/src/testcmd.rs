//! A test channel (OPTCHAT_TEST_HOOKS with OPTCHAT_PROBES): commands in `<probes>.cmd` (a JSON array, taken and
//! deleted every 250 ms) run through the app's own paths: the workspace menu's and dialogs' handlers, the state's
//! methods, keystrokes through the window's dispatch. For runs where no input can reach the window (a locked screen:
//! macOS draws nothing and Computer Use finds no window); the visual checks stay with tests/workspaces.ts.
use crate::state::*;
use crate::workspace::Workspace;
use gpui::*;
use crate::state::View;
use serde_json::Value;

/// OPTCHAT_HEADLESS: the test lays frames out itself (a locked screen draws nothing)
pub fn headless() -> bool {
    std::env::var("OPTCHAT_HEADLESS").is_ok()
}

/// the pending commands (the file is deleted once read)
pub fn take() -> Vec<Value> {
    if std::env::var("OPTCHAT_TEST_HOOKS").is_err() {
        return vec![];
    }
    let Ok(p) = std::env::var("OPTCHAT_PROBES") else { return vec![] };
    let path = format!("{p}.cmd");
    let Ok(text) = std::fs::read_to_string(&path) else { return vec![] };
    let _ = std::fs::remove_file(&path);
    serde_json::from_str(&text).unwrap_or_default()
}

pub fn is_input(c: &Value) -> bool {
    matches!(c["op"].as_str(), Some("key" | "type" | "keydown" | "keyup"))
}
pub fn is_mouse(c: &Value) -> bool {
    matches!(c["op"].as_str(), Some("click" | "scroll" | "drag" | "move" | "down" | "up"))
}
/// our window's NSView (as an address, to use outside the app's update: AppKit hands the events to GPUI, which must
/// not be borrowed then)
#[cfg(target_os = "macos")]
pub fn view_of(window: &Window) -> Option<usize> {
    native::view(window).map(|v| v as usize)
}
/// (platform module: native test input is AppKit-only)
#[cfg(not(target_os = "macos"))]
pub fn view_of(_window: &Window) -> Option<usize> {
    None
}
#[cfg(not(target_os = "macos"))]
pub fn mouse_input(_c: &Value, _view: usize) {}

fn key_of(k: &str) -> Option<Keystroke> {
    // Computer Use names (cmd+shift+p, Return, Escape) → GPUI's (cmd-shift-p, enter, escape)
    let parts: Vec<String> = k.split('+').map(|p| match p {
        "Return" | "return" => "enter".into(),
        "Escape" => "escape".into(),
        "BackSpace" | "Backspace" => "backspace".into(),
        "Delete" => "delete".into(),
        "Tab" => "tab".into(),
        "Up" | "Down" | "Left" | "Right" | "Home" | "End" | "PageUp" | "PageDown" | "Space" => p.to_lowercase().replace("pageup", "pageup").replace("pagedown", "pagedown"),
        "command" | "super" => "cmd".into(),
        "equal" => "=".into(),
        "bracketright" => "]".into(),
        "bracketleft" => "[".into(),
        "comma" => ",".into(),
        "period" => ".".into(),
        "slash" => "/".into(),
        "plus" => "+".into(),
        "option" => "alt".into(),
        "control" => "ctrl".into(),
        p => p.to_string(),
    }).collect();
    let mut ks = if parts.last().map(|p| p == "minus" || p == "-").unwrap_or(false) {
        let mut k = Keystroke::parse(&parts[..parts.len() - 1].iter().cloned().chain(["a".to_string()]).collect::<Vec<_>>().join("-")).ok()?;
        k.key = "-".into();
        k
    } else {
        Keystroke::parse(&parts.join("-")).ok()?
    };
    if ks.modifiers.number_of_modifiers() == 0 || ks.modifiers.shift && ks.modifiers.number_of_modifiers() == 1 {
        if ks.key.chars().count() == 1 {
            ks.key_char = Some(ks.key.clone());
        } else if ks.key == "space" {
            ks.key_char = Some(" ".into());
        }
    }
    Some(ks)
}

#[cfg(target_os = "macos")]
/// Native mouse events (NSEvent) handed to our own window's view, as AppKit would: GPUI's real input path
/// (handle_view_event → hit test of the last frame). Our window only, never another app.
mod native {
    use objc::runtime::Object;
    use objc::{class, msg_send, sel, sel_impl};
    use raw_window_handle::{HasWindowHandle, RawWindowHandle};

    #[repr(C)]
    #[derive(Clone, Copy)]
    struct NSPoint {
        x: f64,
        y: f64,
    }
    unsafe impl objc::Encode for NSPoint {
        fn encode() -> objc::Encoding {
            unsafe { objc::Encoding::from_str("{CGPoint=dd}") }
        }
    }
    #[repr(C)]
    #[derive(Clone, Copy)]
    struct NSRect {
        o: NSPoint,
        s: NSPoint,
    }
    unsafe impl objc::Encode for NSRect {
        fn encode() -> objc::Encoding {
            unsafe { objc::Encoding::from_str("{CGRect={CGPoint=dd}{CGSize=dd}}") }
        }
    }

    pub fn view(window: &gpui::Window) -> Option<*mut Object> {
        match HasWindowHandle::window_handle(window).ok()?.as_raw() {
            RawWindowHandle::AppKit(h) => Some(h.ns_view.as_ptr() as *mut Object),
            _ => None,
        }
    }
    fn height(view: *mut Object) -> f64 {
        unsafe {
            let r: NSRect = msg_send![view, bounds];
            r.s.y
        }
    }
    /// kind: 1 left down, 2 left up, 3 right down, 4 right up, 5 moved, 6 left dragged, 25 other down, 26 other up
    pub fn mouse(view: *mut Object, kind: u64, x: f32, y: f32, clicks: i64) {
        unsafe {
            let win: *mut Object = msg_send![view, window];
            let num: i64 = msg_send![win, windowNumber];
            let loc = NSPoint { x: x as f64, y: height(view) - y as f64 };
            let nil: *mut Object = std::ptr::null_mut();
            let ev: *mut Object = msg_send![class!(NSEvent), mouseEventWithType: kind location: loc modifierFlags: 0u64 timestamp: 0.0f64 windowNumber: num context: nil eventNumber: 0i64 clickCount: clicks pressure: 1.0f32];
            if ev.is_null() {
                if std::env::var("OPTCHAT_TEST_DEBUG").is_ok() {
                    eprintln!("testcmd: no NSEvent for kind {kind}");
                }
                return;
            }
            if std::env::var("OPTCHAT_TEST_DEBUG").is_ok() {
                let l: NSPoint = msg_send![ev, locationInWindow];
                let t: u64 = msg_send![ev, type];
                eprintln!("testcmd: mouse kind {kind} type {t} at ({}, {}) win {num} h {}", l.x, l.y, height(view));
            }
            match kind {
                1 => { let _: () = msg_send![view, mouseDown: ev]; }
                2 => { let _: () = msg_send![view, mouseUp: ev]; }
                3 => { let _: () = msg_send![view, rightMouseDown: ev]; }
                4 => { let _: () = msg_send![view, rightMouseUp: ev]; }
                6 => { let _: () = msg_send![view, mouseDragged: ev]; }
                25 => { let _: () = msg_send![view, otherMouseDown: ev]; }
                26 => { let _: () = msg_send![view, otherMouseUp: ev]; }
                _ => { let _: () = msg_send![view, mouseMoved: ev]; }
            }
        }
    }
    #[repr(C)]
    #[derive(Clone, Copy)]
    struct CGPoint {
        x: f64,
        y: f64,
    }
    #[repr(C)]
    #[derive(Clone, Copy)]
    struct CGRect {
        o: CGPoint,
        s: CGPoint,
    }
    #[link(name = "CoreGraphics", kind = "framework")]
    unsafe extern "C" {
        fn CGEventCreateScrollWheelEvent2(src: *mut std::ffi::c_void, units: u32, count: u32, w1: i32, w2: i32, w3: i32) -> *mut std::ffi::c_void;
        fn CGEventSetLocation(e: *mut std::ffi::c_void, p: CGPoint);
        fn CGEventCreateMouseEvent(src: *mut std::ffi::c_void, kind: u32, p: CGPoint, button: u32) -> *mut std::ffi::c_void;
        fn CGEventSetIntegerValueField(e: *mut std::ffi::c_void, field: u32, v: i64);
        fn CGMainDisplayID() -> u32;
        fn CGDisplayBounds(d: u32) -> CGRect;
        fn CFRelease(p: *const std::ffi::c_void);
    }
    /// a right / middle button event (NSEvent's constructor has no button number): a CGEvent with the button, made
    /// into an NSEvent located at the view's (x, y) like `scroll`
    pub fn button(view: *mut Object, kind: u32, button: u32, x: f32, y: f32, clicks: i64) {
        unsafe {
            let main = CGDisplayBounds(CGMainDisplayID()).s.y;
            let want_y = height(view) - y as f64;
            let ev = CGEventCreateMouseEvent(std::ptr::null_mut(), kind, CGPoint { x: x as f64, y: main - want_y }, button);
            if ev.is_null() {
                return;
            }
            CGEventSetIntegerValueField(ev, 1 /* kCGMouseEventClickState */, clicks);
            let ns: *mut Object = msg_send![class!(NSEvent), eventWithCGEvent: ev];
            if !ns.is_null() {
                match kind {
                    3 => { let _: () = msg_send![view, rightMouseDown: ns]; }
                    4 => { let _: () = msg_send![view, rightMouseUp: ns]; }
                    25 => { let _: () = msg_send![view, otherMouseDown: ns]; }
                    _ => { let _: () = msg_send![view, otherMouseUp: ns]; }
                }
            }
            CFRelease(ev);
        }
    }
    /// a pixel-precise scroll at (x, y): a CGEvent made into an NSEvent whose location is that point in the window
    pub fn scroll(view: *mut Object, x: f32, y: f32, dx: f32, dy: f32) {
        unsafe {
            let ev = CGEventCreateScrollWheelEvent2(std::ptr::null_mut(), 0 /* pixel */, 2, dy as i32, dx as i32, 0);
            if ev.is_null() {
                return;
            }
            // a windowless event's locationInWindow is its screen location (bottom-left origin): put it where the
            // view's (x, y) maps to
            let main = CGDisplayBounds(CGMainDisplayID()).s.y;
            let want_y = height(view) - y as f64;
            CGEventSetLocation(ev, CGPoint { x: x as f64, y: main - want_y });
            let ns: *mut Object = msg_send![class!(NSEvent), eventWithCGEvent: ev];
            if !ns.is_null() {
                let _: () = msg_send![view, scrollWheel: ns];
            }
            CFRelease(ev);
        }
    }
}

#[cfg(target_os = "macos")]
fn mouse(v: *mut objc::runtime::Object, x: f32, y: f32, button: MouseButton, count: usize) {
    let (down, up) = match button {
        MouseButton::Right => (3, 4),
        MouseButton::Middle => (25, 26),
        _ => (1, 2),
    };
    native::mouse(v, 5, x, y, 0);
    match button {
        MouseButton::Left => {
            native::mouse(v, down, x, y, count as i64);
            native::mouse(v, up, x, y, count as i64);
        }
        _ => {
            let b = if down == 3 { 1 } else { 2 };
            native::button(v, down as u32, b, x, y, count as i64);
            native::button(v, up as u32, b, x, y, count as i64);
        }
    }
}

/// native mouse input, called outside any app update (see view_of)
#[cfg(target_os = "macos")]
pub fn mouse_input(c: &Value, view: usize) {
    let v = view as *mut objc::runtime::Object;
    let f = |k: &str| c[k].as_f64().unwrap_or(0.0) as f32;
    match c["op"].as_str().unwrap_or("") {
        "move" => {
            {
                native::mouse(v, 5, f("x"), f("y"), 0);
            }
        }
        "click" => {
            let button = match c["button"].as_str() {
                Some("right") => MouseButton::Right,
                Some("middle") => MouseButton::Middle,
                _ => MouseButton::Left,
            };
            let n = c["count"].as_u64().unwrap_or(1) as usize;
            for i in 1..=n {
                mouse(v, f("x"), f("y"), button, i);
            }
        }
        "drag" => {
            let (x0, y0, x1, y1) = (f("x0"), f("y0"), f("x1"), f("y1"));
            {
                native::mouse(v, 5, x0, y0, 0);
                native::mouse(v, 1, x0, y0, 1);
                for i in 1..=8 {
                    let t = i as f32 / 8.0;
                    native::mouse(v, 6, x0 + (x1 - x0) * t, y0 + (y1 - y0) * t, 1);
                }
                native::mouse(v, 2, x1, y1, 1);
            }
        }
        "scroll" => {
            {
                native::mouse(v, 5, f("x"), f("y"), 0);
                native::scroll(v, f("x"), f("y"), f("dx"), f("dy"));
            }
        }
        // a press and a release on their own (holds, long-presses)
        "down" => {
            native::mouse(v, 5, f("x"), f("y"), 0);
            native::mouse(v, 1, f("x"), f("y"), 1);
        }
        "up" => native::mouse(v, 2, f("x"), f("y"), 1),
        _ => {}
    }
    crate::ui::set_state_extra("lastCmd", serde_json::json!({ "op": c["op"], "ok": true }));
}

/// synthetic input through the window's own dispatch (hit tests and focus of the last laid-out frame)
pub fn input(c: &Value, window: &mut Window, cx: &mut App) {
    match c["op"].as_str().unwrap_or("") {
        "key" => {
            if let Some(k) = key_of(c["keys"].as_str().unwrap_or("")) {
                window.dispatch_keystroke(k, cx);
            }
        }
        // a held key (the tv remote's mic): down, then up later
        "keydown" | "keyup" => {
            if let Some(k) = key_of(c["keys"].as_str().unwrap_or("")) {
                let ev = if c["op"] == "keydown" { PlatformInput::KeyDown(KeyDownEvent { keystroke: k, is_held: false, prefer_character_input: false }) } else { PlatformInput::KeyUp(KeyUpEvent { keystroke: k }) };
                window.dispatch_event(ev, cx);
            }
        }
        "type" => {
            for ch in c["text"].as_str().unwrap_or("").chars() {
                let s = ch.to_string();
                let ks = if ch == '\n' { Keystroke { modifiers: Modifiers { shift: true, ..Default::default() }, key: "enter".into(), key_char: None } } else { Keystroke { modifiers: Modifiers::default(), key: if ch == ' ' { "space".into() } else { s.to_lowercase() }, key_char: Some(s) } };
                window.dispatch_keystroke(ks, cx);
            }
        }
        _ => {}
    }
    crate::ui::set_state_extra("lastCmd", serde_json::json!({ "op": c["op"], "ok": true }));
}

fn endpoint(w: &Workspace, c: &Value, cx: &App) -> String {
    c["endpoint"].as_str().map(String::from).unwrap_or_else(|| w.state.read(cx).engines.first().map(|e| e.cfg.endpoint.clone()).unwrap_or_default())
}

pub fn run(w: &mut Workspace, c: &Value, window: &mut Window, cx: &mut Context<Workspace>) {
    let str_ = |k: &str| c[k].as_str().unwrap_or("").to_string();
    let mut done = serde_json::json!({ "op": c["op"], "ok": true });
    match c["op"].as_str().unwrap_or("") {
        "act" => {
            let name: &'static str = Box::leak(str_("name").into_boxed_str());
            crate::dialogs::run_cmd(w, crate::dialogs::Cmd::Act(name), window, cx);
        }
        "menu" => crate::workspaces::open_panel(w, cx),
        "rail" => {
            let v: View = serde_json::from_value(c["view"].clone()).unwrap_or(View::Tasks);
            w.pick(v, cx);
        }
        "addEngine" => {
            // the panel's Add engine form (a token, an optional name), then its Add button's handler
            crate::workspaces::open_add_engine(w, window, cx);
            let (token, name) = (str_("token"), str_("name"));
            w.ws_panel.update(cx, |p, cx| {
                p.set_add_fields(&token, &name, window, cx);
                p.add(cx);
            });
        }
        "addEndpoint" => {
            // an engine by its address alone (older per-workspace tokens; tests on real workspaces)
            let (ep, name) = (str_("endpoint"), str_("name"));
            let _ = w.state.update(cx, |s, cx| s.add_engine(&ep, &name, cx));
            crate::workspaces::open_panel(w, cx);
        }
        "connect" => {
            // a workspace with its own token (OPTCHAT_TEST_TOKENS {name: token}): the older per-workspace path
            let ep = endpoint(w, c, cx);
            let ws = str_("ws");
            let m: std::collections::BTreeMap<String, String> = std::env::var("OPTCHAT_TEST_TOKENS").ok().and_then(|p| std::fs::read_to_string(p).ok()).and_then(|s| serde_json::from_str(&s).ok()).unwrap_or_default();
            let t = m.get(&ws).cloned().unwrap_or_default();
            crate::workspaces::open_panel(w, cx);
            let _ = w.state.update(cx, |s, cx| s.connect_ws(&ep, &ws, &t, cx));
        }
        "switch" => {
            let key = crate::client::ws_key(&endpoint(w, c, cx), &str_("ws"));
            w.switch_workspace(&key, window, cx);
        }
        "disconnect" => {
            let key = crate::client::ws_key(&endpoint(w, c, cx), &str_("ws"));
            w.state.update(cx, |s, cx| s.disconnect_ws(&key, cx));
        }
        "view" => {
            let v: View = serde_json::from_value(c["view"].clone()).unwrap_or(View::Tasks);
            w.state.update(cx, |s, _| s.prefs.view = v);
            w.set_side_open(true, cx);
        }
        "open" => w.open_task(c["task"].as_i64().unwrap_or(0), window, cx),
        "loadNotices" => w.state.update(cx, |s, cx| s.load_notices(cx)),
        "readNotice" => {
            let id = str_("id");
            let task = w.state.read(cx).notices.iter().find(|n| n.id == id).and_then(|n| n.task);
            w.state.update(cx, |s, cx| s.read_notices(Some(vec![id]), None, cx));
            if let Some(t) = task {
                w.open_task(t, window, cx);
            }
        }
        "readAll" => w.state.update(cx, |s, cx| s.read_notices(None, None, cx)),
        "spawn" => {
            // as the new-task dialog's Start does
            let (title, instruction) = (str_("title"), str_("instruction"));
            let usd = c["usd"].as_f64();
            // a model for the child (tests: Haiku, low), else the parent's
            let settings = c["model"].as_str().map(|m| optchat_proto::types::SettingsPatch { model: Some(m.to_string()), effort: Some("low".into()), ..Default::default() });
            let parent = w.state.read(cx).current;
            w.state.update(cx, |s, cx| {
                s.run(
                    cx,
                    move |h| async move { h.spawn(parent, "handoff", &title, &instruction, vec![], None, usd.map(|u| optchat_proto::api::BudgetArgs { usd: Some(u), ..Default::default() }), settings, true).await },
                    move |s, r, cx| match r {
                        Ok(id) => cx.emit(StateEvent::SpawnedOpen(id)),
                        Err(e) => s.local_notice("error", e),
                    },
                )
            });
        }
        "close" => w.close_overlay(window, cx),
        // draw a frame now (the tv's dev screenshot reads back the next frame)
        "redraw" => window.refresh(),
        "quit" => cx.quit(),
        "resize" => window.resize(size(px(c["w"].as_f64().unwrap_or(400.) as f32), px(c["h"].as_f64().unwrap_or(860.) as f32))),
        "nudge" => {
            // a 1-pt resize and back: AppKit draws (and presents) a resized window at once, even when it is covered
            let sz = window.viewport_size();
            window.resize(size(sz.width + px(1.), sz.height));
            window.resize(sz);
        }
        other => done = serde_json::json!({ "op": other, "ok": false }),
    }
    crate::ui::set_state_extra("lastCmd", done);
}
