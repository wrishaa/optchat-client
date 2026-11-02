//! OptChat for samsung tizen tvs: the shared UI (optchat-ui, the same Workspace as the Mac app) in a thin shell, like
//! shells/mac/src/main.rs, plus the tizen Platform (platform.rs) and the TV layout's remote keys (optchat_ui::tv).
//! A cdylib: the c# host (host/Program.cs) loads it, hands over the mic hooks (oc_set_mic) and calls oc_main().
#![recursion_limit = "256"]
use gpui::*;
use optchat_client::LAUNCH;
use optchat_ui::*;
use optchat_ui::{assets, Assets};
use std::sync::{Arc, OnceLock};

mod dev;
mod platform;

pub fn beacon(m: &str) {
    dev::beacon(m)
}

/// dev builds: gpui / wgpu log records (info and up) to the beacon
struct BeaconLog;
impl log::Log for BeaconLog {
    fn enabled(&self, m: &log::Metadata) -> bool {
        m.level() <= log::Level::Info && m.target() != "wayland_backend"
    }
    fn log(&self, r: &log::Record) {
        if self.enabled(r.metadata()) {
            beacon(&format!("log {} {} {}", r.level(), r.target(), r.args()));
        }
    }
    fn flush(&self) {}
}

/// the ui scale on the tv (gpui_linux's tizen_scale)
/// the default theme on the tv (until dj picks one from the preview)
const TV_THEME: &str = "optchat-dark";
const TV_SCALE: f32 = 1.5;

static MIC: OnceLock<Arc<platform::RemoteMic>> = OnceLock::new();

/// the c# host's mic start / stop (called on the remote's mic key down / up, through the shared dictation)
#[unsafe(no_mangle)]
pub extern "C" fn oc_set_mic(start: extern "C" fn(), stop: extern "C" fn()) {
    *platform::HOOKS.lock() = Some((start, stop));
}
/// pcm16 LE 16 kHz mono from the host's AudioCapture
#[unsafe(no_mangle)]
pub unsafe extern "C" fn oc_mic_pcm(p: *const u8, n: i32) {
    if p.is_null() || n <= 0 {
        return;
    }
    if let Some(m) = MIC.get() {
        m.pcm(unsafe { std::slice::from_raw_parts(p, n as usize) });
    }
}
/// the host's recording ended (the tail after the release included)
#[unsafe(no_mangle)]
pub extern "C" fn oc_mic_end() {
    if let Some(m) = MIC.get() {
        m.end();
    }
}
/// whether this is a dev build (the host's own log lines go to the mac then)
#[unsafe(no_mangle)]
pub extern "C" fn oc_is_dev() -> i32 {
    cfg!(feature = "dev-capture") as i32
}

/// the app's writable data dir (tizen: apps_rw/<pkg>/data)
fn data_dir() -> String {
    let d = ["/opt/usr/home/owner/apps_rw/so.reframe.octv/data".to_string(), format!("{}/data", std::env::var("HOME").unwrap_or_default())]
        .into_iter()
        .find(|d| std::fs::create_dir_all(d).is_ok() && std::fs::metadata(d).map(|m| !m.permissions().readonly()).unwrap_or(false))
        .unwrap_or_else(|| "/tmp/octv".into());
    let _ = std::fs::create_dir_all(&d);
    d
}

/// Dev builds only: shared/res/dev-seed.json ({engines: [{endpoint, name, ws, token}], active}: never committed, see
/// build.sh --seed) adds those engines and connections to the prefs and their tokens to the token store, like the Mac
/// shell's OPTCHAT_ENGINE test bootstrap. The tv has no keyboard to paste a token. Tokens are never logged.
fn seed(prefs_path: &str) {
    let root = "/opt/usr/home/owner/apps_rw/so.reframe.octv";
    let found = ["/opt/usr/home/owner/apps_rw/so.reframe.octv/shared/res", "/opt/usr/globalapps/so.reframe.octv/shared/res"].iter().find_map(|d| std::fs::read_to_string(format!("{d}/dev-seed.json")).ok());
    let Some(text) = found else {
        let ls = |d: &str| std::fs::read_dir(d).map(|r| r.filter_map(|e| e.ok()).map(|e| e.file_name().to_string_lossy().to_string()).collect::<Vec<_>>().join(",")).unwrap_or_else(|e| e.to_string());
        beacon(&format!("no dev seed: {root}: [{}] shared/res: [{}] exe {:?}", ls(root), ls(&format!("{root}/shared/res")), std::env::current_exe()));
        return;
    };
    let Ok(v) = serde_json::from_str::<serde_json::Value>(&text) else { return };
    let mut prefs: state::Prefs = std::fs::read_to_string(prefs_path).ok().and_then(|s| serde_json::from_str(&s).ok()).unwrap_or_default();
    for e in v["engines"].as_array().cloned().unwrap_or_default() {
        let (Some(ep), Some(ws), Some(tok)) = (e["endpoint"].as_str(), e["ws"].as_str(), e["token"].as_str()) else { continue };
        // the dev scratch engine is plain http on the lan (the shared normalize allows http on localhost only)
        let endpoint = client::EngineCfg::normalize(ep).unwrap_or_else(|_| ep.trim().trim_end_matches('/').to_string());
        if !prefs.engines.iter().any(|x| x.endpoint == endpoint) {
            prefs.engines.push(client::EngineCfg { endpoint: endpoint.clone(), name: e["name"].as_str().unwrap_or(ws).to_string(), ..Default::default() });
        }
        let c = state::ConnPref { endpoint: endpoint.clone(), ws: ws.to_string() };
        if tokens::get(&c.key()).is_none() {
            let _ = tokens::set(&c.key(), tok);
        }
        if !prefs.connections.contains(&c) {
            prefs.connections.push(c);
        }
    }
    {
        // "active": a workspace name (dev: the scratch one, so test sends never land in a real chat)
        prefs.active = v["active"].as_str().and_then(|w| prefs.connections.iter().find(|c| c.ws == w)).map(|c| c.key());
    }
    beacon(&format!("seeded {} engines, {} connections", prefs.engines.len(), prefs.connections.len()));
    if let Ok(s) = serde_json::to_string_pretty(&prefs) {
        let _ = std::fs::write(prefs_path, s);
    }
}

#[unsafe(no_mangle)]
pub extern "C" fn oc_main() -> i32 {
    LAUNCH.get_or_init(std::time::Instant::now);
    if cfg!(feature = "dev-capture") {
        let _ = log::set_logger(&BeaconLog).map(|_| log::set_max_level(log::LevelFilter::Info));
        std::panic::set_hook(Box::new(|p| beacon(&format!("panic {p}"))));
    }
    // the tv's mali egl presents fine but doesn't flag NATIVE_RENDERABLE (see vendor/wgpu-hal egl.rs)
    unsafe { std::env::set_var("WGPU_GLES_PRESENT_NON_NATIVE", "1") };
    // the whole ui at 1.5× (a 1280×720 layout on the 1080p panel): read from across a room, every view alike
    unsafe { std::env::set_var("GPUI_TIZEN_SCALE", TV_SCALE.to_string()) };
    let data = data_dir();
    beacon(&format!("oc_main start data={data}"));
    dev::init(&data);
    let mic = MIC.get_or_init(|| Arc::new(platform::RemoteMic::default())).clone();
    optchat_platform::install(Arc::new(platform::Tizen { store: Arc::new(platform::FileStore { path: format!("{data}/tokens.json") }), mic }));
    tv::enable();
    let prefs_path = format!("{data}/prefs.json");
    if cfg!(feature = "dev-capture") {
        seed(&prefs_path);
    }
    let rt: &'static tokio::runtime::Runtime = Box::leak(Box::new(tokio::runtime::Builder::new_multi_thread().worker_threads(2).thread_name("optchat-client").enable_all().build().expect("tokio runtime")));
    ui::start_probe_writer();
    dev::start();

    gpui_platform::application().with_assets(Assets).run(move |cx: &mut App| {
        kit::init(cx);
        ui::register_fonts(cx, assets::ASSETS);
        ui::set_executor(cx.background_executor().clone());
        let prefs: state::Prefs = std::fs::read_to_string(&prefs_path).ok().and_then(|s| serde_json::from_str(&s).ok()).unwrap_or_default();
        // the tv's default theme: dark (a set pref wins); the system appearance isn't followed on the tv
        theme::init(cx, Some(prefs.theme.clone().unwrap_or_else(|| TV_THEME.into())), false, cx.window_appearance());
        if cfg!(feature = "theme-preview") {
            tv::start_theme_preview(cx);
        }
        kit::set_font_size(cx, px(prefs.font_size - 0.5));
        crate::ui::set_zoom(prefs.font_size);
        workspace::bind(cx);
        composer::bind(cx);
        dialogs::bind(cx);
        tv::bind(cx);

        let state = state::new_state(cx, rt.handle().clone(), prefs_path.clone());
        let bounds = Bounds::centered(None, size(px(1920. / TV_SCALE), px(1080. / TV_SCALE)), cx);
        let st2 = state.clone();
        let opened = cx.open_window(
            WindowOptions { window_bounds: Some(WindowBounds::Fullscreen(bounds)), app_id: Some("so.reframe.octv".into()), focus: true, show: true, ..Default::default() },
            |window, cx| {
                theme::sync_window(window, cx);
                let ws = cx.new(|cx| workspace::Workspace::new(st2.clone(), window, cx));
                let f = ws.read(cx).focus.clone();
                window.focus(&f, cx);
                ws
            },
        );
        match opened {
            Ok(_) => beacon("window opened"),
            Err(e) => beacon(&format!("open_window err {e:#}")),
        }
        cx.activate(true);
        let (tx, rx) = tokio::sync::mpsc::unbounded_channel();
        state.update(cx, |s, cx| {
            s.stats.launch_ms = LAUNCH.get().unwrap().elapsed().as_secs_f64() * 1000.0;
            s.start(tx, rx, cx)
        });
    });
    beacon("run returned");
    0
}
