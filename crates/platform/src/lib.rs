//! The platform hooks: everything OS-specific the shared crates (optchat-client, optchat-ui) need goes through the
//! [`Platform`] trait. Each shell (shells/<os>) implements it and calls [`install`] once at launch, before GPUI runs.
//! Shared crates never use an OS API directly; a new platform is a new shell, not a change here.
//!
//! Every method has a default, so a new shell starts with nothing but `impl Platform for MyOs {}`:
//! - a secure token store (Keychain / Credential Manager / Secret Service); none → tokens can't be saved
//! - resident memory for the perf probes; none → 0
//! - QR scanning (a camera); none → no camera button next to the engine token field
//! - bonjour browsing and network path changes (lan-first); none → the tunnel only
use gpui::{App, Window};
use std::sync::{Arc, OnceLock};

pub type ScanDone = Box<dyn FnOnce(Option<String>, &mut Window, &mut App)>;

/// Where client tokens live (engine endpoint + workspace → token). Never logged, never in prefs.
pub trait SecureStore: Send + Sync {
    fn get(&self, key: &str) -> Option<String>;
    fn set(&self, key: &str, token: &str) -> Result<(), String>;
    fn delete(&self, key: &str);
}

pub trait Platform: Send + Sync + 'static {
    /// the OS's secure store for client tokens
    fn secure_store(&self) -> Option<Arc<dyn SecureStore>> {
        None
    }
    /// this process's resident memory in MB (perf probes)
    fn resident_mb(&self) -> Option<f64> {
        None
    }
    /// whether a QR scanner is available (a camera)
    fn can_scan_qr(&self) -> bool {
        false
    }
    /// scan one QR code; call `done` on the main thread with its text (None: cancelled / failed)
    fn scan_qr(&self, window: &mut Window, cx: &mut App, done: ScanDone) {
        done(None, window, cx)
    }
    /// browse bonjour for `service` (e.g. `_optchat-lan._tcp`); events come on any thread. The returned handle stops
    /// the browse when dropped. None: no bonjour here.
    fn browse(&self, _service: &str, _on: Arc<dyn Fn(BonjourEvent) + Send + Sync>) -> Option<Box<dyn Send + Sync>> {
        None
    }
    /// watch the network path (wifi up/down, interface changes); `on` is called at once and on every change.
    /// false: no path monitor here.
    fn watch_net(&self, _on: Arc<dyn Fn(NetPath) + Send + Sync>) -> bool {
        false
    }
    /// a diagnostic line (outcomes and timings only, never tokens)
    fn log(&self, line: &str) {
        eprintln!("{line}");
    }
}

/// A resolved bonjour service: instance name, target host, port, its IPv4 addresses and TXT record.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Bonjour {
    pub name: String,
    pub host: String,
    pub port: u16,
    pub addrs: Vec<String>,
    pub txt: Vec<(String, String)>,
}
#[derive(Clone, Debug, PartialEq)]
pub enum BonjourEvent {
    Found(Bonjour),
    Lost(String),
}
/// The current network path (NWPathMonitor): usable at all, and over which interface kinds.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct NetPath {
    pub up: bool,
    pub wifi: bool,
    pub wired: bool,
    pub cellular: bool,
    pub expensive: bool,
}
impl NetPath {
    /// a local network a lan service can be on (wifi or ethernet)
    pub fn local(&self) -> bool {
        self.up && (self.wifi || self.wired)
    }
}

static PLATFORM: OnceLock<Arc<dyn Platform>> = OnceLock::new();

/// the shell installs its platform once at launch (later calls are ignored)
pub fn install(p: Arc<dyn Platform>) {
    let _ = PLATFORM.set(p);
}
/// the installed platform, if the shell installed one
pub fn get() -> Option<&'static Arc<dyn Platform>> {
    PLATFORM.get()
}
pub fn secure_store() -> Option<Arc<dyn SecureStore>> {
    get().and_then(|p| p.secure_store())
}
pub fn resident_mb() -> f64 {
    get().and_then(|p| p.resident_mb()).unwrap_or(0.0)
}
pub fn can_scan_qr(_cx: &App) -> bool {
    get().is_some_and(|p| p.can_scan_qr())
}
pub fn browse(service: &str, on: Arc<dyn Fn(BonjourEvent) + Send + Sync>) -> Option<Box<dyn Send + Sync>> {
    get().and_then(|p| p.browse(service, on))
}
pub fn watch_net(on: Arc<dyn Fn(NetPath) + Send + Sync>) -> bool {
    get().is_some_and(|p| p.watch_net(on))
}
pub fn log(line: &str) {
    match get() {
        Some(p) => p.log(line),
        None => eprintln!("{line}"),
    }
}
pub fn scan_qr(window: &mut Window, cx: &mut App, done: ScanDone) {
    match get() {
        Some(p) => p.scan_qr(window, cx, done),
        None => done(None, window, cx),
    }
}

/// The Keychain (macOS and iOS): generic passwords, service `service`, account = the key. Shells on Apple platforms
/// enable the `apple-keychain` feature to use it; the item format is the one the apps have always used.
#[cfg(all(feature = "apple-keychain", any(target_os = "macos", target_os = "ios")))]
pub struct Keychain {
    pub service: &'static str,
}
#[cfg(all(feature = "apple-keychain", any(target_os = "macos", target_os = "ios")))]
impl SecureStore for Keychain {
    fn get(&self, key: &str) -> Option<String> {
        security_framework::passwords::get_generic_password(self.service, key).ok().and_then(|b| String::from_utf8(b).ok())
    }
    fn set(&self, key: &str, token: &str) -> Result<(), String> {
        security_framework::passwords::set_generic_password(self.service, key, token.as_bytes()).map_err(|e| format!("Keychain: {e}"))
    }
    fn delete(&self, key: &str) {
        let _ = security_framework::passwords::delete_generic_password(self.service, key);
    }
}

#[cfg(all(feature = "apple-net", any(target_os = "macos", target_os = "ios")))]
pub mod apple_net;
