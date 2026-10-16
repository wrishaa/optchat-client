//! Native Windows shell for the shared OptChat UI (DirectX and DirectWrite).
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]
#![recursion_limit = "256"] // the test state's json! literal
use gpui::*;
use optchat_client::LAUNCH;
use optchat_ui::*;
use optchat_ui::{assets, Assets};

#[cfg(not(target_os = "windows"))]
compile_error!("Build optchat-windows on Windows with the MSVC toolchain.");

mod platform;

actions!(app, [Quit]);

/// Packaged and unpackaged builds share settings; OPTCHAT_PREFS_DIR isolates tests.
fn prefs_path() -> anyhow::Result<String> {
    let dir = config_dir(
        std::env::var_os("OPTCHAT_PREFS_DIR"),
        std::env::var_os("APPDATA"),
    )?;
    std::fs::create_dir_all(&dir)?;
    dir.join("prefs.json")
        .into_os_string()
        .into_string()
        .map_err(|_| anyhow::anyhow!("OptChat settings path is not valid Unicode"))
}

fn config_dir(
    override_dir: Option<std::ffi::OsString>,
    appdata: Option<std::ffi::OsString>,
) -> anyhow::Result<std::path::PathBuf> {
    use std::path::PathBuf;
    if let Some(dir) = override_dir.filter(|d| !d.is_empty()) {
        return Ok(PathBuf::from(dir));
    }
    let dir = appdata
        .map(PathBuf::from)
        .filter(|d| d.is_absolute())
        .ok_or_else(|| anyhow::anyhow!("APPDATA must name an absolute settings directory"))?;
    Ok(dir.join("OptChat"))
}

/// Tests: OPTCHAT_ENGINE (an endpoint) and OPTCHAT_CONNECT (workspace names, comma-separated) add that engine and
/// those connections to the prefs at launch, their tokens from the JSON file OPTCHAT_TEST_TOKENS ({name: token}).
/// The tokens go to the token store only; they are never printed.
fn test_bootstrap(prefs_path: &str) {
    let Ok(endpoint) = std::env::var("OPTCHAT_ENGINE") else {
        return;
    };
    let Ok(endpoint) = client::EngineCfg::normalize(&endpoint) else {
        return;
    };
    let mut prefs: state::Prefs = std::fs::read_to_string(prefs_path)
        .ok()
        .and_then(|s| serde_json::from_str(&s).ok())
        .unwrap_or_default();
    if !prefs.engines.iter().any(|e| e.endpoint == endpoint) {
        let name = std::env::var("OPTCHAT_ENGINE_NAME")
            .unwrap_or_else(|_| endpoint.split("://").nth(1).unwrap_or("engine").to_string());
        prefs.engines.push(client::EngineCfg {
            endpoint: endpoint.clone(),
            name,
            ..Default::default()
        });
    }
    let tokens: std::collections::BTreeMap<String, String> = std::env::var("OPTCHAT_TEST_TOKENS")
        .ok()
        .and_then(|p| std::fs::read_to_string(p).ok())
        .and_then(|s| serde_json::from_str(&s).ok())
        .unwrap_or_default();
    for ws in std::env::var("OPTCHAT_CONNECT")
        .unwrap_or_default()
        .split(',')
        .map(str::trim)
        .filter(|w| !w.is_empty())
    {
        let c = state::ConnPref {
            endpoint: endpoint.clone(),
            ws: ws.to_string(),
        };
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
    // Install Windows hooks before anything touches tokens.
    optchat_platform::install(std::sync::Arc::new(crate::platform::Windows::new()));
    let prefs_path = prefs_path().expect("OptChat config directory");
    test_bootstrap(&prefs_path);
    let rt: &'static tokio::runtime::Runtime = Box::leak(Box::new(
        tokio::runtime::Builder::new_multi_thread()
            .worker_threads(2)
            .thread_name("optchat-client")
            .enable_all()
            .build()
            .expect("tokio runtime"),
    ));
    ui::start_probe_writer();

    gpui_platform::application()
        .with_assets(Assets)
        .run(move |cx: &mut App| {
            kit::init(cx);
            let t0 = std::time::Instant::now();
            ui::register_fonts(cx, assets::ASSETS);
            if std::env::var("OPTCHAT_TIMING").is_ok() {
                eprintln!(
                    "fonts registered in {:.1} ms",
                    t0.elapsed().as_secs_f64() * 1000.0
                );
            }
            ui::set_executor(cx.background_executor().clone());
            let prefs: state::Prefs = std::fs::read_to_string(&prefs_path)
                .ok()
                .and_then(|s| serde_json::from_str(&s).ok())
                .unwrap_or_default();
            theme::init(
                cx,
                prefs.theme.clone(),
                prefs.follow_system,
                cx.window_appearance(),
            );
            kit::set_font_size(cx, px(prefs.font_size - 0.5));
            crate::ui::set_zoom(prefs.font_size);
            workspace::bind(cx);
            composer::bind(cx);
            dialogs::bind(cx);
            cx.on_action(|_: &Quit, cx| cx.quit());
            cx.bind_keys([
                KeyBinding::new("ctrl-q", Quit, None),
                KeyBinding::new("ctrl-m", workspace::Minimize, None),
                KeyBinding::new("f11", workspace::ToggleFullScreen, None),
                KeyBinding::new("ctrl-/", workspace::ShowShortcuts, None),
            ]);
            // Windows editing conventions override the shared line-navigation bindings.
            use kit::input::*;
            cx.bind_keys([
                KeyBinding::new("ctrl-left", WordLeft, Some("Input")),
                KeyBinding::new("ctrl-right", WordRight, Some("Input")),
                KeyBinding::new("ctrl-backspace", DeleteWordLeft, Some("Input")),
                KeyBinding::new("ctrl-home", DocStart, Some("Input")),
                KeyBinding::new("ctrl-end", DocEnd, Some("Input")),
                KeyBinding::new("ctrl-shift-left", SelectWordLeft, Some("Input")),
                KeyBinding::new("ctrl-shift-right", SelectWordRight, Some("Input")),
                KeyBinding::new("ctrl-shift-home", SelectDocStart, Some("Input")),
                KeyBinding::new("ctrl-shift-end", SelectDocEnd, Some("Input")),
            ]);
            cx.on_window_closed(|cx, _| {
                if cx.windows().is_empty() {
                    cx.quit();
                }
            })
            .detach();

            let state = state::new_state(cx, rt.handle().clone(), prefs_path.clone());
            // the last window bounds, when they are sane; else 1360×880 centered
            let bounds = match prefs.window {
                Some([x, y, w, h])
                    if w >= 560.
                        && h >= 420.
                        && w < 10000.
                        && h < 10000.
                        && x > -5000.
                        && y > -5000. =>
                {
                    Bounds::new(point(px(x), px(y)), size(px(w), px(h)))
                }
                _ => Bounds::centered(None, size(px(1360.), px(880.)), cx),
            };
            let st2 = state.clone();
            let win = cx
                .open_window(
                    WindowOptions {
                        window_bounds: Some(WindowBounds::Windowed(bounds)),
                        titlebar: Some(TitlebarOptions {
                            title: Some("OptChat".into()),
                            appears_transparent: false,
                            traffic_light_position: None,
                        }),
                        window_min_size: Some(gpui::size(px(560.), px(420.))),
                        app_id: Some("com.reframe.optchat.gpui".into()),
                        focus: true,
                        show: true,
                        ..Default::default()
                    },
                    |window, cx| {
                        theme::sync_window(window, cx);
                        let ws = cx.new(|cx| workspace::Workspace::new(st2.clone(), window, cx));
                        ws.update(cx, |w, cx| {
                            w.composer.update(cx, |c, cx| c.focus(window, cx))
                        });
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
                let hubs: Vec<_> = s
                    .hub()
                    .into_iter()
                    .chain(s.bg.values().filter_map(|d| d.hub.clone()))
                    .collect();
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

#[cfg(test)]
mod tests {
    use super::config_dir;

    #[test]
    fn settings_use_appdata_and_allow_test_isolation() {
        assert_eq!(
            config_dir(None, Some(r"C:\Users\Zoë\AppData\Roaming".into())).unwrap(),
            std::path::PathBuf::from(r"C:\Users\Zoë\AppData\Roaming\OptChat")
        );
        assert_eq!(
            config_dir(Some(r"C:\scratch".into()), None).unwrap(),
            std::path::PathBuf::from(r"C:\scratch")
        );
        assert!(config_dir(None, None).is_err());
        assert!(config_dir(None, Some("relative".into())).is_err());
    }
}
