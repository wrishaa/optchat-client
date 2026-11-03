//! OptChat for Mac, GPUI edition: a native client of the OptChat hub (the web app's engine at chat.reframe.dev),
//! an equal of the browser and the phone: one websocket, everything in sync. The connection runs on its own tokio
//! runtime; this process draws the UI with GPUI and never blocks its thread on I/O.
#![recursion_limit = "256"] // the test state's json! literal
use gpui::*;
use optchat_ui::*;
use optchat_ui::{assets, Assets};
use optchat_client::LAUNCH;

mod platform;

actions!(app, [Quit, Hide, HideOthers, ShowAll]);

/// The app's prefs (engines, connections, each workspace's tabs, theme, window): `~/Library/Application
/// Support/com.reframe.optchat.gpui/prefs.json`; OPTCHAT_PREFS_DIR overrides the folder (tests).
fn prefs_path() -> String {
    let dir = std::env::var("OPTCHAT_PREFS_DIR").unwrap_or_else(|_| format!("{}/Library/Application Support/com.reframe.optchat.gpui", std::env::var("HOME").unwrap_or_default()));
    let _ = std::fs::create_dir_all(&dir);
    format!("{dir}/prefs.json")
}

/// Tests: OPTCHAT_ENGINE (an endpoint) and OPTCHAT_CONNECT (workspace names, comma-separated) add that engine and
/// those connections to the prefs at launch, their tokens from the JSON file OPTCHAT_TEST_TOKENS ({name: token}).
/// The tokens go to the token store only; they are never printed.
fn test_bootstrap(prefs_path: &str) {
    let Ok(endpoint) = std::env::var("OPTCHAT_ENGINE") else { return };
    let Ok(endpoint) = client::EngineCfg::normalize(&endpoint) else { return };
    let mut prefs: state::Prefs = std::fs::read_to_string(prefs_path).ok().and_then(|s| serde_json::from_str(&s).ok()).unwrap_or_default();
    if !prefs.engines.iter().any(|e| e.endpoint == endpoint) {
        let name = std::env::var("OPTCHAT_ENGINE_NAME").unwrap_or_else(|_| endpoint.split("://").nth(1).unwrap_or("engine").to_string());
        prefs.engines.push(client::EngineCfg { endpoint: endpoint.clone(), name, ..Default::default() });
    }
    let tokens: std::collections::BTreeMap<String, String> = std::env::var("OPTCHAT_TEST_TOKENS").ok().and_then(|p| std::fs::read_to_string(p).ok()).and_then(|s| serde_json::from_str(&s).ok()).unwrap_or_default();
    for ws in std::env::var("OPTCHAT_CONNECT").unwrap_or_default().split(',').map(str::trim).filter(|w| !w.is_empty()) {
        let c = state::ConnPref { endpoint: endpoint.clone(), ws: ws.to_string() };
        if let Some(t) = tokens.get(ws) {
            let _ = tokens::set(&c.key(), t);
        }
        if !prefs.connections.contains(&c) {
            prefs.connections.push(c);
        }
    }
    if let Ok(s) = serde_json::to_string_pretty(&prefs) {
        let _ = std::fs::write(prefs_path, s);
    }
}

fn main() {
    LAUNCH.get_or_init(std::time::Instant::now);
    // the macOS platform (Keychain, memory, the test QR "scanner" when OPTCHAT_TEST_QR is set): before anything
    // touches tokens
    optchat_platform::install(std::sync::Arc::new(crate::platform::Mac::new()));
    // OPTCHAT_TV=1: the TV layout (crates/ui/src/tv.rs) in a 1280×720 window (the tv's 1080p at 1.5×), a fast stand-in for the tizen shell
    let tv = std::env::var("OPTCHAT_TV").is_ok();
    if tv {
        optchat_ui::tv::enable();
    }
    let prefs_path = prefs_path();
    test_bootstrap(&prefs_path);
    let rt: &'static tokio::runtime::Runtime = Box::leak(Box::new(tokio::runtime::Builder::new_multi_thread().worker_threads(2).thread_name("optchat-client").enable_all().build().expect("tokio runtime")));
    ui::start_probe_writer();

    gpui_platform::application().with_assets(Assets).run(move |cx: &mut App| {
        kit::init(cx);
        let t0 = std::time::Instant::now();
        ui::register_fonts(cx, assets::ASSETS);
        if std::env::var("OPTCHAT_TIMING").is_ok() {
            eprintln!("fonts registered in {:.1} ms", t0.elapsed().as_secs_f64() * 1000.0);
        }
        ui::set_executor(cx.background_executor().clone());
        let prefs: state::Prefs = std::fs::read_to_string(&prefs_path).ok().and_then(|s| serde_json::from_str(&s).ok()).unwrap_or_default();
        theme::init(cx, prefs.theme.clone(), prefs.follow_system, cx.window_appearance());
        kit::set_font_size(cx, px(prefs.font_size - 0.5));
        crate::ui::set_zoom(prefs.font_size);
        workspace::bind(cx);
        composer::bind(cx);
        dialogs::bind(cx);
        cx.on_action(|_: &Quit, cx| cx.quit());
        cx.on_action(|_: &Hide, cx| cx.hide());
        cx.on_action(|_: &HideOthers, cx| cx.hide_other_apps());
        cx.on_action(|_: &ShowAll, cx| cx.unhide_other_apps());
        cx.bind_keys([
            KeyBinding::new("cmd-q", Quit, None),
            KeyBinding::new("cmd-h", Hide, None),
            KeyBinding::new("alt-cmd-h", HideOthers, None),
            KeyBinding::new("cmd-m", workspace::Minimize, None),
            KeyBinding::new("ctrl-cmd-f", workspace::ToggleFullScreen, None),
            KeyBinding::new("cmd-/", workspace::ShowShortcuts, None),
        ]);
        set_menus(cx);
        if tv {
            tv::bind(cx);
        }

        let state = state::new_state(cx, rt.handle().clone(), prefs_path.clone());
        // the last window bounds, when they are sane; else 1360×880 centered
        let bounds = match prefs.window {
            _ if tv => Bounds::centered(None, size(px(1280.), px(720.)), cx), // the tv's layout size (1920×1080 at 1.5)
            Some([x, y, w, h]) if w >= 560. && h >= 420. && w < 10000. && h < 10000. && x > -5000. && y > -5000. => Bounds::new(point(px(x), px(y)), size(px(w), px(h))),
            _ => Bounds::centered(None, size(px(1360.), px(880.)), cx),
        };
        let st2 = state.clone();
        let win = cx
            .open_window(
                WindowOptions {
                    window_bounds: Some(WindowBounds::Windowed(bounds)),
                    titlebar: Some(TitlebarOptions { title: Some("OptChat".into()), appears_transparent: false, traffic_light_position: None }),
                    window_min_size: Some(gpui::size(px(560.), px(420.))),
                    app_id: Some("com.reframe.optchat.gpui".into()),
                    focus: true,
                    show: true,
                    ..Default::default()
                },
                |window, cx| {
                    theme::sync_window(window, cx);
                    let ws = cx.new(|cx| workspace::Workspace::new(st2.clone(), window, cx));
                    ws.update(cx, |w, cx| w.composer.update(cx, |c, cx| c.focus(window, cx)));
                    if tv {
                        let f = ws.read(cx).focus.clone();
                        window.focus(&f, cx);
                    }
                    ws
                },
            )
            .expect("open window");
        let _ = win;
        cx.activate(true);

        // the engines: one websocket per engine (and token), carrying every connected workspace there
        let (tx, rx) = tokio::sync::mpsc::unbounded_channel();
        state.update(cx, |s, cx| {
            s.stats.launch_ms = LAUNCH.get().unwrap().elapsed().as_secs_f64() * 1000.0;
            s.start(tx, rx, cx)
        });
        let st4 = state.clone();
        cx.on_app_quit(move |cx| {
            let s = st4.read(cx);
            let conns = s.conns.clone();
            let hubs: Vec<_> = s.hub().into_iter().chain(s.bg.values().filter_map(|d| d.hub.clone())).collect();
            async move {
                for h in hubs {
                    h.shutdown();
                }
                for c in conns {
                    c.shutdown();
                }
            }
        })
        .detach();
    });
}


fn set_menus(cx: &mut App) {
    use crate::kit::input::{Copy, Cut, Paste, Redo, SelectAll, Undo};
    use workspace::*;
    cx.set_menus(vec![
        Menu {
            name: "OptChat".into(), disabled: false,
            items: vec![
                MenuItem::action("About OptChat", About),
                MenuItem::separator(),
                MenuItem::action("Settings…", ShowSettings),
                MenuItem::separator(),
                MenuItem::os_submenu("Services", SystemMenuType::Services),
                MenuItem::separator(),
                MenuItem::action("Hide OptChat", Hide),
                MenuItem::action("Hide Others", HideOthers),
                MenuItem::action("Show All", ShowAll),
                MenuItem::separator(),
                MenuItem::action("Quit OptChat", Quit),
            ],
        },
        Menu { name: "File".into(), disabled: false, items: vec![MenuItem::action("New Task…", NewTask), MenuItem::action("Open Main Chat", OpenMainChat), MenuItem::action("Go to Task…", TaskSwitcher), MenuItem::separator(), MenuItem::action("Close Tab", CloseTab)] },
        Menu {
            name: "Edit".into(), disabled: false,
            items: vec![
                MenuItem::os_action("Undo", Undo, OsAction::Undo),
                MenuItem::os_action("Redo", Redo, OsAction::Redo),
                MenuItem::separator(),
                MenuItem::os_action("Cut", Cut, OsAction::Cut),
                MenuItem::os_action("Copy", Copy, OsAction::Copy),
                MenuItem::os_action("Paste", Paste, OsAction::Paste),
                MenuItem::os_action("Select All", SelectAll, OsAction::SelectAll),
            ],
        },
        Menu {
            name: "View".into(), disabled: false,
            items: vec![
                MenuItem::action("Command Palette…", CommandPalette),
                MenuItem::separator(),
                MenuItem::action("Toggle Sidebar", ToggleSidebar),
                MenuItem::action("Tasks", ShowTasks),
                MenuItem::action("Memory", ShowMemory),
                MenuItem::action("Context", ShowContext),
                MenuItem::action("Log", ShowLog),
                MenuItem::action("Models", ShowModels),
                MenuItem::separator(),
                MenuItem::action("Next Theme", NextTheme),
                MenuItem::action("Bigger Text", ZoomIn),
                MenuItem::action("Smaller Text", ZoomOut),
                MenuItem::action("Actual Size", ZoomReset),
                MenuItem::separator(),
                MenuItem::action("Next Tab", NextTab),
                MenuItem::action("Previous Tab", PrevTab),
            ],
        },
        Menu {
            name: "Task".into(), disabled: false,
            items: vec![
                MenuItem::action("Focus Composer", FocusComposer),
                MenuItem::action("Stop the Running Turn", StopTurn),
                MenuItem::separator(),
                MenuItem::action("Pause…", PauseTask),
                MenuItem::action("Resume…", ResumeTask),
                MenuItem::action("Close as Done…", CloseTask),
                MenuItem::action("Cancel…", CancelTask),
            ],
        },
        Menu {
            name: "Window".into(), disabled: false,
            items: vec![
                MenuItem::action("Minimize", Minimize),
                MenuItem::action("Zoom", ZoomWindow),
                MenuItem::action("Enter Full Screen", ToggleFullScreen),
                MenuItem::separator(),
                MenuItem::action("Show Next Tab", NextTab),
                MenuItem::action("Show Previous Tab", PrevTab),
                MenuItem::action("Main Chat", OpenMainChat),
            ],
        },
        Menu {
            name: "Help".into(), disabled: false,
            items: vec![MenuItem::action("Keyboard Shortcuts", ShowShortcuts), MenuItem::action("Commands…", CommandPalette), MenuItem::separator(), MenuItem::action("Show Data Folder", OpenDataFolder)],
        },
    ]);
}
