//! Android entry point for the shared OptChat client and GPUI UI.
#[cfg(target_os = "android")]
mod android {
    use gpui::*;
    use gpui_mobile::android::jni;
    use optchat_ui::*;
    use std::sync::Arc;
    actions!(android_shell, [Back]);

    mod platform;

    struct Shell {
        workspace: Entity<workspace::Workspace>,
        dark: Option<bool>,
    }

    impl Render for Shell {
        fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
            use gpui::prelude::*;
            let insets = window.insets().effective();
            let dark = theme::c(cx).dark;
            if self.dark != Some(dark) {
                self.dark = Some(dark);
                let _ = jni::with_env(|env| {
                    let activity = jni::activity(env)?;
                    env.call_method(&activity, ::jni::jni_str!("gpuiAppearance"), ::jni::jni_sig!("(Z)V"),
                        &[::jni::objects::JValue::Bool(dark)]).map_err(|e| e.to_string())?;
                    Ok(())
                });
            }
            div().id("android-shell").size_full().bg(theme::c(cx).bg)
                .pt(insets.top).pb(insets.bottom).pl(insets.left).pr(insets.right)
                .child(self.workspace.clone())
                .on_action(cx.listener(|shell, _: &Back, window, cx| {
                    shell.workspace.update(cx, |workspace, cx| {
                        if workspace.overlay.is_some() || workspace.ask.is_some() || workspace.metrics_open || workspace.sheet {
                            window.dispatch_action(Box::new(workspace::Dismiss), cx);
                        } else if workspace.phone.screen.is_some() {
                            workspace.phone_back(cx);
                        } else if workspace.phone.expanded {
                            workspace.phone.expanded = false;
                            window.dismiss_virtual_keyboard();
                            window.blur(cx);
                            cx.notify();
                        } else {
                            gpui_mobile::android::editor::call_void(::jni::jni_str!("gpuiBackground"));
                        }
                    });
                }))
        }
    }

    #[no_mangle]
    fn android_main(app: android_activity::AndroidApp) {
        android_logger::init_once(android_logger::Config::default()
            .with_max_level(log::LevelFilter::Info).with_tag("OptChat"));
        jni::install_panic_hook();
        optchat_client::LAUNCH.get_or_init(std::time::Instant::now);
        jni::init_platform(&app);
        optchat_platform::install(Arc::new(platform::Android));
        let directory = app.internal_data_path().expect("Android private data directory");
        std::fs::create_dir_all(&directory).expect("Create settings directory");
        // Opt-in debug instrumentation, enabled only by an app-private file via adb run-as.
        // Normal launches write no test state or input contents.
        if cfg!(debug_assertions) && directory.join("test-mode").is_file() {
            std::env::set_var("OPTCHAT_TEST_HOOKS", "1");
            std::env::set_var("OPTCHAT_TEST_NO_GLOBAL", "1");
            std::env::set_var("OPTCHAT_LOCAL_TOKEN_FILE", directory.join("no-local-token"));
            std::env::set_var("OPTCHAT_PROBES", directory.join("probes.json"));
            ui::start_probe_writer();
        }
        let prefs_path = directory.join("prefs.json").to_string_lossy().into_owned();
        let rt = Box::leak(Box::new(tokio::runtime::Builder::new_multi_thread()
            .worker_threads(2).thread_name("optchat-client").enable_all().build().expect("Tokio")));
        let shared = jni::shared_platform().expect("Android GPUI platform");
        Application::with_platform(shared.into_rc()).with_assets(Assets).run(move |cx: &mut App| {
            kit::init(cx);
            ui::register_fonts(cx, assets::ASSETS);
            ui::set_executor(cx.background_executor().clone());
            let prefs: state::Prefs = std::fs::read_to_string(&prefs_path).ok()
                .and_then(|s| serde_json::from_str(&s).ok()).unwrap_or_default();
            theme::init(cx, prefs.theme.clone(), prefs.follow_system, cx.window_appearance());
            kit::set_font_size(cx, px(prefs.font_size - 0.5));
            ui::set_zoom(prefs.font_size);
            workspace::bind(cx);
            cx.bind_keys([KeyBinding::new("escape", Back, Some("Workspace"))]);
            composer::bind(cx);
            dialogs::bind(cx);
            let state = state::new_state(cx, rt.handle().clone(), prefs_path.clone());
            let st = state.clone();
            let needs_engine = prefs.engines.is_empty();
            cx.open_window(WindowOptions::default(), |window, cx| {
                theme::sync_window(window, cx);
                let workspace = cx.new(|cx| workspace::Workspace::new(st, window, cx));
                if needs_engine {
                    workspace.update(cx, |w, cx| w.phone_open(phone::Screen::Panel(state::View::Workspaces), cx));
                }
                cx.new(|_| Shell { workspace, dark: None })
            }).expect("Open OptChat");
            cx.activate(true);
            let (tx, rx) = tokio::sync::mpsc::unbounded_channel();
            state.update(cx, |s, cx| s.start(tx, rx, cx));
            log::info!("OPTCHAT_WINDOW_OPEN");
        });
    }
}
