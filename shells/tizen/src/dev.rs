//! dev-only (feature dev-capture, `build.sh --dev-capture`; never in a normal build): a channel to the mac dev server
//! (optchat-scratch/tizen/scripts/devsrv.py on :8767) for tests without touching the tv:
//! - screenshots: the renderer reads back a frame (gpui_wgpu frame-capture), we png it and POST /frame
//! - the ui's own test channel (crates/ui/src/testcmd.rs, OPTCHAT_TEST_HOOKS): commands from GET /poll go to
//!   `<probes>.cmd`; the app state (`<probes>.state.json`) goes to POST /state on request
//! - a recorded clip as the microphone (GET /clip, wav), to prove the dictation path without a voice
//! - log lines (beacon) to the mac log server (:8765)
#[cfg(feature = "dev-capture")]
mod imp {
    use std::io::{Read, Write};
    use std::sync::OnceLock;
    const MAC: &str = "192.0.2.1";
    static FAKE: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);
    static CLIP: OnceLock<std::sync::Mutex<Option<Vec<u8>>>> = OnceLock::new();
    static PROBES: OnceLock<String> = OnceLock::new();

    fn http(port: u16, req: &[u8]) -> Option<Vec<u8>> {
        let addr: std::net::SocketAddr = format!("{MAC}:{port}").parse().ok()?;
        let mut s = std::net::TcpStream::connect_timeout(&addr, std::time::Duration::from_secs(2)).ok()?;
        s.set_read_timeout(Some(std::time::Duration::from_secs(5))).ok()?;
        s.write_all(req).ok()?;
        let mut out = Vec::new();
        let _ = s.read_to_end(&mut out);
        let i = out.windows(4).position(|w| w == b"\r\n\r\n")?;
        Some(out[i + 4..].to_vec())
    }
    fn post(path: &str, mt: &str, body: &[u8]) -> bool {
        let mut req = format!("POST {path} HTTP/1.0\r\nHost: x\r\nContent-Type: {mt}\r\nContent-Length: {}\r\n\r\n", body.len()).into_bytes();
        req.extend_from_slice(body);
        http(8767, &req).is_some()
    }
    pub fn beacon(m: &str) {
        let enc: String = m.bytes().map(|b| if b.is_ascii_alphanumeric() || b"-_.~".contains(&b) { (b as char).to_string() } else { format!("%{:02X}", b) }).collect();
        let _ = http(8765, format!("GET /?m=gpui+{enc} HTTP/1.0\r\nHost: x\r\n\r\n").as_bytes());
    }
    pub fn fake_clip() -> Option<Vec<u8>> {
        if !FAKE.load(std::sync::atomic::Ordering::SeqCst) {
            return None;
        }
        let m = CLIP.get_or_init(|| std::sync::Mutex::new(None));
        let mut g = m.lock().unwrap();
        if g.is_none() {
            let wav = http(8767, b"GET /clip HTTP/1.0\r\nHost: x\r\n\r\n")?;
            *g = Some(optchat_client::media::wav_pcm(&wav));
        }
        g.clone()
    }
    fn sink(w: u32, h: u32, px: Vec<u8>) {
        std::thread::spawn(move || {
            let mut png = Vec::new();
            let enc = image::codecs::png::PngEncoder::new_with_quality(&mut png, image::codecs::png::CompressionType::Fast, image::codecs::png::FilterType::Sub);
            if image::ImageEncoder::write_image(enc, &px, w, h, image::ExtendedColorType::Rgba8).is_ok() {
                let ok = post("/frame", "image/png", &png);
                beacon(&format!("cap {w}x{h} png {} sent {ok}", png.len()));
            }
        });
    }
    fn queue_cmds(cmds: &[serde_json::Value]) {
        let Some(p) = PROBES.get() else { return };
        let path = format!("{p}.cmd");
        let mut all: Vec<serde_json::Value> = std::fs::read_to_string(&path).ok().and_then(|s| serde_json::from_str(&s).ok()).unwrap_or_default();
        all.extend_from_slice(cmds);
        let _ = std::fs::write(&path, serde_json::to_string(&all).unwrap_or_default());
    }
    /// env for the ui's probes and test channel: before the ui starts
    pub fn init(data: &str) {
        let p = format!("{data}/probes.json");
        unsafe {
            std::env::set_var("OPTCHAT_PROBES", &p);
            std::env::set_var("OPTCHAT_TEST_HOOKS", "1");
        }
        let _ = PROBES.set(p);
        gpui_wgpu::frame_capture::set_capture_sink(sink);
    }
    /// the poll loop (its own thread)
    pub fn start() {
        std::thread::spawn(|| loop {
            std::thread::sleep(std::time::Duration::from_millis(500));
            let Some(body) = http(8767, b"GET /poll HTTP/1.0\r\nHost: x\r\n\r\n") else { continue };
            let Ok(v) = serde_json::from_slice::<serde_json::Value>(&body) else { continue };
            if let Some(f) = v["fakemic"].as_bool() {
                FAKE.store(f, std::sync::atomic::Ordering::SeqCst);
            }
            if let Some(c) = v["cmds"].as_array().filter(|c| !c.is_empty()) {
                queue_cmds(c);
            }
            if v["shot"].as_bool() == Some(true) {
                gpui_wgpu::frame_capture::request_capture();
                queue_cmds(&[serde_json::json!({ "op": "redraw" })]);
            }
            if v["state"].as_bool() == Some(true) {
                std::thread::sleep(std::time::Duration::from_millis(300));
                let st = PROBES.get().and_then(|p| std::fs::read(format!("{p}.state.json")).ok()).unwrap_or_default();
                post("/state", "application/json", &st);
            }
        });
    }
}
#[cfg(feature = "dev-capture")]
pub use imp::*;

#[cfg(not(feature = "dev-capture"))]
mod imp {
    pub fn beacon(_: &str) {}
    pub fn fake_clip() -> Option<Vec<u8>> {
        None
    }
    pub fn init(_: &str) {}
    pub fn start() {}
}
#[cfg(not(feature = "dev-capture"))]
pub use imp::*;
