//! OptChat for iPhone and iPad: the shared client and UI (optchat-client, optchat-ui: the macOS app's code) in an
//! iOS shell. The shell adds what iOS needs around the shared workspace view: safe-area insets, keyboard avoidance
//! (the IME inset, animated like UIKit's), and footprint logging per lifecycle phase. The GPUI patches release the
//! drawables while backgrounded (patches/zed). The connection is seeded at launch for development (OPTCHAT_SEED:
//! base64 JSON {endpoint, workspace, token}): prefs get the engine + connection, the token goes to the
//! shared token store (Keychain), never printed.
#[cfg(target_os = "ios")]
mod shell {
    use gpui::*;
    use optchat_client::LAUNCH;
    use optchat_ui::*;
    use optchat_ui::{assets, Assets};

    fn prefs_path() -> String {
        let dir = format!("{}/Library/Application Support/OptChat", std::env::var("HOME").unwrap_or_default());
        let _ = std::fs::create_dir_all(&dir);
        format!("{dir}/prefs.json")
    }

    /// OPTCHAT_SEED (development): the engine + connection into prefs, the token (and a tunnel key) into the Keychain
    fn seed(prefs_path: &str) {
        let Some(v) = std::env::var("OPTCHAT_SEED").ok().and_then(|b| {
            use base64::Engine;
            serde_json::from_slice::<serde_json::Value>(&base64::engine::general_purpose::STANDARD.decode(b.trim()).ok()?).ok()
        }) else { return };
        let (Some(endpoint), Some(ws), Some(token)) = (v["endpoint"].as_str(), v["workspace"].as_str(), v["token"].as_str()) else { return };
        let Ok(endpoint) = client::EngineCfg::normalize(endpoint) else { return };
        let mut prefs: state::Prefs = std::fs::read_to_string(prefs_path).ok().and_then(|s| serde_json::from_str(&s).ok()).unwrap_or_default();
        if !prefs.engines.iter().any(|e| e.endpoint == endpoint) {
            // via serde: works whatever optional fields EngineCfg gains (they default)
            let name = endpoint.split("://").nth(1).unwrap_or("engine").to_string();
            if let Ok(e) = serde_json::from_value::<client::EngineCfg>(serde_json::json!({ "endpoint": endpoint, "name": name })) { prefs.engines.push(e); }
        }
        let c = state::ConnPref { endpoint: endpoint.clone(), ws: ws.to_string() };
        let ok = tokens::set(&c.key(), token).is_ok();
        if !prefs.connections.contains(&c) { prefs.connections.push(c); }
        prefs.active.get_or_insert_with(|| client::ws_key(&endpoint, ws));
        if let Ok(s) = serde_json::to_string_pretty(&prefs) { let _ = std::fs::write(prefs_path, s); }
        diag(&format!("seed: prefs written, token stored: {ok}"));
    }

    /// the iOS Platform (optchat-platform): tokens in the Keychain, the memory footprint, and the QR scanner
    /// (native/qr_scanner.m presents the camera; only oc1 engine tokens end a scan)
    mod qr {
        use gpui::{App, AppContext, Window};
        use optchat_ui::platform::{BonjourEvent, Keychain, NetPath, Platform, ScanDone, SecureStore};
        use std::sync::Arc;
        /// the Keychain service the app has always used (shared with the macOS app's item format)
        const KEYCHAIN_SERVICE: &str = "com.reframe.optchat.gpui";
        use std::ffi::{c_char, c_void, CStr};
        type Tx = tokio::sync::oneshot::Sender<Option<String>>;
        unsafe extern "C" {
            fn optchat_can_scan_qr() -> bool;
            fn optchat_scan_qr(cb: extern "C" fn(*const c_char, *mut c_void), ctx: *mut c_void) -> bool;
        }
        /// what the scanner accepts: a whole oc1 token (protocol-tokens-v1: 3 parts, base64url JSON header, v 1)
        #[unsafe(no_mangle)]
        pub extern "C" fn optchat_qr_valid(s: *const c_char) -> bool {
            if s.is_null() { return false; }
            let Ok(t) = unsafe { CStr::from_ptr(s) }.to_str() else { return false };
            t.starts_with("oc1.") && t.split('.').count() == 3 && optchat_client::client::parse_engine_token(t).is_ok()
        }
        extern "C" fn on_done(s: *const c_char, ctx: *mut c_void) {
            let tx = unsafe { Box::from_raw(ctx as *mut Tx) };
            let text = (!s.is_null()).then(|| unsafe { CStr::from_ptr(s) }.to_string_lossy().into_owned());
            super::diag(&format!("qr: {}", if text.is_some() { "scanned a token" } else { "closed" }));
            let _ = tx.send(text);
        }
        pub struct Ios;
        impl Platform for Ios {
            fn secure_store(&self) -> Option<Arc<dyn SecureStore>> {
                Some(Arc::new(Keychain { service: KEYCHAIN_SERVICE }))
            }
            fn resident_mb(&self) -> Option<f64> {
                let mb = super::footprint_mb();
                (mb >= 0.0).then_some(mb)
            }
            fn can_scan_qr(&self) -> bool {
                unsafe { optchat_can_scan_qr() }
            }
            /// lan-first: bonjour (dns_sd; Info.plist NSBonjourServices + NSLocalNetworkUsageDescription, the Local
            /// Network prompt on first use) and NWPathMonitor
            fn browse(&self, service: &str, on: Arc<dyn Fn(BonjourEvent) + Send + Sync>) -> Option<Box<dyn Send + Sync>> {
                optchat_ui::platform::apple_net::browse(service, on).map(|h| Box::new(h) as Box<dyn Send + Sync>)
            }
            fn watch_net(&self, on: Arc<dyn Fn(NetPath) + Send + Sync>) -> bool {
                optchat_ui::platform::apple_net::watch_net(on)
            }
            /// diag.log (Documents) + stderr, with wall-clock ms for switch timings
            fn log(&self, line: &str) {
                let ms = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_millis()).unwrap_or(0);
                super::diag(&format!("{ms} {line}"));
                eprintln!("{ms} {line}");
            }
            fn scan_qr(&self, window: &mut Window, cx: &mut App, done: ScanDone) {
                let (tx, rx) = tokio::sync::oneshot::channel::<Option<String>>();
                let ctx = Box::into_raw(Box::new(tx)) as *mut c_void;
                if !unsafe { optchat_scan_qr(on_done, ctx) } {
                    drop(unsafe { Box::from_raw(ctx as *mut Tx) });
                    return done(None, window, cx);
                }
                let handle = window.window_handle();
                cx.spawn(async move |cx| {
                    let text = rx.await.ok().flatten();
                    let _ = cx.update_window(handle, move |_, window, cx| done(text, window, cx));
                })
                .detach();
            }
        }
    }

    /// a line in Documents/diag.log (outcomes only, never values)
    pub fn diag(line: &str) {
        if let Ok(home) = std::env::var("HOME") {
            use std::io::Write;
            if let Ok(mut f) = std::fs::OpenOptions::new().create(true).append(true).open(std::path::Path::new(&home).join("Documents/diag.log")) { let _ = writeln!(f, "{line}"); }
        }
    }

    fn footprint_mb() -> f64 {
        unsafe {
            let mut info: mach2::task_info::task_vm_info = std::mem::zeroed();
            let mut count = (std::mem::size_of::<mach2::task_info::task_vm_info>() / 4) as u32;
            let kr = mach2::task::task_info(mach2::traps::mach_task_self(), mach2::task_info::TASK_VM_INFO, &mut info as *mut _ as *mut i32, &mut count);
            if kr == 0 { info.phys_footprint as f64 / 1_048_576. } else { -1. }
        }
    }

    /// The root view: the shared workspace inside the safe area, lifted above the keyboard.
    struct Shell {
        ws: Entity<workspace::Workspace>,
        kb_from: f32,
        kb_to: f32,
        kb_now: f32,
        kb_t0: std::time::Instant,
        dark: Option<bool>,
        autotest_open: Option<i64>,
    }

    impl Render for Shell {
        fn render(&mut self, window: &mut Window, _cx: &mut Context<Self>) -> impl IntoElement {
            // status bar text follows the theme
            let dark = theme::c(_cx).dark;
            if self.dark != Some(dark) {
                self.dark = Some(dark);
                gpui_ios::ios::set_status_bar_style(if dark { gpui_ios::StatusBarContentStyle::Light } else { gpui_ios::StatusBarContentStyle::Dark });
            }
            // debug builds: OPTCHAT_AUTOTEST_OPEN=<task> opens it once, 4 s after launch (the rest is driven by touch)
            if let Some(id) = self.autotest_open.take() {
                let ws = self.ws.clone();
                _cx.spawn_in(window, async move |_, cx| {
                    cx.background_executor().timer(std::time::Duration::from_secs(4)).await;
                    let _ = ws.update_in(cx, |w, window, cx| w.open_task(id, window, cx));
                }).detach();
            }
            // keyboard avoidance: follow the IME inset with a 250 ms ease-out (UIKit's keyboard curve is similar)
            let insets = window.insets();
            let target = insets.effective().bottom.as_f32();
            if (target - self.kb_to).abs() > 0.5 { self.kb_from = self.kb_now; self.kb_to = target; self.kb_t0 = std::time::Instant::now(); }
            let p = (self.kb_t0.elapsed().as_secs_f32() / 0.25).clamp(0., 1.);
            self.kb_now = self.kb_from + (self.kb_to - self.kb_from) * (1. - (1. - p).powi(3));
            if p < 1. { window.request_animation_frame(); }
            div().size_full().bg(theme::c(_cx).bg)
                // the status bar: iPad landscape reports no top safe area, but the status bar is still drawn
                .pt(insets.safe_area.top.max(px(24.))).pb(px(self.kb_now)).pl(insets.safe_area.left).pr(insets.safe_area.right)
                .child(self.ws.clone())
        }
    }

    pub fn main() {
        LAUNCH.get_or_init(std::time::Instant::now);
        // the iOS platform: before the seed hooks touch tokens
        optchat_ui::platform::install(std::sync::Arc::new(qr::Ios));
        let prefs_path = prefs_path();
        // OPTCHAT_RESET=1 (debug builds, tests): start from no prefs, as a fresh install would
        if cfg!(debug_assertions) && std::env::var("OPTCHAT_RESET").is_ok_and(|v| v == "1") {
            let _ = std::fs::remove_file(&prefs_path);
            diag("reset: prefs removed");
        }
        seed(&prefs_path);
        let rt: &'static tokio::runtime::Runtime = Box::leak(Box::new(tokio::runtime::Builder::new_multi_thread().worker_threads(2).thread_name("optchat-client").enable_all().build().expect("tokio runtime")));

        gpui_platform::application().with_assets(Assets).run(move |cx: &mut App| {
            kit::init(cx);
            ui::register_fonts(cx, assets::ASSETS);
            ui::set_executor(cx.background_executor().clone());
            let prefs: state::Prefs = std::fs::read_to_string(&prefs_path).ok().and_then(|s| serde_json::from_str(&s).ok()).unwrap_or_default();
            theme::init(cx, prefs.theme.clone(), prefs.follow_system, cx.window_appearance());
            kit::set_font_size(cx, px(prefs.font_size - 0.5));
            ui::set_zoom(prefs.font_size);
            workspace::bind(cx);
            composer::bind(cx);
            dialogs::bind(cx);
            cx.on_app_lifecycle(|phase, _| diag(&format!("{:?} {:.1} MB", phase, footprint_mb()))).detach();

            let state = state::new_state(cx, rt.handle().clone(), prefs_path.clone());
            let st = state.clone();
            cx.open_window(WindowOptions::default(), |window, cx| {
                theme::sync_window(window, cx);
                let ws = cx.new(|cx| workspace::Workspace::new(st.clone(), window, cx));
                let autotest_open = if cfg!(debug_assertions) { std::env::var("OPTCHAT_AUTOTEST_OPEN").ok().and_then(|v| v.parse().ok()) } else { None };
                cx.new(|_| Shell { ws, kb_from: 0., kb_to: 0., kb_now: 0., kb_t0: std::time::Instant::now(), dark: None, autotest_open })
            }).expect("open window");
            cx.activate(true);

            // the engines: one websocket per engine, carrying every connected workspace there
            let (tx, rx) = tokio::sync::mpsc::unbounded_channel();
            state.update(cx, |s, cx| {
                s.stats.launch_ms = LAUNCH.get().unwrap().elapsed().as_secs_f64() * 1000.0;
                s.start(tx, rx, cx)
            });
            // OPTCHAT_SEED_OC1 (base64 of an oc1 engine token; tests): added as the Add engine panel would
            if let Some(tok) = std::env::var("OPTCHAT_SEED_OC1").ok().and_then(|b| {
                use base64::Engine;
                String::from_utf8(base64::engine::general_purpose::STANDARD.decode(b.trim()).ok()?).ok()
            }) {
                state.update(cx, |s, cx| s.add_engine_token("", &tok, cx));
                diag("seed: oc1 engine token added");
            }
        });
    }
}

#[cfg(target_os = "ios")]
fn main() {
    shell::main();
}

#[cfg(not(target_os = "ios"))]
fn main() {
    eprintln!("OptChat iOS: build for an iOS target (app/build.sh).");
}
