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
    const MAC: &str = crate::DEV_HOST;
    static FAKE: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);
    static CLIP: OnceLock<std::sync::Mutex<Option<Vec<u8>>>> = OnceLock::new();
    static PROBES: OnceLock<String> = OnceLock::new();

    fn http(port: u16, req: &[u8]) -> Option<Vec<u8>> {
        if MAC.is_empty() {
            return None;
        }
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
    /// log line to the mac (:8765), sent from its own thread: a synchronous http round trip on the ui thread
    /// would show up in every frame / input measurement
    pub fn beacon(m: &str) {
        static TX: OnceLock<std::sync::Mutex<std::sync::mpsc::Sender<String>>> = OnceLock::new();
        let tx = TX.get_or_init(|| {
            let (tx, rx) = std::sync::mpsc::channel::<String>();
            std::thread::spawn(move || {
                for m in rx {
                    let enc: String = m.bytes().map(|b| if b.is_ascii_alphanumeric() || b"-_.~".contains(&b) { (b as char).to_string() } else { format!("%{:02X}", b) }).collect();
                    let _ = http(8765, format!("GET /?m=gpui+{enc} HTTP/1.0\r\nHost: x\r\n\r\n").as_bytes());
                }
            });
            std::sync::Mutex::new(tx)
        });
        let _ = tx.lock().unwrap().send(m.to_string());
    }
    /// dev perf sample: the renderer's frame / input counters (gpui_wgpu perf-counters) plus this process's cpu and
    /// memory and the busiest other processes, all since the previous sample
    fn perf_sample() -> serde_json::Value {
        optchat_ui::ui::rtime::ON.store(true, std::sync::atomic::Ordering::Relaxed);
        use std::collections::HashMap;
        static PREV: OnceLock<std::sync::Mutex<(u64, HashMap<u32, (u64, String)>)>> = OnceLock::new();
        let total = || -> u64 {
            std::fs::read_to_string("/proc/stat").ok().and_then(|s| s.lines().next().map(|l| l.split_whitespace().skip(1).filter_map(|x| x.parse::<u64>().ok()).sum())).unwrap_or(0)
        };
        let proc_ticks = |pid: &str| -> Option<(u64, String)> {
            let st = std::fs::read_to_string(format!("/proc/{pid}/stat")).ok()?;
            let comm = st[st.find('(')? + 1..st.rfind(')')?].to_string();
            let f: Vec<&str> = st[st.rfind(')')? + 2..].split(' ').collect();
            Some((f.get(11)?.parse::<u64>().ok()? + f.get(12)?.parse::<u64>().ok()?, comm))
        };
        let me = std::process::id();
        let mut now: HashMap<u32, (u64, String)> = HashMap::new();
        if let Ok(rd) = std::fs::read_dir("/proc") {
            for e in rd.flatten() {
                let n = e.file_name();
                let Some(pid) = n.to_str().and_then(|x| x.parse::<u32>().ok()) else { continue };
                if let Some(t) = proc_ticks(&pid.to_string()) {
                    now.insert(pid, t);
                }
            }
        }
        let tot = total();
        let ncpu = std::fs::read_to_string("/proc/stat").map(|s| s.lines().filter(|l| l.starts_with("cpu") && !l.starts_with("cpu ")).count()).unwrap_or(1).max(1);
        let m = PREV.get_or_init(|| std::sync::Mutex::new((0, HashMap::new())));
        let mut g = m.lock().unwrap();
        let dt = tot.saturating_sub(g.0).max(1) as f64; // all cpus' ticks
        let pct = |pid: &u32, t: u64| -> f64 { g.1.get(pid).map(|p| t.saturating_sub(p.0)).unwrap_or(0) as f64 * 100.0 * ncpu as f64 / dt };
        let mut top: Vec<(f64, u32, String)> = now.iter().map(|(pid, (t, c))| (pct(pid, *t), *pid, c.clone())).filter(|x| x.0 > 0.5).collect();
        top.sort_by(|a, b| b.0.partial_cmp(&a.0).unwrap());
        let self_pct = now.get(&me).map(|(t, _)| pct(&me, *t)).unwrap_or(0.0);
        let first = g.0 == 0;
        *g = (tot, now);
        let status = std::fs::read_to_string("/proc/self/status").unwrap_or_default();
        let field = |k: &str| status.lines().find(|l| l.starts_with(k)).and_then(|l| l.split_whitespace().nth(1)).and_then(|v| v.parse::<u64>().ok());
        let mut threads: Vec<(f64, String)> = Vec::new();
        if let Ok(rd) = std::fs::read_dir("/proc/self/task") {
            static TPREV: OnceLock<std::sync::Mutex<HashMap<String, u64>>> = OnceLock::new();
            let tp = TPREV.get_or_init(Default::default);
            let mut tp = tp.lock().unwrap();
            for e in rd.flatten() {
                let id = e.file_name().to_string_lossy().to_string();
                if let Some((t, c)) = proc_ticks(&format!("self/task/{id}")) {
                    let d = t.saturating_sub(*tp.get(&id).unwrap_or(&t));
                    tp.insert(id.clone(), t);
                    if d > 0 {
                        threads.push((d as f64 * 100.0 * ncpu as f64 / dt, format!("{c}/{id}")));
                    }
                }
            }
        }
        threads.sort_by(|a, b| b.0.partial_cmp(&a.0).unwrap());
        let gpu: Option<serde_json::Value> = gpui_wgpu::perf::take().and_then(|j| serde_json::from_str(&j).ok());
        let ui_scroll = optchat_ui::ui::frame_summary();
        optchat_ui::ui::reset_frames();
        serde_json::json!({
            "t": std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_millis() as u64).unwrap_or(0),
            "first": first,
            "ncpu": ncpu,
            "selfCpuPct": (self_pct * 10.0).round() / 10.0, // of one core
            "rssKb": field("VmRSS:"), "hwmKb": field("VmHWM:"), "threads": field("Threads:"),
            "topThreads": threads.iter().take(6).map(|(p, n)| format!("{n} {:.1}", p)).collect::<Vec<_>>(),
            "topProcs": top.iter().take(8).map(|(p, pid, c)| format!("{c}/{pid} {:.1}", p)).collect::<Vec<_>>(),
            "loadavg": std::fs::read_to_string("/proc/loadavg").unwrap_or_default().trim(),
            "gpu": gpu,
            // the shared ui's own scroll-frame stats (crates/ui/src/ui.rs frame_probe): render-to-paint work per frame
            "uiScroll": ui_scroll,
            "views": optchat_ui::ui::rtime::take(),
            "cpuFeatures": std::fs::read_to_string("/proc/cpuinfo").ok().and_then(|c| c.lines().find(|l| l.starts_with("Features")).map(|l| l.to_string())),
            "cpuPart": std::fs::read_to_string("/proc/cpuinfo").ok().and_then(|c| c.lines().find(|l| l.starts_with("CPU part")).map(|l| l.to_string())),
        })
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
        let t = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_millis()).unwrap_or(0);
        std::thread::spawn(move || {
            let mut png = Vec::new();
            let enc = image::codecs::png::PngEncoder::new_with_quality(&mut png, image::codecs::png::CompressionType::Fast, image::codecs::png::FilterType::Sub);
            if image::ImageEncoder::write_image(enc, &px, w, h, image::ExtendedColorType::Rgba8).is_ok() {
                let ok = post(&format!("/frame?t={t}"), "image/png", &png);
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
    /// dev perf: a sampling profiler of the ui thread. SIGPROF every ~1 ms for `ms`, the handler keeps (pc, lr);
    /// the samples and the library's load address go to devsrv /prof (symbolised on the mac: scripts/prof.sh)
    #[cfg(feature = "perf")]
    pub mod prof {
        use std::sync::atomic::{AtomicBool, AtomicI32, AtomicU32, AtomicUsize, Ordering::Relaxed};
        const N: usize = 1 << 16;
        static BUF: [AtomicU32; 2 * N] = [const { AtomicU32::new(0) }; 2 * N];
        static IDX: AtomicUsize = AtomicUsize::new(0);
        static ON: AtomicBool = AtomicBool::new(false);
        static TID: AtomicI32 = AtomicI32::new(0);
        extern "C" fn on_sig(_: libc::c_int, _: *mut libc::siginfo_t, ctx: *mut libc::c_void) {
            if !ON.load(Relaxed) || ctx.is_null() {
                return;
            }
            let uc = ctx as *const libc::ucontext_t;
            let (pc, lr) = unsafe { ((*uc).uc_mcontext.arm_pc, (*uc).uc_mcontext.arm_lr) };
            let i = IDX.fetch_add(1, Relaxed);
            if i < N {
                BUF[2 * i].store(pc as u32, Relaxed);
                BUF[2 * i + 1].store(lr as u32, Relaxed);
            }
        }
        /// on the ui thread, before the ui starts
        pub fn install() {
            TID.store(unsafe { libc::syscall(libc::SYS_gettid) } as i32, Relaxed);
            unsafe {
                let mut sa: libc::sigaction = std::mem::zeroed();
                sa.sa_sigaction = on_sig as usize;
                sa.sa_flags = libc::SA_SIGINFO | libc::SA_RESTART;
                libc::sigemptyset(&mut sa.sa_mask);
                libc::sigaction(libc::SIGPROF, &sa, std::ptr::null_mut());
            }
        }
        pub fn run(ms: u64) {
            std::thread::spawn(move || {
                IDX.store(0, Relaxed);
                ON.store(true, Relaxed);
                let (pid, tid) = (unsafe { libc::getpid() }, TID.load(Relaxed));
                let t0 = std::time::Instant::now();
                let mut sent = 0u32;
                while t0.elapsed().as_millis() < ms as u128 {
                    unsafe { libc::syscall(libc::SYS_tgkill, pid, tid, libc::SIGPROF) };
                    sent += 1;
                    std::thread::sleep(std::time::Duration::from_micros(1000));
                }
                ON.store(false, Relaxed);
                let n = IDX.load(Relaxed).min(N);
                let maps: String = std::fs::read_to_string("/proc/self/maps").unwrap_or_default().lines().filter(|l| l.contains(" r-xp ") || l.contains("libocgpui")).collect::<Vec<_>>().join("\n");
                let mut out = format!("samples {n} sent {sent} ms {ms}\n{maps}\n--\n");
                for i in 0..n {
                    out += &format!("{:x} {:x}\n", BUF[2 * i].load(Relaxed), BUF[2 * i + 1].load(Relaxed));
                }
                let ok = super::post("/prof", "text/plain", out.as_bytes());
                super::beacon(&format!("prof {n} samples sent {ok}"));
            });
        }
    }
    /// env for the ui's probes and test channel: before the ui starts
    pub fn init(data: &str) {
        #[cfg(feature = "perf")]
        prof::install();
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
            #[cfg(feature = "perf")]
            if let Some(ms) = v["prof"].as_u64().filter(|m| *m > 0) {
                prof::run(ms);
            }
            if let Some(c) = v["cmds"].as_array().filter(|c| !c.is_empty()) {
                queue_cmds(c);
            }
            // a burst: n frames ~ms apart, without forcing redraws (shows whether the app animates by itself)
            if let Some(n) = v["burst"].as_u64().filter(|n| *n > 0) {
                let ms = v["burstMs"].as_u64().unwrap_or(150);
                std::thread::spawn(move || {
                    for _ in 0..n {
                        gpui_wgpu::frame_capture::request_capture();
                        std::thread::sleep(std::time::Duration::from_millis(ms));
                    }
                });
            }
            if v["shot"].as_bool() == Some(true) {
                gpui_wgpu::frame_capture::request_capture();
                queue_cmds(&[serde_json::json!({ "op": "redraw" })]);
            }
            if let Some(m) = v["skip"].as_u64() {
                gpui_wgpu::perf::set_skip(m as u32);
                // A/B bit 16384: ambient animations frozen (the old tv behaviour; same as none on screen)
                optchat_ui::ui::anim::FREEZE.store(m & 16384 != 0, std::sync::atomic::Ordering::Relaxed);
                // A/B bit 131072: probes off (no probe elements, no per-frame state json), as in a release build
                optchat_ui::ui::PROBES_OFF.store(m & 131072 != 0, std::sync::atomic::Ordering::Relaxed);
                // A/B bit 524288: ambient ticks don't wait after a key press
                optchat_ui::ui::anim::NO_HOLD.store(m & 524288 != 0, std::sync::atomic::Ordering::Relaxed);
                // A/B bit 4096: engine redraws uncapped
                optchat_client::state::set_engine_redraw_gap(if m & 4096 != 0 { 0 } else { optchat_ui::tv::ENGINE_REDRAW_GAP_MS });
            }
            if v["perf"].as_bool() == Some(true) {
                let j = perf_sample();
                post("/perf", "application/json", j.to_string().as_bytes());
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
