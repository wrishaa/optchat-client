//! The tizen tv Platform: tokens in a 0600 file in the app's data dir (the tv has no keychain we can reach from a
//! dotnet app's native side), resident memory from /proc, and the remote's microphone: the c# host records with
//! Tizen.Multimedia AudioCapture while the mic key is held and hands us the pcm (oc_mic_pcm / oc_mic_end).
use optchat_platform::{MicSource, Platform, SecureStore};
use parking_lot::{Condvar, Mutex};
use std::collections::BTreeMap;
use std::sync::Arc;

pub struct FileStore {
    pub path: String,
}
impl FileStore {
    fn read(&self) -> BTreeMap<String, String> {
        std::fs::read_to_string(&self.path).ok().and_then(|s| serde_json::from_str(&s).ok()).unwrap_or_default()
    }
    fn write(&self, m: &BTreeMap<String, String>) -> Result<(), String> {
        use std::io::Write;
        use std::os::unix::fs::OpenOptionsExt;
        let mut f = std::fs::OpenOptions::new().write(true).create(true).truncate(true).mode(0o600).open(&self.path).map_err(|e| e.to_string())?;
        f.write_all(serde_json::to_string(m).unwrap_or_default().as_bytes()).map_err(|e| e.to_string())
    }
}
impl SecureStore for FileStore {
    fn get(&self, key: &str) -> Option<String> {
        self.read().get(key).cloned()
    }
    fn set(&self, key: &str, token: &str) -> Result<(), String> {
        let mut m = self.read();
        m.insert(key.into(), token.into());
        self.write(&m)
    }
    fn delete(&self, key: &str) {
        let mut m = self.read();
        m.remove(key);
        let _ = self.write(&m);
    }
}

type Hooks = (extern "C" fn(), extern "C" fn());
/// the c# host's mic start / stop, set once at launch (oc_set_mic)
pub static HOOKS: Mutex<Option<Hooks>> = Mutex::new(None);

/// the remote's microphone through the c# host. dev builds can swap in a recorded clip (crate::dev::fake_clip).
#[derive(Default)]
pub struct RemoteMic {
    sink: Mutex<Option<Arc<dyn Fn(&[u8]) + Send + Sync>>>,
    ended: Mutex<bool>,
    cv: Condvar,
    fake_stop: Arc<std::sync::atomic::AtomicBool>,
    fake: Mutex<Option<std::thread::JoinHandle<()>>>,
}
impl RemoteMic {
    /// pcm16 16 kHz mono from the host (any thread)
    pub fn pcm(&self, b: &[u8]) {
        if let Some(s) = self.sink.lock().clone() {
            s(b);
        }
    }
    /// the host delivered its last audio (the tail after the release included)
    pub fn end(&self) {
        *self.ended.lock() = true;
        self.cv.notify_all();
    }
}
impl MicSource for RemoteMic {
    fn start(&self, on_pcm: Arc<dyn Fn(&[u8]) + Send + Sync>) -> Result<(), String> {
        *self.sink.lock() = Some(on_pcm.clone());
        *self.ended.lock() = false;
        if let Some(clip) = crate::dev::fake_clip() {
            // dev: a recorded clip plays as the mic, in real time, then silence until stopped
            use std::sync::atomic::Ordering;
            self.fake_stop.store(false, Ordering::SeqCst);
            let stop = self.fake_stop.clone();
            *self.fake.lock() = Some(std::thread::spawn(move || {
                let mut k = 0;
                while !stop.load(Ordering::SeqCst) {
                    let c: Vec<u8> = if k < clip.len() { clip[k..(k + 3200).min(clip.len())].to_vec() } else { vec![0u8; 3200] };
                    k += 3200;
                    on_pcm(&c);
                    std::thread::sleep(std::time::Duration::from_millis(100));
                }
            }));
            crate::beacon(&format!("mic: fake clip {} bytes", k_len()));
            return Ok(());
        }
        let hooks = *HOOKS.lock();
        match hooks {
            Some((start, _)) => {
                start();
                Ok(())
            }
            None => Err("no microphone host".into()),
        }
    }
    fn stop(&self) {
        if let Some(t) = self.fake.lock().take() {
            self.fake_stop.store(true, std::sync::atomic::Ordering::SeqCst);
            let _ = t.join();
        } else if let Some((_, stop)) = *HOOKS.lock() {
            stop();
            // the remote keeps sending what it buffered after the release: wait for the host's end (capped)
            let mut g = self.ended.lock();
            let deadline = std::time::Instant::now() + std::time::Duration::from_secs(8);
            while !*g && std::time::Instant::now() < deadline {
                self.cv.wait_until(&mut g, deadline);
            }
        }
        *self.sink.lock() = None;
    }
}
fn k_len() -> usize {
    crate::dev::fake_clip().map(|c| c.len()).unwrap_or(0)
}

pub struct Tizen {
    pub store: Arc<FileStore>,
    pub mic: Arc<RemoteMic>,
}
impl Platform for Tizen {
    fn secure_store(&self) -> Option<Arc<dyn SecureStore>> {
        Some(self.store.clone())
    }
    fn resident_mb(&self) -> Option<f64> {
        let s = std::fs::read_to_string("/proc/self/statm").ok()?;
        let pages: f64 = s.split_whitespace().nth(1)?.parse().ok()?;
        Some(pages * 4096.0 / (1024.0 * 1024.0))
    }
    fn mic(&self) -> Option<Arc<dyn MicSource>> {
        Some(self.mic.clone())
    }
}
