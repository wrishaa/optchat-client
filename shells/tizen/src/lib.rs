//! optchat tizen tv shell, milestone 1: prove gpui can put pixels on a samsung tv.
use gpui::{div, prelude::*, px, rgb, size, App, Bounds, Context, FocusHandle, KeyDownEvent, KeyUpEvent, Window, WindowBounds, WindowOptions};
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

/// mic hooks the c# host hands us (Tizen.Multimedia AudioCapture lives on the .net side). null until set.
static MIC: std::sync::Mutex<Option<(extern "C" fn(), extern "C" fn())>> = std::sync::Mutex::new(None);

#[unsafe(no_mangle)]
pub extern "C" fn oc_set_mic(start: extern "C" fn(), stop: extern "C" fn()) {
    *MIC.lock().unwrap() = Some((start, stop));
}
fn mic(on: bool) {
    let hooks = *MIC.lock().unwrap();
    match hooks { Some((start, stop)) => if on { start() } else { stop() }, None => beacon("mic: no host hooks") }
}

const WORDS: [&str; 5] = ["hello", "from", "the", "tv", "pineapple"];

struct Hello { frames: u32, focus: FocusHandle, sel: usize, draft: String, sent: Vec<String>, mic_on: bool, last_key: String }

/// dev-only screenshots: the renderer reads back a frame on request, we png it and POST it to the mac.
#[cfg(feature = "dev-capture")]
mod devcap {
    use std::io::{Read, Write};
    const SRV: &str = "192.0.2.1:8767";
    fn http(req: &[u8]) -> Option<Vec<u8>> {
        let mut s = std::net::TcpStream::connect_timeout(&SRV.parse().ok()?, std::time::Duration::from_secs(2)).ok()?;
        s.set_read_timeout(Some(std::time::Duration::from_secs(5))).ok()?;
        s.write_all(req).ok()?;
        let mut out = Vec::new();
        let _ = s.read_to_end(&mut out);
        Some(out)
    }
    /// true once per pending request on the mac (`scripts/tvshot.sh` arms it).
    pub fn wanted() -> bool { http(b"GET /want HTTP/1.0\r\nHost: x\r\n\r\n").is_some_and(|r| r.ends_with(b"\r\n\r\n1")) }
    fn sink(w: u32, h: u32, px: Vec<u8>) {
        std::thread::spawn(move || {
            let t = std::time::Instant::now();
            let mut png = Vec::new();
            let enc = image::codecs::png::PngEncoder::new_with_quality(&mut png, image::codecs::png::CompressionType::Fast, image::codecs::png::FilterType::Sub);
            if let Err(e) = image::ImageEncoder::write_image(enc, &px, w, h, image::ExtendedColorType::Rgba8) { super::beacon(&format!("cap png err {e}")); return; }
            let mut req = format!("POST /frame HTTP/1.0\r\nHost: x\r\nContent-Type: image/png\r\nContent-Length: {}\r\n\r\n", png.len()).into_bytes();
            req.extend_from_slice(&png);
            let ok = http(&req).is_some();
            super::beacon(&format!("cap {w}x{h} png {} bytes in {}ms sent {ok}", png.len(), t.elapsed().as_millis()));
        });
    }
    pub fn install() { gpui_wgpu::frame_capture::set_capture_sink(sink); }
    pub fn request() { gpui_wgpu::frame_capture::request_capture(); }
}

impl Hello {
    fn new(cx: &mut Context<Self>) -> Self {
        #[cfg(feature = "dev-capture")]
        {
            devcap::install();
            cx.spawn(async move |this, cx| loop {
                cx.background_executor().timer(std::time::Duration::from_secs(1)).await;
                if cx.background_executor().spawn(async { devcap::wanted() }).await {
                    devcap::request();
                    let _ = this.update(cx, |_, cx| cx.notify());
                }
            }).detach();
        }
        Hello { frames: 0, focus: cx.focus_handle(), sel: 0, draft: String::new(), sent: Vec::new(), mic_on: false, last_key: "-".into() }
    }

    /// remote keys: up/down pick a word, OK (enter) appends it, red deletes the last word, green "sends" (local only),
    /// mic button (XF86BTVoice, grabbed via tizen_keyrouter) records while held.
    fn key_down(&mut self, ev: &KeyDownEvent, cx: &mut Context<Self>) {
        let k = ev.keystroke.key.as_str();
        if !ev.is_held { beacon(&format!("key down {k}")); }
        self.last_key = k.to_string();
        match k {
            "up" | "left" => self.sel = (self.sel + WORDS.len() - 1) % WORDS.len(),
            "down" | "right" => self.sel = (self.sel + 1) % WORDS.len(),
            "enter" if !ev.is_held => { if !self.draft.is_empty() { self.draft.push(' '); } self.draft.push_str(WORDS[self.sel]); }
            "xf86red" if !ev.is_held => {
                let t = self.draft.trim_end();
                self.draft = match t.rfind(' ') { Some(i) => t[..i].to_string(), None => String::new() };
            }
            "xf86green" if !ev.is_held && !self.draft.trim().is_empty() => { self.sent.push(std::mem::take(&mut self.draft)); }
            "xf86btvoice" if !self.mic_on => { self.mic_on = true; mic(true); }
            _ => {}
        }
        cx.notify();
    }
    fn key_up(&mut self, ev: &KeyUpEvent, cx: &mut Context<Self>) {
        if ev.keystroke.key == "xf86btvoice" && self.mic_on { self.mic_on = false; mic(false); beacon("key up xf86btvoice"); cx.notify(); }
    }
}

impl Render for Hello {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        self.frames += 1;
        if self.frames <= 3 { beacon(&format!("render {} viewport {:?}", self.frames, window.viewport_size())); }
        let words = WORDS.iter().enumerate().map(|(i, w)| {
            let on = i == self.sel;
            div().px(px(28.)).py(px(14.)).rounded_lg().text_size(px(44.))
                .bg(if on { rgb(0xe8590c) } else { rgb(0x313244) })
                .text_color(rgb(0xffffff)).child(*w)
        });
        div().track_focus(&self.focus)
            .on_key_down(cx.listener(|this, ev: &KeyDownEvent, _, cx| this.key_down(ev, cx)))
            .on_key_up(cx.listener(|this, ev: &KeyUpEvent, _, cx| this.key_up(ev, cx)))
            .size_full().bg(rgb(0x1e1e2e)).flex().flex_col().justify_center().items_center().gap_6()
            .child(div().text_size(px(56.)).text_color(rgb(0xffffff)).child("optchat gpui on tizen"))
            .child(div().flex().gap_4().children(words))
            .child(div().w(px(1200.)).min_h(px(90.)).px(px(24.)).py(px(16.)).rounded_lg()
                .bg(if self.mic_on { rgb(0x7f1d1d) } else { rgb(0x11111b) })
                .text_size(px(44.)).text_color(rgb(0xffffff))
                .child(if self.draft.is_empty() { "…".to_string() } else { self.draft.clone() }))
            .child(div().text_size(px(32.)).text_color(rgb(0xa6adc8)).child(format!("key: {}   mic: {}   sent (local only): {}", self.last_key, if self.mic_on { "recording" } else { "off" }, self.sent.join(" | "))))
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
        match cx.open_window(WindowOptions { window_bounds: Some(WindowBounds::Fullscreen(bounds)), ..Default::default() }, |window, cx| { let h = cx.new(Hello::new); let f = h.read(cx).focus.clone(); window.focus(&f, cx); h }) {
            Ok(_) => beacon("window opened"),
            Err(e) => beacon(&format!("open_window err {e:#}")),
        }
        cx.activate(true);
    });
    beacon("run returned");
    0
}
