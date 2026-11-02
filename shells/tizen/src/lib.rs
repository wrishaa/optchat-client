//! optchat tizen tv shell, milestone 1: prove gpui can put pixels on a samsung tv.
use gpui::{div, prelude::*, px, rgb, size, App, Bounds, Context, Window, WindowBounds, WindowOptions};
use std::io::{Read, Write};

/// debug beacon to the mac log server (same one the web shell used); remove before shipping.
pub fn beacon(m: &str) {
    let enc: String = m.bytes().map(|b| if b.is_ascii_alphanumeric() || b"-_.~".contains(&b) { (b as char).to_string() } else { format!("%{:02X}", b) }).collect();
    if let Ok(mut s) = std::net::TcpStream::connect_timeout(&"192.0.2.1:8765".parse().unwrap(), std::time::Duration::from_secs(2)) {
        let _ = write!(s, "GET /?m=gpui+{} HTTP/1.0\r\nHost: x\r\n\r\n", enc);
        let _ = s.read(&mut [0u8; 16]);
    }
}

/// forwards gpui/wgpu log records (info and up) to the beacon while we bring the shell up.
struct BeaconLog;
impl log::Log for BeaconLog {
    fn enabled(&self, m: &log::Metadata) -> bool { m.level() <= log::Level::Info && m.target() != "wayland_backend" }
    fn log(&self, r: &log::Record) { if self.enabled(r.metadata()) { beacon(&format!("log {} {} {}", r.level(), r.target(), r.args())); } }
    fn flush(&self) {}
}

struct Hello { frames: u32 }

impl Render for Hello {
    fn render(&mut self, window: &mut Window, _cx: &mut Context<Self>) -> impl IntoElement {
        self.frames += 1;
        if self.frames <= 3 { beacon(&format!("render {} viewport {:?}", self.frames, window.viewport_size())); }
        div().size_full().bg(rgb(0x1e1e2e)).flex().flex_col().justify_center().items_center().gap_6()
            .child(div().w(px(640.)).h(px(240.)).bg(rgb(0xe8590c)).rounded_lg())
            .child(div().text_size(px(56.)).text_color(rgb(0xffffff)).child("optchat gpui on tizen: hello"))
    }
}

#[unsafe(no_mangle)]
pub extern "C" fn oc_main() -> i32 {
    let _ = log::set_logger(&BeaconLog).map(|_| log::set_max_level(log::LevelFilter::Info));
    // the tv's mali egl presents fine but doesn't flag NATIVE_RENDERABLE (see vendor/wgpu-hal egl.rs)
    unsafe { std::env::set_var("WGPU_GLES_PRESENT_NON_NATIVE", "1") };
    std::panic::set_hook(Box::new(|p| beacon(&format!("panic {p}"))));
    beacon(&format!("oc_main start WAYLAND_DISPLAY={:?}", std::env::var("WAYLAND_DISPLAY")));
    gpui_platform::application().run(|cx: &mut App| {
        let fonts: Vec<std::borrow::Cow<'static, [u8]>> = vec![std::borrow::Cow::Borrowed(include_bytes!("../../../vendor/zed/assets/fonts/ibm-plex-sans/IBMPlexSans-Regular.ttf").as_slice())];
        if let Err(e) = cx.text_system().add_fonts(fonts) { beacon(&format!("add_fonts err {e}")); }
        let bounds = Bounds::centered(None, size(px(1920.), px(1080.)), cx);
        match cx.open_window(WindowOptions { window_bounds: Some(WindowBounds::Fullscreen(bounds)), ..Default::default() }, |_, cx| cx.new(|_| Hello { frames: 0 })) {
            Ok(_) => beacon("window opened"),
            Err(e) => beacon(&format!("open_window err {e:#}")),
        }
        cx.activate(true);
    });
    beacon("run returned");
    0
}
