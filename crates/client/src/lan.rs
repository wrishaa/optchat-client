//! Lan-first (poc, native-lan/PROTOCOL.md v1): on the same network as the engine's Mac, the engine socket goes
//! straight to a lan front (`_optchat-lan._tcp`, pinned TLS, short-lived tickets) instead of the tunnel; the tunnel
//! stays the baseline and the fallback.
//!
//! - discovery: the platform's bonjour browse + network path monitor (optchat-platform) feed one [`World`];
//! - trust: the lan front's self-signed leaf is accepted only if the SHA-256 of its SubjectPublicKeyInfo equals the
//!   advertised pin, checked right after the handshake and before a single byte is written (native-tls: the system's
//!   TLS, so no second TLS stack and no export-compliance change). The tunnel path keeps normal verification;
//! - auth: the engine token goes once to `POST /lan/ticket` over the pinned connection; the socket, blobs and
//!   workspace lists use the ticket (`Authorization: Bearer olt1.…`);
//! - switching: [`Sm`] decides which path the socket should be on; `client::EngineConn` moves it make-before-break.
use base64::Engine as _;
use bytes::Bytes;
use http_body_util::{BodyExt, Empty};
use optchat_platform::{BonjourEvent, NetPath};
use parking_lot::Mutex;
use sha2::{Digest, Sha256};
use std::sync::{Arc, OnceLock};
use std::time::{Duration, Instant};
use tokio::net::TcpStream;
use tokio::sync::watch;
use tokio_native_tls::TlsStream;

pub const SERVICE: &str = "_optchat-lan._tcp";
/// the fixed name in the lan leaf's SANs (SNI and Host when dialing an address)
pub const SNI: &str = "optchat-lan";

pub fn log(line: &str) {
    optchat_platform::log(&format!("lan: {line}"));
}

// ---- pins ------------------------------------------------------------------------------------------------------------
/// one DER TLV at `b[i..]`: (header length, content length)
fn tlv(b: &[u8], i: usize) -> Option<(usize, usize)> {
    let l0 = *b.get(i + 1)? as usize;
    if l0 < 0x80 {
        return Some((2, l0));
    }
    let n = l0 & 0x7f;
    if n == 0 || n > 4 {
        return None;
    }
    let mut len = 0usize;
    for k in 0..n {
        len = (len << 8) | *b.get(i + 2 + k)? as usize;
    }
    Some((2 + n, len))
}
/// The SubjectPublicKeyInfo of an X.509 certificate (DER), tag and length included: Certificate → tbsCertificate →
/// [version] serial signature issuer validity subject **spki**.
pub fn spki_of(cert: &[u8]) -> Option<&[u8]> {
    let (h, _) = tlv(cert, 0).filter(|_| cert.first() == Some(&0x30))?;
    let tbs = h;
    let (th, _) = tlv(cert, tbs).filter(|_| cert.get(tbs) == Some(&0x30))?;
    let mut i = tbs + th;
    if cert.get(i) == Some(&0xa0) {
        let (vh, vl) = tlv(cert, i)?;
        i += vh + vl;
    }
    // serial, signature, issuer, validity, subject
    for _ in 0..5 {
        let (fh, fl) = tlv(cert, i)?;
        i += fh + fl;
    }
    if cert.get(i) != Some(&0x30) {
        return None;
    }
    let (sh, sl) = tlv(cert, i)?;
    cert.get(i..i + sh + sl)
}
/// `sha256/<base64 of sha256(SPKI DER)>`, the form the lan front advertises (and curl's --pinnedpubkey takes)
pub fn pin_of(cert: &[u8]) -> Option<String> {
    Some(format!("sha256/{}", base64::engine::general_purpose::STANDARD.encode(Sha256::digest(spki_of(cert)?))))
}
/// whether a certificate matches a pin (`sha256/<b64>` or bare `<b64>`), in constant time
pub fn pin_ok(cert: &[u8], pin: &str) -> bool {
    let want = pin.trim().trim_start_matches("sha256/").trim_start_matches('/');
    let Ok(want) = base64::engine::general_purpose::STANDARD.decode(want) else { return false };
    let Some(spki) = spki_of(cert) else { return false };
    let got = Sha256::digest(spki);
    want.len() == got.len() && want.iter().zip(got.iter()).fold(0u8, |a, (x, y)| a | (x ^ y)) == 0
}

// ---- services --------------------------------------------------------------------------------------------------------
/// A lan front found by bonjour (TXT `v=1`, `engine`, `pin`, …).
#[derive(Clone, Debug, PartialEq, Default)]
pub struct LanService {
    pub name: String,
    pub engine: String,
    pub host: String,
    pub port: u16,
    pub addrs: Vec<String>,
    pub pin: String,
}
impl LanService {
    pub fn from_bonjour(b: &optchat_platform::Bonjour) -> Option<LanService> {
        let get = |k: &str| b.txt.iter().find(|(x, _)| x == k).map(|(_, v)| v.clone());
        // an unknown major version is ignored (PROTOCOL §7)
        if get("v")?.split('.').next()? != "1" {
            return None;
        }
        let pin = get("pin").filter(|p| !p.is_empty())?;
        let engine = get("engine")?;
        Some(LanService { name: b.name.clone(), engine, host: b.host.clone(), port: b.port, addrs: b.addrs.clone(), pin })
    }
    /// whether this front serves the engine of a token whose urls are `urls` (TXT `engine` = the host of one of them)
    pub fn serves(&self, urls: &[String]) -> bool {
        let e = self.engine.to_ascii_lowercase();
        !e.is_empty()
            && urls.iter().any(|u| {
                let auth = u.split("://").nth(1).unwrap_or(u).split('/').next().unwrap_or("").to_ascii_lowercase();
                auth == e || auth.split(':').next() == Some(e.as_str())
            })
    }
    /// where to dial: the resolved addresses, then the host name
    pub fn targets(&self) -> Vec<String> {
        let mut t: Vec<String> = self.addrs.iter().map(|a| format!("{a}:{}", self.port)).collect();
        if !self.host.is_empty() {
            t.push(format!("{}:{}", self.host, self.port));
        }
        t
    }
}

// ---- the world: services seen + the network path ---------------------------------------------------------------------
#[derive(Clone, Debug, Default, PartialEq)]
pub struct World {
    pub services: Vec<LanService>,
    pub net: NetPath,
    /// bumped on every network change (re-probe now)
    pub net_epoch: u64,
}

struct Disco {
    tx: watch::Sender<World>,
    browse: Mutex<Option<Box<dyn Send + Sync>>>,
}
static DISCO: OnceLock<Option<Arc<Disco>>> = OnceLock::new();

fn on_bonjour(d: &Disco, e: BonjourEvent) {
    d.tx.send_modify(|w| match e {
        BonjourEvent::Found(b) => match LanService::from_bonjour(&b) {
            Some(s) => {
                w.services.retain(|x| x.name != s.name);
                log(&format!("found {:?} {} at {:?}:{}", s.name, s.engine, s.addrs, s.port));
                w.services.push(s);
            }
            None => log(&format!("ignored {:?} (no v=1 / pin / engine)", b.name)),
        },
        BonjourEvent::Lost(n) => {
            if w.services.iter().any(|x| x.name == n) {
                log(&format!("lost {n:?}"));
            }
            w.services.retain(|x| x.name != n);
        }
    });
}
fn start_browse(d: &Arc<Disco>) {
    let d2 = d.clone();
    // the old browse stops first (its cache goes with it); a fresh one queries the new network
    *d.browse.lock() = None;
    *d.browse.lock() = optchat_platform::browse(SERVICE, Arc::new(move |e| on_bonjour(&d2, e)));
}

/// The lan world (services + network path), started on first use: the platform's bonjour browse (on iOS this shows
/// the Local Network prompt once) and its path monitor. None: this platform has neither (tunnel only), or
/// OPTCHAT_LAN=0.
pub fn world() -> Option<watch::Receiver<World>> {
    DISCO
        .get_or_init(|| {
            if std::env::var("OPTCHAT_LAN").is_ok_and(|v| v == "0") {
                return None;
            }
            let (tx, _) = watch::channel(World { net: NetPath { up: true, wifi: true, ..Default::default() }, ..Default::default() });
            let d = Arc::new(Disco { tx, browse: Mutex::new(None) });
            start_browse(&d);
            if d.browse.lock().is_none() {
                return None;
            }
            let d2 = d.clone();
            let watching = optchat_platform::watch_net(Arc::new(move |p: NetPath| {
                let mut changed = false;
                let mut rejoined = false;
                d2.tx.send_modify(|w| {
                    if w.net != p {
                        changed = true;
                        rejoined = p.local() && !w.net.local();
                        log(&format!("net: {}{}{}{}", if p.up { "up" } else { "down" }, if p.wifi { " wifi" } else { "" }, if p.wired { " wired" } else { "" }, if p.cellular { " cellular" } else { "" }));
                        w.net = p;
                        w.net_epoch += 1;
                        if !p.local() {
                            // off wifi: nothing found there is reachable any more
                            w.services.clear();
                        }
                    }
                });
                if changed && (rejoined || p.local()) {
                    // joined (or changed) a local network: browse again from scratch
                    start_browse(&d2);
                }
            }));
            if !watching {
                log("no path monitor: lan only while bonjour sees the front");
            }
            Some(d)
        })
        .as_ref()
        .map(|d| d.tx.subscribe())
}

// ---- tickets ---------------------------------------------------------------------------------------------------------
#[derive(Clone, Debug)]
pub struct Ticket {
    pub value: String,
    pub minted: Instant,
    pub ttl: Duration,
}
impl Ticket {
    /// still good to open something with (a margin for clock skew and the request itself)
    pub fn usable(&self, now: Instant) -> bool {
        now + Duration::from_secs(30) < self.minted + self.ttl
    }
    /// time to renew (PROTOCOL §3: at ~ttl/2)
    pub fn renew_due(&self, now: Instant) -> bool {
        now >= self.minted + self.ttl / 2
    }
    pub fn parse(body: &[u8], now: Instant) -> Result<Ticket, String> {
        let v: serde_json::Value = serde_json::from_slice(body).map_err(|_| "a ticket answer that is not JSON".to_string())?;
        let value = v["ticket"].as_str().filter(|t| t.starts_with("olt1.")).ok_or("no ticket in the answer")?.to_string();
        let ttl = v["ttl"].as_u64().unwrap_or(600).clamp(30, 3600);
        Ok(Ticket { value, minted: now, ttl: Duration::from_secs(ttl) })
    }
}

// ---- pinned connections ----------------------------------------------------------------------------------------------
/// TCP + TLS to the lan front; the peer's leaf must match `pin` before anything is sent. Tries each target in turn.
pub async fn dial(svc: &LanService, timeout: Duration) -> Result<TlsStream<TcpStream>, String> {
    let mut last = String::from("no address");
    for t in svc.targets() {
        match tokio::time::timeout(timeout, dial_one(&t, &svc.pin)).await {
            Ok(Ok(s)) => return Ok(s),
            Ok(Err(e)) => last = format!("{t}: {e}"),
            Err(_) => last = format!("{t}: timed out"),
        }
        if last.contains("pin") {
            break; // a wrong key is not a reachability problem: don't try the next address
        }
    }
    Err(last)
}
async fn dial_one(target: &str, pin: &str) -> Result<TlsStream<TcpStream>, String> {
    let tcp = TcpStream::connect(target).await.map_err(|e| e.to_string())?;
    let _ = tcp.set_nodelay(true);
    // validation off on purpose: the pin below is the trust decision (the system's chain + hostname checks can't
    // know a per-Mac self-signed CA); nothing is written before it passes
    let c = native_tls::TlsConnector::builder()
        .danger_accept_invalid_certs(true)
        .danger_accept_invalid_hostnames(true)
        .min_protocol_version(Some(native_tls::Protocol::Tlsv12))
        .build()
        .map_err(|e| e.to_string())?;
    let tls = tokio_native_tls::TlsConnector::from(c).connect(SNI, tcp).await.map_err(|e| e.to_string())?;
    let cert = tls.get_ref().peer_certificate().map_err(|e| e.to_string())?.ok_or("no peer certificate")?;
    let der = cert.to_der().map_err(|e| e.to_string())?;
    if !pin_ok(&der, pin) {
        return Err(format!("pin mismatch (got {})", pin_of(&der).unwrap_or_default()));
    }
    Ok(tls)
}

/// A response from the lan front.
pub struct Resp {
    pub status: u16,
    pub body: Bytes,
    pub content_type: String,
    /// the front's own error (header x-optchat-lan) and its code
    pub lan_code: Option<String>,
}

/// HTTP/1.1 over pinned TLS with one kept-alive connection (hyper's client connection; reqwest has no hook between
/// the handshake and the first byte).
pub struct LanHttp {
    pub svc: LanService,
    conn: Mutex<Option<hyper::client::conn::http1::SendRequest<Empty<Bytes>>>>,
}
impl LanHttp {
    pub fn new(svc: LanService) -> LanHttp {
        LanHttp { svc, conn: Mutex::new(None) }
    }
    async fn sender(&self) -> Result<hyper::client::conn::http1::SendRequest<Empty<Bytes>>, String> {
        let kept = self.conn.lock().take();
        if let Some(mut s) = kept {
            if !s.is_closed() && futures_util::future::poll_fn(|cx| s.poll_ready(cx)).await.is_ok() {
                return Ok(s);
            }
        }
        let tls = dial(&self.svc, Duration::from_millis(1500)).await?;
        let (s, conn) = hyper::client::conn::http1::handshake(hyper_util::rt::TokioIo::new(tls)).await.map_err(|e| e.to_string())?;
        tokio::spawn(async move {
            let _ = conn.await;
        });
        Ok(s)
    }
    pub async fn request(&self, method: &str, path: &str, bearer: Option<&str>) -> Result<Resp, String> {
        let mut s = self.sender().await?;
        let mut b = hyper::Request::builder().method(method).uri(path).header("host", format!("{SNI}:{}", self.svc.port));
        if method == "POST" {
            b = b.header("content-length", "0");
        }
        if let Some(t) = bearer {
            b = b.header("authorization", format!("Bearer {t}"));
        }
        let req = b.body(Empty::new()).map_err(|e| e.to_string())?;
        let r = tokio::time::timeout(Duration::from_secs(10), s.send_request(req)).await.map_err(|_| "timed out".to_string())?.map_err(|e| e.to_string())?;
        let status = r.status().as_u16();
        let content_type = r.headers().get("content-type").and_then(|v| v.to_str().ok()).unwrap_or("").to_string();
        let lan = r.headers().contains_key("x-optchat-lan");
        let body = r.into_body().collect().await.map_err(|e| e.to_string())?.to_bytes();
        *self.conn.lock() = Some(s);
        let lan_code = if lan && status >= 400 { serde_json::from_slice::<serde_json::Value>(&body).ok().and_then(|v| v["code"].as_str().map(String::from)) } else { None };
        Ok(Resp { status, body, content_type, lan_code })
    }
    /// POST /lan/ticket with the engine token (or a ticket to renew)
    pub async fn mint(&self, bearer: &str) -> Result<Ticket, String> {
        let r = self.request("POST", "/lan/ticket", Some(bearer)).await?;
        if r.status != 200 {
            return Err(format!("ticket: {} {}", r.status, r.lan_code.unwrap_or_default()));
        }
        Ticket::parse(&r.body, Instant::now())
    }
}

// ---- which path ------------------------------------------------------------------------------------------------------
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PathKind {
    Tunnel,
    Lan,
}
impl PathKind {
    pub fn name(self) -> &'static str {
        match self {
            PathKind::Tunnel => "tunnel",
            PathKind::Lan => "lan",
        }
    }
}

/// What the manager sees when it decides.
#[derive(Clone, Copy, Debug, Default)]
pub struct Env {
    /// a lan front for this engine is advertised (and its pin isn't a changed one)
    pub lan_seen: bool,
    /// on wifi / ethernet
    pub net_local: bool,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Step {
    /// open a socket on this path (make-before-break when a socket is up)
    Go(PathKind),
    Idle,
}

/// The switching rules, without I/O (unit-tested):
/// - no socket → lan if it's eligible, else the tunnel;
/// - on the tunnel → lan as soon as it's eligible (found, on wifi, not failed in the last `lan_retry`);
/// - on lan → the tunnel at once when wifi goes, lan is disabled or it failed (heartbeat miss);
/// - a lan failure backs lan off for `lan_retry` (a network change clears that); a tunnel failure backs off
///   0.4 s doubling to 4 s, but lan is tried at once when eligible.
#[derive(Clone, Debug)]
pub struct Sm {
    pub primary: Option<PathKind>,
    pub lan_disabled: bool,
    pub lan_retry: Duration,
    lan_backoff_until: Option<Instant>,
    tunnel_fails: u32,
}
impl Default for Sm {
    fn default() -> Self {
        Sm { primary: None, lan_disabled: false, lan_retry: Duration::from_secs(30), lan_backoff_until: None, tunnel_fails: 0 }
    }
}
impl Sm {
    pub fn lan_eligible(&self, now: Instant, env: Env) -> bool {
        env.lan_seen && env.net_local && !self.lan_disabled && self.lan_backoff_until.is_none_or(|t| now >= t)
    }
    pub fn step(&self, now: Instant, env: Env) -> Step {
        let lan = self.lan_eligible(now, env);
        match self.primary {
            None => Step::Go(if lan { PathKind::Lan } else { PathKind::Tunnel }),
            Some(PathKind::Tunnel) if lan => Step::Go(PathKind::Lan),
            Some(PathKind::Tunnel) => Step::Idle,
            Some(PathKind::Lan) => {
                // a working lan socket stays put when only the advert goes (mDNS caches expire); it moves when wifi
                // goes, lan is disabled, or it just failed
                let failed = self.lan_backoff_until.is_some_and(|t| now < t);
                if !env.net_local || self.lan_disabled || failed { Step::Go(PathKind::Tunnel) } else { Step::Idle }
            }
        }
    }
    pub fn adopted(&mut self, p: PathKind) {
        self.primary = Some(p);
        if p == PathKind::Tunnel {
            self.tunnel_fails = 0;
        }
    }
    /// a dial on `p` failed (or its socket died / missed a heartbeat): how long to wait before the next step
    pub fn failed(&mut self, p: PathKind, now: Instant) -> Duration {
        match p {
            PathKind::Lan => {
                self.lan_backoff_until = Some(now + self.lan_retry);
                Duration::ZERO
            }
            PathKind::Tunnel => {
                let ms = (400u64 << self.tunnel_fails.min(4)).min(4000);
                self.tunnel_fails += 1;
                Duration::from_millis(ms)
            }
        }
    }
    /// the primary socket closed
    pub fn lost(&mut self, p: PathKind, now: Instant) -> Duration {
        if self.primary == Some(p) {
            self.primary = None;
        }
        self.failed(p, now)
    }
    /// when a lan backoff ends (the manager wakes up then to try lan again)
    pub fn retry_in(&self, now: Instant) -> Option<Duration> {
        self.lan_backoff_until.filter(|t| *t > now).map(|t| t - now)
    }
    /// a network change: lan gets another chance at once
    pub fn net_changed(&mut self) {
        self.lan_backoff_until = None;
    }
}

pub fn median_p90(v: &[f64]) -> (f64, f64) {
    if v.is_empty() {
        return (0.0, 0.0);
    }
    let mut s = v.to_vec();
    s.sort_by(|a, b| a.partial_cmp(b).unwrap());
    let q = |p: f64| s[((s.len() as f64 - 1.0) * p).round() as usize];
    (q(0.5), q(0.9))
}

#[cfg(test)]
mod tests {
    use super::*;

    // a real P-256 leaf (self-signed, CA:FALSE) and its pin, made with openssl for this test
    const LEAF_B64: &str = include_str!("../testdata/lan-leaf.b64");
    const LEAF_PIN: &str = include_str!("../testdata/lan-leaf.pin");

    #[test]
    fn pin_check() {
        let der = base64::engine::general_purpose::STANDARD.decode(LEAF_B64.trim()).unwrap();
        assert_eq!(pin_of(&der).unwrap(), LEAF_PIN.trim());
        assert!(pin_ok(&der, LEAF_PIN.trim()));
        assert!(pin_ok(&der, LEAF_PIN.trim().trim_start_matches("sha256/")));
        // one bit off, wrong length, garbage, a truncated cert
        let mut bad = base64::engine::general_purpose::STANDARD.decode(LEAF_PIN.trim().trim_start_matches("sha256/")).unwrap();
        bad[0] ^= 1;
        assert!(!pin_ok(&der, &format!("sha256/{}", base64::engine::general_purpose::STANDARD.encode(&bad))));
        assert!(!pin_ok(&der, "sha256/AAAA"));
        assert!(!pin_ok(&der, "not base64 !"));
        assert!(!pin_ok(&der[..der.len() / 2], LEAF_PIN.trim()));
        assert!(spki_of(&[0x30, 0x82]).is_none());
    }

    #[test]
    fn ticket_expiry() {
        let t0 = Instant::now();
        let t = Ticket::parse(br#"{"v":1,"ticket":"olt1.x.y","exp":1,"ttl":600}"#, t0).unwrap();
        assert!(t.usable(t0) && !t.renew_due(t0));
        assert!(t.renew_due(t0 + Duration::from_secs(300)));
        assert!(t.usable(t0 + Duration::from_secs(569)));
        assert!(!t.usable(t0 + Duration::from_secs(570)));
        assert!(Ticket::parse(br#"{"ticket":"oc1.not-a-ticket"}"#, t0).is_err());
        assert!(Ticket::parse(b"nope", t0).is_err());
        // a silly ttl is clamped
        assert_eq!(Ticket::parse(br#"{"ticket":"olt1.a.b","ttl":1}"#, t0).unwrap().ttl, Duration::from_secs(30));
    }

    #[test]
    fn service_matching() {
        let b = optchat_platform::Bonjour {
            name: "OptChat LAN (mac)".into(),
            host: "mac.local".into(),
            port: 18950,
            addrs: vec!["192.168.1.7".into()],
            txt: vec![("v".into(), "1".into()), ("engine".into(), "chat.tunnel.reframe.sh".into()), ("pin".into(), "sha256/x".into())],
        };
        let s = LanService::from_bonjour(&b).unwrap();
        assert!(s.serves(&["https://chat.tunnel.reframe.sh".into(), "https://chat.reframe.dev".into()]));
        assert!(!s.serves(&["https://other.tunnel.reframe.sh".into()]));
        assert_eq!(s.targets(), vec!["192.168.1.7:18950".to_string(), "mac.local:18950".to_string()]);
        let mut v2 = b.clone();
        v2.txt[0].1 = "2".into();
        assert!(LanService::from_bonjour(&v2).is_none());
        let mut nopin = b.clone();
        nopin.txt.retain(|(k, _)| k != "pin");
        assert!(LanService::from_bonjour(&nopin).is_none());
    }

    #[test]
    fn switching() {
        let t = Instant::now();
        let on = Env { lan_seen: true, net_local: true };
        let none = Env { lan_seen: false, net_local: true };
        let off_wifi = Env { lan_seen: false, net_local: false };
        let mut sm = Sm::default();
        // start: no lan seen yet → tunnel; then lan appears → move to lan
        assert_eq!(sm.step(t, none), Step::Go(PathKind::Tunnel));
        sm.adopted(PathKind::Tunnel);
        assert_eq!(sm.step(t, none), Step::Idle);
        assert_eq!(sm.step(t, on), Step::Go(PathKind::Lan));
        sm.adopted(PathKind::Lan);
        assert_eq!(sm.step(t, on), Step::Idle);
        // the bonjour advert going away alone doesn't move a working socket
        assert_eq!(sm.step(t, Env { lan_seen: false, net_local: true }), Step::Idle);
        // wifi drops → tunnel at once
        assert_eq!(sm.step(t, off_wifi), Step::Go(PathKind::Tunnel));
        sm.adopted(PathKind::Tunnel);
        // wifi back + found again → lan
        sm.net_changed();
        assert_eq!(sm.step(t, on), Step::Go(PathKind::Lan));
        sm.adopted(PathKind::Lan);
        // heartbeat miss → backoff lan, tunnel at once; no lan retry for 30 s unless the network changes
        assert_eq!(sm.failed(PathKind::Lan, t), Duration::ZERO);
        assert_eq!(sm.step(t, on), Step::Go(PathKind::Tunnel));
        sm.adopted(PathKind::Tunnel);
        assert_eq!(sm.step(t + Duration::from_secs(10), on), Step::Idle);
        assert_eq!(sm.retry_in(t + Duration::from_secs(10)), Some(Duration::from_secs(20)));
        assert_eq!(sm.step(t + Duration::from_secs(31), on), Step::Go(PathKind::Lan));
        sm.net_changed();
        assert_eq!(sm.step(t + Duration::from_secs(1), on), Step::Go(PathKind::Lan));
        // the lan socket dies → straight to the tunnel (no wait)
        sm.adopted(PathKind::Lan);
        assert_eq!(sm.lost(PathKind::Lan, t), Duration::ZERO);
        assert_eq!(sm.step(t, on), Step::Go(PathKind::Tunnel));
        // the tunnel dies with no lan around → backoff 0.4, 0.8, … 4 s; lan showing up is taken at once
        sm.adopted(PathKind::Tunnel);
        assert_eq!(sm.lost(PathKind::Tunnel, t), Duration::from_millis(400));
        assert_eq!(sm.failed(PathKind::Tunnel, t), Duration::from_millis(800));
        for _ in 0..5 {
            sm.failed(PathKind::Tunnel, t);
        }
        assert_eq!(sm.failed(PathKind::Tunnel, t), Duration::from_millis(4000));
        sm.net_changed();
        assert_eq!(sm.step(t, on), Step::Go(PathKind::Lan));
        // disabled (tests, a user switch) → back to the tunnel
        sm.adopted(PathKind::Lan);
        sm.lan_disabled = true;
        assert_eq!(sm.step(t, on), Step::Go(PathKind::Tunnel));
    }

    #[test]
    fn medians() {
        let v: Vec<f64> = (1..=20).map(|x| x as f64).collect();
        assert_eq!(median_p90(&v), (11.0, 18.0));
    }
}
