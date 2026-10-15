//! The app's connections to OptChat engines (spec-workspaces §2, §4) (app/ws/serve.ts, lib/hub.ts): ONE websocket, like the browser's page.
//! A snapshot of the task on screen arrives on connect and after every `open`; then the hub's events; commands go up
//! (send / cancel / stopTool / settings / global / control / spawn / approve) and rpc calls come back by id.
//! Reconnects with backoff (0.4 s doubling to 5 s) and resnapshots. Images come from GET /blob/<sha>; each dictation
//! is a second, short-lived socket to /stt. Every URL carries `db=t<id>` on a test instance.
//! TLS: the macOS trust store (native-tls / Security.framework), verification always on.
use futures_util::{SinkExt, StreamExt};
use optchat_proto::api::*;
use optchat_proto::rates::Per1M;
use optchat_proto::types::*;
use parking_lot::Mutex;
use serde_json::{json, Value};
use crate::lan::{self, PathKind};
use std::collections::{HashMap, HashSet};
use std::sync::atomic::{AtomicBool, AtomicI64, AtomicU64, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio::sync::{mpsc, oneshot};
use tokio_tungstenite::tungstenite::client::IntoClientRequest;
use tokio_tungstenite::tungstenite::Message as WsMsg;

/// An engine: where it is (https://<host>, or http://127.0.0.1:<port> for a local relay in tests) and its name.
#[derive(Clone, Debug, Default, serde::Serialize, serde::Deserialize, PartialEq)]
pub struct EngineCfg {
    /// the address in use (for a token-added engine: the first of its urls that answered)
    pub endpoint: String,
    pub name: String,
    /// spec-workspaces §6: the addresses the engine's token carries, in order of preference
    #[serde(default)]
    pub urls: Vec<String>,
    /// added with an engine token (or the local engine): the token's workspaces connect by themselves
    #[serde(default)]
    pub auto: bool,
}

/// What an `oc1` token says (spec-workspaces §6): `oc1.<base64url(JSON {v:1, urls:[...], id})>.<secret>`.
#[derive(Clone, Debug, PartialEq)]
pub struct EngineToken {
    pub urls: Vec<String>,
    pub id: String,
}
pub fn parse_engine_token(t: &str) -> Result<EngineToken, String> {
    use base64::Engine;
    let t = t.trim();
    let mut parts = t.splitn(3, '.');
    if parts.next() != Some("oc1") {
        return Err("not an engine token (they start with oc1.)".into());
    }
    let payload = parts.next().ok_or("an incomplete token")?;
    if parts.next().map(|s| s.is_empty()).unwrap_or(true) {
        return Err("an incomplete token".into());
    }
    let raw = base64::engine::general_purpose::URL_SAFE_NO_PAD
        .decode(payload.trim_end_matches('='))
        .map_err(|_| "a damaged token (its address part is not base64url)".to_string())?;
    let v: Value = serde_json::from_slice(&raw).map_err(|_| "a damaged token (its address part is not JSON)".to_string())?;
    if v["v"].as_i64() != Some(1) {
        return Err(format!("a token of a version this app does not know ({})", v["v"]));
    }
    let urls: Vec<String> = v["urls"].as_array().map(|a| a.iter().filter_map(|u| u.as_str()).filter_map(|u| EngineCfg::normalize(u).ok()).collect()).unwrap_or_default();
    if urls.is_empty() {
        return Err("the token carries no engine address".into());
    }
    Ok(EngineToken { urls, id: v["id"].as_str().unwrap_or("").to_string() })
}

/// protocol-tokens-v1: try the token's addresses in order (GET /engine/workspaces with the token) and keep the first
/// that answers; on the same Mac the local address wins when it answers too
pub async fn pick_url(urls: &[String], token: &str) -> Result<String, String> {
    let mut last = String::from("no address");
    let mut ok: Vec<String> = vec![];
    for u in urls {
        let cfg = EngineCfg { endpoint: u.clone(), ..Default::default() };
        match list_workspaces_with(&cfg, Some(token)).await {
            Ok(_) => ok.push(u.clone()),
            Err(e) if e.contains("refused") => return Err(format!("the engine refused the token ({u})")),
            Err(e) => last = format!("{u}: {e}"),
        }
    }
    // OPTCHAT_NO_LOCAL_URL=1 (tests, lan-first measurements): the tunnel even when this Mac's own address answers
    let no_local = std::env::var("OPTCHAT_NO_LOCAL_URL").is_ok_and(|v| v == "1");
    if no_local {
        ok.retain(|u| !crate::tokens::is_local(u));
    } else if let Some(local) = ok.iter().find(|u| crate::tokens::is_local(u)) {
        return Ok(local.clone());
    }
    ok.into_iter().next().ok_or_else(|| format!("no address of the engine answers ({last})"))
}
impl EngineCfg {
    pub fn normalize(endpoint: &str) -> Result<String, String> {
        let e = endpoint.trim().trim_end_matches('/').to_string();
        let e = if e.contains("://") { e } else { format!("https://{e}") };
        let local = e.starts_with("http://127.0.0.1:") || e.starts_with("http://localhost:");
        if !(e.starts_with("https://") || local) {
            return Err(format!("an engine is https://<host> (got {e})"));
        }
        Ok(e)
    }
    fn host(&self) -> &str {
        self.endpoint.split("://").nth(1).unwrap_or(&self.endpoint)
    }
    /// the edge admits same-origin requests only: Origin https://<host> (a local relay rewrites it)
    pub fn origin(&self) -> String {
        format!("https://{}", self.host())
    }
    pub fn ws_base(&self) -> String {
        if self.endpoint.starts_with("http://") { format!("ws://{}", self.host()) } else { format!("wss://{}", self.host()) }
    }
    pub fn label(&self) -> String {
        self.name.clone()
    }
}

/// The key of a connected workspace in the app: `<endpoint>|<workspace>`.
pub fn ws_key(endpoint: &str, ws: &str) -> String {
    format!("{endpoint}|{ws}")
}

/// What the connections send the app: (engine endpoint, workspace or None for the engine itself, event).
pub type Routed = (String, Option<String>, HubEvent);

pub struct Blob {
    pub media_type: String,
    pub bytes: Vec<u8>,
}

type RpcReply = oneshot::Sender<Result<Value, String>>;
type ControlReply = oneshot::Sender<(bool, String, Option<i64>)>;
type Ws = tokio_tungstenite::WebSocketStream<tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>>;

/// One engine socket (lan-first: up to three at once while switching: the primary, a candidate, a draining one).
struct Sock {
    gen: u64,
    tx: mpsc::UnboundedSender<WsMsg>,
    role: Role,
}
enum Role {
    /// opened next to the primary: its messages wait here until every workspace has its snapshot, then it takes over
    Candidate { buf: Vec<String>, need: HashSet<String>, ready: Option<oneshot::Sender<()>> },
    /// the socket the app uses
    Primary,
    /// replaced: only answers to requests still in flight are taken from it, then it closes
    Draining,
}
enum Ctl {
    /// the primary socket closed
    Lost(PathKind),
    /// the lan socket missed a heartbeat
    HbMiss(u64),
    /// re-evaluate the path now (tests, lan on/off)
    Kick,
}

/// what the lan-first test hooks count (OPTCHAT_LAN_PINGER): every ping id should be answered exactly once
#[derive(Default, Debug)]
pub struct PingStats {
    pub sent: u64,
    pub answered: u64,
    pub dup: u64,
    answered_ids: HashSet<u64>,
}

/// One socket to an engine (`/engine/ws`) with one client token; it carries every workspace of that token the app is
/// connected to (`connect {ws}` / `disconnect {ws}`; `ws` on every message). Reconnects with backoff and connects
/// its workspaces again (each answers with `hello` and a snapshot).
pub struct EngineConn {
    /// the token the socket itself opens with (a token-scoped engine: the engine then lists only its workspaces)
    socket_token: Option<String>,
    pub cfg: EngineCfg,
    events: mpsc::UnboundedSender<Routed>,
    out: Mutex<Option<mpsc::UnboundedSender<WsMsg>>>,
    rpcs: Mutex<HashMap<u64, RpcReply>>,
    controls: Mutex<HashMap<u64, ControlReply>>,
    next: AtomicU64,
    /// connected workspaces → (the task on screen there, the client token that connects it)
    subs: Mutex<HashMap<String, (i64, String)>>,
    stopped: AtomicBool,
    pub online: AtomicBool,
    pub reconnects: AtomicU64,
    http: reqwest::Client,
    // ---- lan-first ----
    socks: Mutex<Vec<Sock>>,
    gens: AtomicU64,
    ctl: mpsc::UnboundedSender<Ctl>,
    ctl_rx: Mutex<Option<mpsc::UnboundedReceiver<Ctl>>>,
    /// the primary socket's path, and the last heartbeat round trip on it (ms)
    path: Mutex<Option<PathKind>>,
    rtt_ms: Mutex<Option<f64>>,
    lan_http: Mutex<Option<Arc<lan::LanHttp>>>,
    ticket: Mutex<Option<lan::Ticket>>,
    /// trust on first use (poc): the first pin seen per engine; a later advert with another pin is ignored
    pins: Mutex<HashMap<String, String>>,
    pub lan_disabled: AtomicBool,
    pings: Mutex<HashMap<u64, oneshot::Sender<()>>>,
    pub ping_stats: Mutex<PingStats>,
}

impl EngineConn {
    pub fn start(cfg: EngineCfg, socket_token: Option<String>, events: mpsc::UnboundedSender<Routed>) -> Arc<EngineConn> {
        let (ctl, ctl_rx) = mpsc::unbounded_channel();
        let c = Arc::new(EngineConn {
            socks: Mutex::new(vec![]),
            gens: AtomicU64::new(1),
            ctl,
            ctl_rx: Mutex::new(Some(ctl_rx)),
            path: Mutex::new(None),
            rtt_ms: Mutex::new(None),
            lan_http: Mutex::new(None),
            ticket: Mutex::new(None),
            pins: Mutex::new(HashMap::new()),
            lan_disabled: AtomicBool::new(false),
            pings: Mutex::new(HashMap::new()),
            ping_stats: Mutex::new(PingStats::default()),
            socket_token,
            cfg,
            events,
            out: Mutex::new(None),
            rpcs: Mutex::new(HashMap::new()),
            controls: Mutex::new(HashMap::new()),
            next: AtomicU64::new(1),
            subs: Mutex::new(HashMap::new()),
            stopped: AtomicBool::new(false),
            online: AtomicBool::new(false),
            reconnects: AtomicU64::new(0),
            http: reqwest::Client::builder().timeout(Duration::from_secs(30)).build().expect("http client"),
        });
        let me = c.clone();
        tokio::spawn(async move { me.run().await });
        // lan-first test hooks (env; never set in a normal launch)
        if c.socket_token.is_some() {
            if let Some(n) = std::env::var("OPTCHAT_LAN_BENCH").ok().and_then(|v| v.parse::<usize>().ok()) {
                let me = c.clone();
                tokio::spawn(async move { me.bench(n).await });
            }
            if let Some(ms) = std::env::var("OPTCHAT_LAN_PINGER").ok().and_then(|v| v.parse::<u64>().ok()) {
                let me = c.clone();
                tokio::spawn(async move { me.pinger(ms).await });
            }
        }
        c
    }
    fn emit(&self, ws: Option<&str>, e: HubEvent) {
        let _ = self.events.send((self.cfg.endpoint.clone(), ws.map(String::from), e));
    }
    fn emit_all(&self, e: impl Fn() -> HubEvent) {
        let names: Vec<String> = self.subs.lock().keys().cloned().collect();
        for n in names {
            self.emit(Some(&n), e());
        }
    }

    // ---- the socket(s): lan-first path manager --------------------------------------------------------------------
    /// Keeps the engine socket on the best path (lan.rs `Sm`): the tunnel (cfg.endpoint) as the baseline, a lan front
    /// of this engine when bonjour finds one on the local network. Switches make-before-break; a dead lan socket or
    /// a heartbeat miss goes back to the tunnel at once; a dead tunnel reconnects with backoff (0.4 s doubling to 4 s)
    /// and resnapshots, as before.
    async fn run(self: Arc<Self>) {
        let Some(mut ctl_rx) = self.ctl_rx.lock().take() else { return };
        // lan only for engines added with a token (the token mints the lan ticket) that have a public address
        let mut world = if self.socket_token.is_some() { lan::world() } else { None };
        let mut epoch = world.as_ref().map(|w| w.borrow().net_epoch).unwrap_or(0);
        let mut sm = lan::Sm::default();
        if let Some(s) = std::env::var("OPTCHAT_LAN_RETRY_S").ok().and_then(|v| v.parse().ok()) {
            sm.lan_retry = Duration::from_secs(s);
        }
        let mut first = true;
        while !self.stopped.load(Ordering::SeqCst) {
            sm.lan_disabled = self.lan_disabled.load(Ordering::SeqCst);
            let (env, svc) = self.lan_pick(world.as_ref());
            // re-probe at least every 30 s, and when a lan backoff ends
            let mut wait = sm.retry_in(Instant::now()).map(|d| d + Duration::from_millis(5)).unwrap_or(Duration::from_secs(30)).min(Duration::from_secs(30));
            if let lan::Step::Go(p) = sm.step(Instant::now(), env) {
                let from = sm.primary;
                if from.is_none() {
                    self.emit_all(|| HubEvent::Conn(Conn::Connecting));
                }
                let t0 = Instant::now();
                match self.open_path(p, svc).await {
                    Ok(how) => {
                        sm.adopted(p);
                        if from.is_none() && !first {
                            self.reconnects.fetch_add(1, Ordering::SeqCst);
                        }
                        first = false;
                        lan::log(&format!("{} {} -> {} in {:.0} ms ({how})", self.cfg.name, from.map(|f| f.name()).unwrap_or("none"), p.name(), t0.elapsed().as_secs_f64() * 1000.0));
                        continue;
                    }
                    Err(e) => {
                        wait = sm.failed(p, Instant::now());
                        lan::log(&format!("{} {} failed after {:.0} ms: {e}", self.cfg.name, p.name(), t0.elapsed().as_secs_f64() * 1000.0));
                        if sm.primary.is_none() && p == PathKind::Tunnel {
                            self.online.store(false, Ordering::SeqCst);
                            self.emit(None, HubEvent::EngineOnline(false));
                            self.emit_all(|| HubEvent::Conn(Conn::Closed));
                            self.emit(None, HubEvent::EngineError(e));
                        }
                        if wait.is_zero() {
                            continue;
                        }
                    }
                }
            }
            // wait for a socket event, a network / bonjour change, or the next re-probe
            tokio::select! {
                c = ctl_rx.recv() => match c {
                    Some(Ctl::Lost(p)) => {
                        let w = sm.lost(p, Instant::now());
                        if !w.is_zero() && !self.stopped.load(Ordering::SeqCst) {
                            tokio::time::sleep(w).await;
                        }
                    }
                    Some(Ctl::HbMiss(gen)) => {
                        if self.is_primary(gen) {
                            sm.failed(PathKind::Lan, Instant::now());
                        }
                    }
                    Some(Ctl::Kick) => {}
                    None => break,
                },
                _ = async { match world.as_mut() { Some(w) => { let _ = w.changed().await; } None => std::future::pending::<()>().await } } => {
                    let e = world.as_ref().map(|w| w.borrow().net_epoch).unwrap_or(0);
                    if e != epoch {
                        epoch = e;
                        sm.net_changed();
                    }
                }
                _ = tokio::time::sleep(wait) => {}
            }
        }
        for s in self.socks.lock().drain(..) {
            let _ = s.tx.send(WsMsg::Close(None));
        }
    }

    /// the lan front for this engine, if one is advertised on this network (and the network is local)
    fn lan_pick(&self, world: Option<&tokio::sync::watch::Receiver<lan::World>>) -> (lan::Env, Option<lan::LanService>) {
        let Some(w) = world else { return (lan::Env::default(), None) };
        let w = w.borrow();
        let mut urls = self.cfg.urls.clone();
        urls.push(self.cfg.endpoint.clone());
        let svc = w.services.iter().find(|s| s.serves(&urls) && self.pin_trusted(s)).cloned();
        (lan::Env { lan_seen: svc.is_some(), net_local: w.net.local() }, svc)
    }
    fn pin_trusted(&self, s: &lan::LanService) -> bool {
        let mut pins = self.pins.lock();
        match pins.get(&s.engine) {
            Some(p) if p != &s.pin => false,
            Some(_) => true,
            None => {
                pins.insert(s.engine.clone(), s.pin.clone());
                true
            }
        }
    }
    fn is_primary(&self, gen: u64) -> bool {
        self.socks.lock().iter().any(|s| s.gen == gen && matches!(s.role, Role::Primary))
    }
    /// the path the engine socket is on now ("lan" / "tunnel"), for the status bar and tests
    pub fn path(&self) -> Option<PathKind> {
        *self.path.lock()
    }
    pub fn path_label(&self) -> String {
        match (self.path(), *self.rtt_ms.lock()) {
            (Some(p), Some(ms)) => format!("{} {:.0} ms", p.name(), ms),
            (Some(p), None) => p.name().to_string(),
            _ => String::new(),
        }
    }
    /// tests / a user switch: keep the socket off the lan (it moves to the tunnel make-before-break)
    pub fn set_lan_disabled(&self, off: bool) {
        self.lan_disabled.store(off, Ordering::SeqCst);
        let _ = self.ctl.send(Ctl::Kick);
    }

    /// Open a socket on `p`. With no socket up it becomes the primary at once (as before lan-first). Next to a
    /// primary it's a candidate: every workspace is connected on it, and once each has its snapshot it takes over;
    /// the old one drains for 5 s (answers to requests in flight), then closes.
    async fn open_path(self: &Arc<Self>, p: PathKind, svc: Option<lan::LanService>) -> Result<String, String> {
        let t0 = Instant::now();
        let ws = match p {
            PathKind::Tunnel => self.connect_with(&format!("{}/engine/ws", self.cfg.ws_base()), self.socket_token.as_deref()).await?,
            PathKind::Lan => self.dial_lan_ws(svc.ok_or("no lan front")?).await?,
        };
        let dial_ms = t0.elapsed().as_secs_f64() * 1000.0;
        let gen = self.gens.fetch_add(1, Ordering::SeqCst);
        let (mut w, r) = ws.split();
        let (tx, mut rx) = mpsc::unbounded_channel::<WsMsg>();
        tokio::spawn(async move {
            while let Some(m) = rx.recv().await {
                if w.send(m).await.is_err() {
                    break;
                }
            }
            let _ = w.close().await;
        });
        let subs: Vec<(String, i64, String)> = self.subs.lock().iter().map(|(w, (t, k))| (w.clone(), *t, k.clone())).collect();
        let (rtx, rrx) = oneshot::channel();
        let switching = {
            let mut socks = self.socks.lock();
            let switching = socks.iter().any(|s| matches!(s.role, Role::Primary));
            let role = if switching { Role::Candidate { buf: vec![], need: subs.iter().map(|s| s.0.clone()).collect(), ready: Some(rtx) } } else { Role::Primary };
            if !switching {
                *self.out.lock() = Some(tx.clone());
            }
            socks.push(Sock { gen, tx: tx.clone(), role });
            switching
        };
        if !switching {
            self.online.store(true, Ordering::SeqCst);
            self.emit(None, HubEvent::EngineOnline(true));
            self.emit(None, HubEvent::EngineError(String::new()));
        }
        // every connected workspace on the new socket: each answers hello + snapshot
        for (ws, task, token) in &subs {
            let _ = tx.send(WsMsg::text(json!({ "type": "connect", "ws": ws, "task": task, "token": token }).to_string()));
        }
        let pong = Arc::new(tokio::sync::Notify::new());
        let me = self.clone();
        let pg = pong.clone();
        tokio::spawn(async move { me.reader(gen, p, r, pg).await });
        let me = self.clone();
        let htx = tx.clone();
        tokio::spawn(async move { me.heartbeat(gen, p, htx, pong).await });
        let mut ready_ms = 0.0;
        if switching {
            if !subs.is_empty() {
                let ok = matches!(tokio::time::timeout(Duration::from_secs(8), rrx).await, Ok(Ok(())));
                if !ok {
                    self.socks.lock().retain(|s| s.gen != gen);
                    let _ = tx.send(WsMsg::Close(None));
                    return Err("no snapshots on the new socket".into());
                }
            }
            ready_ms = t0.elapsed().as_secs_f64() * 1000.0;
            self.promote(gen, &subs);
        }
        *self.path.lock() = Some(p);
        *self.rtt_ms.lock() = None;
        // the status bar shows the path: a refresh
        self.emit(None, HubEvent::EngineOnline(true));
        Ok(if switching { format!("dial {dial_ms:.0} ms, snapshots {ready_ms:.0} ms, make-before-break") } else { format!("dial {dial_ms:.0} ms") })
    }

    /// the candidate `gen` takes over: its buffered messages (hello + snapshots and what followed) go to the app in
    /// order, the old primary drains
    fn promote(self: &Arc<Self>, gen: u64, sent: &[(String, i64, String)]) {
        let mut old: Vec<mpsc::UnboundedSender<WsMsg>> = vec![];
        {
            let mut socks = self.socks.lock();
            for s in socks.iter_mut() {
                if matches!(s.role, Role::Primary) {
                    s.role = Role::Draining;
                    old.push(s.tx.clone());
                }
            }
            let Some(s) = socks.iter_mut().find(|s| s.gen == gen) else { return };
            let buf = match std::mem::replace(&mut s.role, Role::Primary) {
                Role::Candidate { buf, .. } => buf,
                _ => vec![],
            };
            *self.out.lock() = Some(s.tx.clone());
            // under the lock: the reader can't deliver a newer message before these
            for m in buf {
                self.on_message(&m);
            }
        }
        // a workspace connected (or a task opened) while switching: connect it again on the new socket
        let now: Vec<(String, i64, String)> = self.subs.lock().iter().map(|(w, (t, k))| (w.clone(), *t, k.clone())).collect();
        for (ws, task, token) in now {
            if !sent.iter().any(|(w, t, _)| *w == ws && *t == task) {
                let _ = self.send_json(json!({ "type": "connect", "ws": ws, "task": task, "token": token }));
            }
        }
        for o in old {
            tokio::spawn(async move {
                tokio::time::sleep(Duration::from_secs(5)).await;
                let _ = o.send(WsMsg::Close(None));
            });
        }
    }

    async fn reader(self: Arc<Self>, gen: u64, p: PathKind, mut r: futures_util::stream::SplitStream<Ws>, pong: Arc<tokio::sync::Notify>) {
        while let Some(msg) = r.next().await {
            match msg {
                Ok(WsMsg::Text(t)) => self.dispatch(gen, t.as_str()),
                Ok(WsMsg::Pong(_)) => pong.notify_one(),
                Ok(WsMsg::Close(_)) | Err(_) => break,
                _ => {}
            }
        }
        let role = {
            let mut socks = self.socks.lock();
            let i = socks.iter().position(|s| s.gen == gen);
            i.map(|i| socks.remove(i).role)
        };
        if matches!(role, Some(Role::Primary)) {
            *self.out.lock() = None;
            *self.path.lock() = None;
            for (_, tx) in self.rpcs.lock().drain() {
                let _ = tx.send(Err("disconnected from the engine".into()));
            }
            for (_, tx) in self.controls.lock().drain() {
                let _ = tx.send((false, "disconnected from the engine".into(), None));
            }
            self.online.store(false, Ordering::SeqCst);
            self.emit(None, HubEvent::EngineOnline(false));
            self.emit_all(|| HubEvent::Conn(Conn::Closed));
            lan::log(&format!("{} {} socket closed", self.cfg.name, p.name()));
            let _ = self.ctl.send(Ctl::Lost(p));
        }
    }

    /// a message from socket `gen`, by its role
    fn dispatch(&self, gen: u64, text: &str) {
        let mut socks = self.socks.lock();
        let Some(s) = socks.iter_mut().find(|s| s.gen == gen) else { return };
        match &mut s.role {
            Role::Primary => {
                drop(socks);
                self.on_message(text);
            }
            Role::Candidate { buf, need, ready } => {
                buf.push(text.to_string());
                if let Ok(m) = serde_json::from_str::<Value>(text) {
                    let ws = m["ws"].as_str().unwrap_or("");
                    let done = match m["type"].as_str() {
                        Some("snapshot") | Some("disabled") => true,
                        Some("error") => m.get("id").is_none(),
                        _ => false,
                    };
                    if done {
                        need.remove(ws);
                    }
                }
                if need.is_empty() {
                    if let Some(r) = ready.take() {
                        let _ = r.send(());
                    }
                }
            }
            Role::Draining => {
                drop(socks);
                // only answers to what was asked on it
                if let Ok(m) = serde_json::from_str::<Value>(text) {
                    if m.get("id").is_some() && matches!(m["type"].as_str(), Some("rpc") | Some("controlDone") | Some("error")) {
                        self.on_message(text);
                    }
                }
            }
        }
    }

    /// websocket ping frames (PROTOCOL §6: lan every 10 s, 3 s deadline; the lan front answers them itself, so a miss
    /// means the lan hop is gone). The tunnel is pinged too (15 s), only to show its round trip.
    async fn heartbeat(self: Arc<Self>, gen: u64, p: PathKind, tx: mpsc::UnboundedSender<WsMsg>, pong: Arc<tokio::sync::Notify>) {
        let every = std::env::var("OPTCHAT_LAN_HB_MS").ok().and_then(|v| v.parse().ok()).map(Duration::from_millis).unwrap_or(Duration::from_secs(if p == PathKind::Lan { 10 } else { 15 }));
        let deadline = Duration::from_secs(3);
        let mut first = true;
        loop {
            // the first one soon, so the label shows a round trip
            tokio::time::sleep(if first { Duration::from_millis(500) } else { every }).await;
            first = false;
            if !self.socks.lock().iter().any(|s| s.gen == gen) {
                return;
            }
            if !self.is_primary(gen) {
                continue; // a candidate or a draining socket: only the primary's health matters
            }
            let n = pong.notified();
            tokio::pin!(n);
            n.as_mut().enable();
            let t = Instant::now();
            if tx.send(WsMsg::Ping(Vec::new().into())).is_err() {
                return;
            }
            match tokio::time::timeout(deadline, n).await {
                Ok(()) => {
                    if self.is_primary(gen) {
                        *self.rtt_ms.lock() = Some(t.elapsed().as_secs_f64() * 1000.0);
                    }
                }
                Err(_) => {
                    lan::log(&format!("{} heartbeat missed on {} ({} ms deadline)", self.cfg.name, p.name(), deadline.as_millis()));
                    if p == PathKind::Lan {
                        let _ = self.ctl.send(Ctl::HbMiss(gen));
                        return;
                    }
                }
            }
        }
    }

    // ---- lan: tickets, the pinned socket, pinned GETs ----------------------------------------------------------------
    fn lan_http_for(&self, svc: &lan::LanService) -> Arc<lan::LanHttp> {
        let mut g = self.lan_http.lock();
        match g.as_ref() {
            Some(h) if h.svc == *svc => h.clone(),
            _ => {
                let h = Arc::new(lan::LanHttp::new(svc.clone()));
                *g = Some(h.clone());
                h
            }
        }
    }
    /// a ticket good for opening something now (minted with the engine token over the pinned connection)
    async fn lan_ticket(&self, http: &lan::LanHttp, fresh: bool) -> Result<String, String> {
        let now = Instant::now();
        if !fresh {
            if let Some(t) = self.ticket.lock().as_ref().filter(|t| t.usable(now) && !t.renew_due(now)) {
                return Ok(t.value.clone());
            }
        }
        let tok = self.socket_token.clone().ok_or("no engine token")?;
        let t0 = Instant::now();
        let t = http.mint(&tok).await?;
        lan::log(&format!("ticket minted in {:.0} ms (ttl {} s)", t0.elapsed().as_secs_f64() * 1000.0, t.ttl.as_secs()));
        let v = t.value.clone();
        *self.ticket.lock() = Some(t);
        Ok(v)
    }
    async fn dial_lan_ws(&self, svc: lan::LanService) -> Result<Ws, String> {
        let http = self.lan_http_for(&svc);
        let mut fresh = false;
        loop {
            let ticket = self.lan_ticket(&http, fresh).await?;
            let tls = lan::dial(&svc, Duration::from_millis(1500)).await?;
            let mut req = format!("wss://{}:{}/engine/ws", lan::SNI, svc.port).into_client_request().map_err(|e| e.to_string())?;
            req.headers_mut().insert("Authorization", format!("Bearer {ticket}").parse().map_err(|_| "bad ticket".to_string())?);
            let r = tokio::time::timeout(Duration::from_secs(5), tokio_tungstenite::client_async(req, tokio_tungstenite::MaybeTlsStream::NativeTls(tls))).await.map_err(|_| "timed out".to_string())?;
            match r {
                Ok((ws, _)) => return Ok(ws),
                Err(tokio_tungstenite::tungstenite::Error::Http(r)) if r.status().as_u16() == 401 && !fresh => {
                    // ticket_expired / ticket_invalid (the front restarted): mint again, once
                    fresh = true;
                }
                Err(e) => return Err(e.to_string()),
            }
        }
    }
    /// GET over the pinned lan connection with a ticket (blobs, workspace lists, measurements)
    pub async fn lan_get(&self, path: &str) -> Result<lan::Resp, String> {
        let http = self.lan_http.lock().clone().ok_or("no lan front")?;
        let ticket = self.lan_ticket(&http, false).await?;
        let r = http.request("GET", path, Some(&ticket)).await?;
        if r.status == 401 && r.lan_code.as_deref().is_some_and(|c| c.starts_with("ticket_")) {
            let ticket = self.lan_ticket(&http, true).await?;
            return http.request("GET", path, Some(&ticket)).await;
        }
        Ok(r)
    }

    /// one round trip to the engine over the current socket: a message it answers at once (an error for a
    /// workspace nobody connects to; the engine has no ping message yet, PROTOCOL §8)
    pub async fn ping(&self) -> Result<f64, String> {
        let id = self.next.fetch_add(1, Ordering::SeqCst);
        let (tx, rx) = oneshot::channel();
        self.pings.lock().insert(id, tx);
        self.ping_stats.lock().sent += 1;
        let t = Instant::now();
        if let Err(e) = self.send_json(json!({ "type": "ping", "id": id, "ws": "__lanprobe__" })) {
            self.pings.lock().remove(&id);
            return Err(e);
        }
        match tokio::time::timeout(Duration::from_secs(5), rx).await {
            Ok(Ok(())) => Ok(t.elapsed().as_secs_f64() * 1000.0),
            _ => {
                self.pings.lock().remove(&id);
                Err("no answer".into())
            }
        }
    }

    // ---- lan-first measurements (test hooks) -------------------------------------------------------------------------
    async fn wait_path(&self, p: PathKind, within: Duration) -> Option<f64> {
        let t = Instant::now();
        while t.elapsed() < within {
            if self.path() == Some(p) {
                return Some(t.elapsed().as_secs_f64() * 1000.0);
            }
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
        None
    }
    async fn sample(&self, n: usize, p: PathKind) -> (Vec<f64>, Vec<f64>) {
        let mut ws = vec![];
        let mut get = vec![];
        for _ in 0..n {
            if let Ok(ms) = self.ping().await {
                ws.push(ms);
            }
        }
        for _ in 0..n {
            let t = Instant::now();
            let ok = match p {
                PathKind::Lan => self.lan_get("/engine/workspaces").await.map(|r| r.status == 200).unwrap_or(false),
                PathKind::Tunnel => {
                    let mut rb = self.http.get(format!("{}/engine/workspaces", self.cfg.endpoint)).header("Origin", self.cfg.origin());
                    if let Some(t) = &self.socket_token {
                        rb = rb.bearer_auth(t);
                    }
                    match rb.send().await {
                        Ok(r) => r.status().is_success() && r.bytes().await.is_ok(),
                        Err(_) => false,
                    }
                }
            };
            if ok {
                get.push(t.elapsed().as_secs_f64() * 1000.0);
            }
        }
        (ws, get)
    }
    /// OPTCHAT_LAN_BENCH=<n>: once on lan, n engine round trips + n GET /engine/workspaces on lan, a planned switch
    /// to the tunnel, the same there, and a planned switch back; one `lan-bench` JSON line in the log.
    async fn bench(self: Arc<Self>, n: usize) {
        let Some(found) = self.wait_path(PathKind::Lan, Duration::from_secs(90)).await else {
            lan::log("bench: never on lan");
            return;
        };
        tokio::time::sleep(Duration::from_secs(2)).await;
        // a warm-up GET each way (connection + ticket), not counted
        let _ = self.lan_get("/engine/workspaces").await;
        let (lan_ws, lan_get) = self.sample(n, PathKind::Lan).await;
        let t = Instant::now();
        self.set_lan_disabled(true);
        let to_tunnel = self.wait_path(PathKind::Tunnel, Duration::from_secs(30)).await;
        tokio::time::sleep(Duration::from_secs(6)).await;
        let _ = self.sample(1, PathKind::Tunnel).await;
        let (tun_ws, tun_get) = self.sample(n, PathKind::Tunnel).await;
        let _ = t;
        self.set_lan_disabled(false);
        let to_lan = self.wait_path(PathKind::Lan, Duration::from_secs(30)).await;
        let mp = |v: &Vec<f64>| {
            let (m, p) = lan::median_p90(v);
            json!({ "n": v.len(), "median": (m * 10.0).round() / 10.0, "p90": (p * 10.0).round() / 10.0 })
        };
        let out = json!({
            "lan": { "ws_rtt": mp(&lan_ws), "get": mp(&lan_get) },
            "tunnel": { "ws_rtt": mp(&tun_ws), "get": mp(&tun_get) },
            "switch_ms": { "lan_to_tunnel": to_tunnel.map(|x| x.round()), "tunnel_to_lan": to_lan.map(|x| x.round()) },
            "on_lan_after_ms": found.round(),
            "pings": { "sent": self.ping_stats.lock().sent, "answered": self.ping_stats.lock().answered, "dup": self.ping_stats.lock().dup },
        });
        lan::log(&format!("lan-bench {out}"));
    }
    /// OPTCHAT_LAN_PINGER=<ms>: an engine round trip every <ms> for the whole run; every 5 s a line with the path,
    /// how many were sent / answered / answered twice, for the switching tests (no lost or doubled answers)
    async fn pinger(self: Arc<Self>, every_ms: u64) {
        let mut last = Instant::now();
        let mut fails = 0u64;
        let mut rtts: Vec<f64> = vec![];
        loop {
            tokio::time::sleep(Duration::from_millis(every_ms)).await;
            if self.stopped.load(Ordering::SeqCst) {
                return;
            }
            let me = self.clone();
            match me.ping().await {
                Ok(ms) => rtts.push(ms),
                Err(_) => fails += 1,
            }
            if last.elapsed() >= Duration::from_secs(5) {
                last = Instant::now();
                let st = self.ping_stats.lock();
                let (m, p) = lan::median_p90(&rtts);
                lan::log(&format!("pinger path={} sent={} answered={} dup={} unanswered={} rtt_med={:.1} p90={:.1}", self.path().map(|p| p.name()).unwrap_or("none"), st.sent, st.answered, st.dup, fails, m, p));
                rtts.clear();
            }
        }
    }

    /// a socket to the engine; `token` only for a dictation socket (the engine socket's tokens come with each connect)
    async fn connect_with(&self, url: &str, token: Option<&str>) -> Result<tokio_tungstenite::WebSocketStream<tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>>, String> {
        let mut req = url.into_client_request().map_err(|e| e.to_string())?;
        let h = req.headers_mut();
        h.insert("Origin", self.cfg.origin().parse().map_err(|_| "bad origin".to_string())?);
        if let Some(t) = token {
            h.insert("Authorization", format!("Bearer {t}").parse().map_err(|_| "bad token".to_string())?);
        }
        let (ws, _) = tokio::time::timeout(Duration::from_secs(10), tokio_tungstenite::connect_async(req)).await.map_err(|_| "timed out".to_string())?.map_err(|e| match e {
            tokio_tungstenite::tungstenite::Error::Http(r) if r.status().as_u16() == 401 => "the token was refused".to_string(),
            e => e.to_string(),
        })?;
        Ok(ws)
    }

    fn on_message(&self, text: &str) {
        let Ok(m) = serde_json::from_str::<Value>(text) else { return };
        let ws = m["ws"].as_str().map(String::from);
        let ws = ws.as_deref();
        match m["type"].as_str() {
            Some("rpc") => {
                if let Some(tx) = m["id"].as_u64().and_then(|id| self.rpcs.lock().remove(&id)) {
                    let _ = tx.send(match m.get("error").and_then(|e| e.as_str()) {
                        Some(e) => Err(e.to_string()),
                        None => Ok(m["result"].clone()),
                    });
                }
            }
            Some("controlDone") => {
                let ok = m["ok"].as_bool().unwrap_or(false);
                let message = m["message"].as_str().unwrap_or("").to_string();
                let spawned = m["spawned"].as_i64();
                match m["id"].as_u64().and_then(|id| self.controls.lock().remove(&id)) {
                    Some(tx) => {
                        let _ = tx.send((ok, message, spawned));
                    }
                    None => self.emit(ws, HubEvent::Notice { ok, message, spawned }),
                }
            }
            Some("error") if m["id"].as_u64().is_some() && m["ws"].as_str() == Some("__lanprobe__") => {
                // an answer to ping(): counted once per id (a second one would be a doubled message)
                let id = m["id"].as_u64().unwrap();
                let mut st = self.ping_stats.lock();
                if st.answered_ids.insert(id) {
                    st.answered += 1;
                } else {
                    st.dup += 1;
                }
                drop(st);
                if let Some(tx) = self.pings.lock().remove(&id) {
                    let _ = tx.send(());
                }
            }
            Some("error") if m["id"].as_u64().is_some() => {
                // a refused request of ours (not connected, out of scope): answer whoever waits
                let id = m["id"].as_u64().unwrap();
                let msg = m["message"].as_str().unwrap_or("refused").to_string();
                if let Some(tx) = self.rpcs.lock().remove(&id) {
                    let _ = tx.send(Err(msg));
                } else if let Some(tx) = self.controls.lock().remove(&id) {
                    let _ = tx.send((false, msg, None));
                }
            }
            Some("error") if ws.is_some_and(|w| self.subs.lock().contains_key(w)) && m["message"].as_str().is_some_and(|t| t.starts_with("no token")) => {
                // a connect refused (its token does not reach the workspace): stop asking for it
                let w = ws.unwrap();
                self.subs.lock().remove(w);
                self.emit(Some(w), HubEvent::Refused(m["message"].as_str().unwrap_or("refused").to_string()));
            }
            Some("workspaces") => {
                let list: Vec<WsEntry> = serde_json::from_value(m["workspaces"].clone()).unwrap_or_default();
                self.emit(None, HubEvent::EngineWorkspaces(list));
            }
            Some("hello") => {
                self.emit(ws, HubEvent::Hello { version: m["version"].as_str().unwrap_or("").into(), build: m["build"].as_str().unwrap_or("").into() });
                self.emit(ws, HubEvent::Conn(Conn::Open));
            }
            Some("disabled") => {
                if let Some(w) = ws {
                    self.subs.lock().remove(w);
                }
                self.emit(ws, HubEvent::Disabled(m["reason"].as_str().unwrap_or("disabled on the engine").into()));
            }
            Some("disconnected") => {
                // dropped by the engine (its token revoked): the reason says so; our own disconnects have none
                if let (Some(w), Some(r)) = (ws, m["reason"].as_str()) {
                    self.subs.lock().remove(w);
                    self.emit(Some(w), HubEvent::Refused(r.to_string()));
                }
            }
            Some("snapshot") => {
                if let Some(s) = Snapshot::parse(&m) {
                    self.emit(ws, HubEvent::Snapshot(Box::new(s)));
                }
            }
            Some("notices") => {
                self.emit(ws, HubEvent::Notices { changed: serde_json::from_value(m["notices"].clone()).unwrap_or_default(), unread: serde_json::from_value(m["unread"].clone()).unwrap_or_default() });
            }
            _ => {
                if let Some(e) = parse_event(&m) {
                    self.emit(ws, e);
                }
            }
        }
    }

    fn send_json(&self, v: Value) -> Result<(), String> {
        match self.out.lock().as_ref() {
            Some(tx) => tx.send(WsMsg::text(v.to_string())).map_err(|_| "not connected to the engine".to_string()),
            None => Err("not connected to the engine".into()),
        }
    }
    pub fn connect_ws(&self, ws: &str, task: i64, token: &str) {
        self.subs.lock().insert(ws.to_string(), (task, token.to_string()));
        let _ = self.send_json(json!({ "type": "connect", "ws": ws, "task": task, "token": token }));
    }
    pub fn disconnect_ws(&self, ws: &str) {
        self.subs.lock().remove(ws);
        let _ = self.send_json(json!({ "type": "disconnect", "ws": ws }));
    }
    pub fn has_subs(&self) -> bool {
        !self.subs.lock().is_empty()
    }
    pub fn shutdown(&self) {
        self.stopped.store(true, Ordering::SeqCst);
        if let Some(tx) = self.out.lock().take() {
            let _ = tx.send(WsMsg::Close(None));
        }
    }
    pub fn drop_connection(&self) {
        if let Some(tx) = self.out.lock().as_ref() {
            let _ = tx.send(WsMsg::Close(None));
        }
    }
}

/// GET <engine>/engine/workspaces: the engine's workspaces {name, running} (no token); Err when it is unreachable.
pub async fn list_workspaces(cfg: &EngineCfg) -> Result<Vec<WsEntry>, String> {
    list_workspaces_with(cfg, None).await
}
/// GET /engine/workspaces with a token: only the token's workspaces (401: the token is refused)
pub async fn list_workspaces_with(cfg: &EngineCfg, token: Option<&str>) -> Result<Vec<WsEntry>, String> {
    let mut rb = reqwest::Client::builder()
        .timeout(Duration::from_secs(4))
        .build()
        .map_err(|e| e.to_string())?
        .get(format!("{}/engine/workspaces", cfg.endpoint))
        .header("Origin", cfg.origin());
    if let Some(t) = token {
        rb = rb.bearer_auth(t);
    }
    let r = rb.send().await.map_err(|e| e.to_string())?;
    if r.status().as_u16() == 401 {
        return Err("the token was refused".into());
    }
    if !r.status().is_success() {
        return Err(format!("{}", r.status()));
    }
    let v: Value = serde_json::from_str(&r.text().await.map_err(|e| e.to_string())?).map_err(|e| e.to_string())?;
    let a = v.as_array().cloned().or_else(|| v["workspaces"].as_array().cloned()).unwrap_or_default();
    Ok(a.into_iter().filter_map(|w| serde_json::from_value(w).ok()).collect())
}

/// A connected workspace: the API the views use (the hub's commands and rpc), tagged with its name, on its engine's
/// socket.
pub struct Client {
    pub conn: Arc<EngineConn>,
    pub ws: String,
    token: Mutex<String>,
    task: AtomicI64,
    dict: Mutex<Option<Arc<DictSession>>>,
}

impl Client {
    pub fn new(conn: Arc<EngineConn>, ws: &str, task: i64, token: &str) -> Arc<Client> {
        conn.connect_ws(ws, task, token);
        Arc::new(Client { conn, ws: ws.to_string(), token: Mutex::new(token.to_string()), task: AtomicI64::new(task), dict: Mutex::new(None) })
    }
    /// connect again (after a refusal or the engine dropped it), same token
    pub fn resubscribe(&self) {
        let t = self.token.lock().clone();
        self.conn.connect_ws(&self.ws, self.current_task(), &t);
    }
    /// connect again with another token (the local engine's token was rotated)
    pub fn retoken(&self, token: &str) {
        *self.token.lock() = token.to_string();
        self.resubscribe();
    }
    pub fn uses_token(&self, token: &str) -> bool {
        *self.token.lock() == token
    }
    fn emit(&self, e: HubEvent) {
        self.conn.emit(Some(&self.ws), e);
    }
    pub fn key(&self) -> String {
        ws_key(&self.conn.cfg.endpoint, &self.ws)
    }
    /// what the about box and the server log show
    pub fn label(&self) -> String {
        format!("{} · {}", self.conn.cfg.name, self.ws)
    }
    pub fn reconnects(&self) -> u64 {
        self.conn.reconnects.load(Ordering::SeqCst)
    }
    fn send_json(&self, mut v: Value) -> Result<(), String> {
        v["ws"] = json!(self.ws);
        self.conn.send_json(v)
    }
    /// tests: an event through the app's own path (AppState::apply), as if the engine had sent it
    pub fn inject(&self, e: HubEvent) {
        self.emit(e);
    }
    /// an image from the workspace's database, or None
    pub async fn get_blob(&self, sha: &str) -> Option<Blob> {
        self.blob(sha).await.ok().map(|(media_type, bytes)| Blob { media_type, bytes })
    }
    /// tests: close the engine socket as a network drop would; it reconnects and resnapshots
    pub fn drop_connection(&self) {
        self.conn.drop_connection();
    }
    pub fn current_task(&self) -> i64 {
        self.task.load(Ordering::SeqCst)
    }
    pub fn disconnect(&self) {
        if let Some(d) = self.dict.lock().take() {
            d.stop();
        }
        self.conn.disconnect_ws(&self.ws);
    }
    pub fn shutdown(&self) {
        if let Some(d) = self.dict.lock().take() {
            d.stop();
        }
    }
    /// mark notices read on the engine (ids, every unread one of a level, or all); every client sees it
    pub fn read_notices(&self, ids: Option<Vec<String>>, level: Option<&str>, all: bool) {
        let mut m = json!({ "type": "notices.read", "all": all });
        if let Some(i) = ids {
            m["ids"] = json!(i);
        }
        if let Some(l) = level {
            m["level"] = json!(l);
        }
        let _ = self.send_json(m);
    }
    pub async fn notices(&self, level: Option<&str>, limit: usize) -> Vec<Notice> {
        let mut p = json!({ "limit": limit });
        if let Some(l) = level {
            p["level"] = json!(l);
        }
        self.rpc("notices", p).await.ok().and_then(|v| serde_json::from_value(v).ok()).unwrap_or_default()
    }

    // ---- calls ------------------------------------------------------------------------------------------------------
    pub async fn rpc(&self, method: &str, mut params: Value) -> Result<Value, String> {
        let id = self.conn.next.fetch_add(1, Ordering::SeqCst);
        if params.get("task").is_none() {
            params["task"] = json!(self.current_task());
        }
        let (tx, rx) = oneshot::channel();
        self.conn.rpcs.lock().insert(id, tx);
        if let Err(e) = self.send_json(json!({ "type": "rpc", "id": id, "method": method, "params": params })) {
            self.conn.rpcs.lock().remove(&id);
            return Err(e);
        }
        match tokio::time::timeout(Duration::from_secs(30), rx).await {
            Ok(Ok(r)) => r,
            Ok(Err(_)) => Err("disconnected from the hub".into()),
            Err(_) => {
                self.conn.rpcs.lock().remove(&id);
                Err(format!("{method}: timed out"))
            }
        }
    }

    async fn control_msg(&self, mut msg: Value) -> Result<(String, Option<i64>), String> {
        let id = self.conn.next.fetch_add(1, Ordering::SeqCst);
        msg["id"] = json!(id);
        let (tx, rx) = oneshot::channel();
        self.conn.controls.lock().insert(id, tx);
        if let Err(e) = self.send_json(msg) {
            self.conn.controls.lock().remove(&id);
            return Err(e);
        }
        match tokio::time::timeout(Duration::from_secs(60), rx).await {
            Ok(Ok((true, m, spawned))) => Ok((m, spawned)),
            Ok(Ok((false, m, _))) => Err(m),
            Ok(Err(_)) => Err("disconnected from the hub".into()),
            Err(_) => {
                self.conn.controls.lock().remove(&id);
                Err("the engine did not answer".into())
            }
        }
    }
    /// a task control (lib/hub.ts control): pause / resume / cancel / close / tell / budget / rate / hideModel / title
    pub async fn control(&self, task: i64, op: &str, fields: Value) -> Result<String, String> {
        let mut m = json!({ "type": "control", "task": task, "op": op });
        if let Value::Object(o) = fields {
            for (k, v) in o {
                m[k] = v;
            }
        }
        self.control_msg(m).await.map(|r| r.0)
    }

    /// show this task: the hub answers with its snapshot (a HubEvent::Snapshot)
    pub fn open(&self, task: i64) {
        self.task.store(task, Ordering::SeqCst);
        if let Some(s) = self.conn.subs.lock().get_mut(&self.ws) {
            s.0 = task;
        }
        let _ = self.send_json(json!({ "type": "open", "task": task }));
    }

    pub fn images_json(images: Vec<ImageIn>) -> Value {
        use base64::Engine;
        Value::Array(images.into_iter().map(|i| json!({ "mediaType": i.media_type, "data": base64::engine::general_purpose::STANDARD.encode(&i.bytes), "name": i.name, "w": i.w, "h": i.h })).collect())
    }

    // ---- the engine's operations, as the hub exposes them ------------------------------------------------------------
    pub fn send_message(&self, task: i64, text: &str, images: Vec<ImageIn>, run: bool) -> Result<String, String> {
        self.send_json(json!({ "type": "send", "task": task, "text": text, "images": Self::images_json(images), "run": run }))?;
        Ok(String::new())
    }
    pub fn continue_turn(&self, task: i64) -> Result<(), String> {
        self.send_json(json!({ "type": "continue", "task": task }))
    }
    pub fn stop(&self, task: i64) {
        let _ = self.send_json(json!({ "type": "cancel", "task": task }));
    }
    pub fn stop_tool(&self, call_id: &str) {
        let _ = self.send_json(json!({ "type": "stopTool", "callId": call_id }));
    }
    /// the outcome (now · next turn · saved) comes back in controlDone and as an info notice
    pub async fn update_settings(&self, task: i64, patch: SettingsPatch) -> Result<String, String> {
        self.control_msg(json!({ "type": "settings", "task": task, "patch": patch })).await.map(|r| r.0)
    }
    pub fn update_global(&self, max_turns: Option<u32>, dictation: Option<DictationService>) {
        // test runs (OPTCHAT_TEST_NO_GLOBAL) never change engine-wide settings on a real engine; a local fake one is fine
        if std::env::var("OPTCHAT_TEST_NO_GLOBAL").is_ok() && !self.conn.cfg.endpoint.starts_with("http://127.0.0.1:") {
            self.emit(HubEvent::Notice { ok: false, message: "test run: engine-wide settings are not changed on a real engine".into(), spawned: None });
            return;
        }
        let mut patch = json!({});
        if let Some(m) = max_turns {
            patch["maxTurns"] = json!(m);
        }
        if let Some(d) = dictation {
            patch["dictation"] = json!(d);
        }
        let _ = self.send_json(json!({ "type": "global", "patch": patch }));
    }
    pub fn approve(&self, id: &str, accept: bool, persist: Option<String>) {
        let mut m = json!({ "type": "approve", "id": id, "accept": accept });
        if let Some(p) = persist {
            m["persist"] = json!(p);
        }
        let _ = self.send_json(m);
        // answered here: the dialog goes at once (the hub's approvalDone, to every client, removes it elsewhere)
        self.emit(HubEvent::ApprovalDone(id.to_string()));
    }
    #[allow(clippy::too_many_arguments)]
    pub async fn spawn(&self, parent: i64, mode: &str, title: &str, instruction: &str, images: Vec<ImageIn>, prev: Option<i64>, budget: Option<BudgetArgs>, settings: Option<SettingsPatch>, start: bool) -> Result<i64, String> {
        let mut m = json!({ "type": "spawn", "task": parent, "mode": mode, "title": title, "instruction": instruction, "images": Self::images_json(images), "start": start });
        if let Some(p) = prev {
            m["prev"] = json!(p);
        }
        if let Some(b) = budget {
            m["budget"] = serde_json::to_value(b).unwrap_or(Value::Null);
        }
        if let Some(s) = settings {
            m["settings"] = serde_json::to_value(s).unwrap_or(Value::Null);
        }
        let (_, spawned) = self.control_msg(m).await?;
        spawned.ok_or_else(|| "the hub did not say which task it spawned".to_string())
    }
    pub async fn pause(&self, task: i64, reason: &str) -> Result<String, String> {
        self.control(task, "pause", json!({ "reason": reason })).await
    }
    pub async fn resume(&self, task: i64, message: &str, images: Vec<ImageIn>) -> Result<String, String> {
        self.control(task, "resume", json!({ "message": message, "images": Self::images_json(images) })).await
    }
    pub async fn cancel(&self, task: i64, reason: &str) -> Result<String, String> {
        self.control(task, "cancel", json!({ "reason": reason })).await
    }
    pub async fn close(&self, task: i64, result: &str) -> Result<String, String> {
        self.control(task, "close", json!({ "result": result })).await
    }
    pub async fn set_budget(&self, task: i64, b: BudgetArgs) -> Result<String, String> {
        self.control(task, "budget", json!({ "budget": b })).await
    }
    pub async fn rename(&self, task: i64, title: &str) -> Result<String, String> {
        self.control(task, "title", json!({ "title": title })).await
    }
    pub async fn set_rate(&self, model: &str, per1m: Per1M, note: &str) -> Result<String, String> {
        let task = self.current_task();
        self.control(task, "rate", json!({ "model": model, "per1m": per1m, "note": note })).await
    }
    pub async fn hide_model(&self, model: &str, hidden: bool) -> Result<String, String> {
        let task = self.current_task();
        self.control(task, "hideModel", json!({ "model": model, "hidden": hidden })).await
    }

    // ---- rpc --------------------------------------------------------------------------------------------------------
    pub async fn turns_recent(&self, task: i64, limit: usize) -> Vec<TurnRecord> {
        self.rpc("turns.recent", json!({ "task": task, "limit": limit })).await.ok().and_then(|v| serde_json::from_value(v).ok()).unwrap_or_default()
    }
    pub async fn requests_recent(&self, task: i64, subtree: bool, limit: usize) -> Vec<RequestRecord> {
        let mut p = json!({ "task": task, "limit": limit });
        if subtree {
            p["scope"] = json!("subtree");
        }
        self.rpc("requests.recent", p).await.ok().and_then(|v| serde_json::from_value(v).ok()).unwrap_or_default()
    }
    pub async fn turn_requests(&self, turn_id: &str) -> Vec<RequestRecord> {
        self.rpc("turn.requests", json!({ "turnId": turn_id })).await.ok().and_then(|v| serde_json::from_value(v).ok()).unwrap_or_default()
    }
    /// log entries [from, to) of a task (slim unless `full`)
    pub async fn log_range(&self, task: i64, from: i64, to: i64, full: bool) -> Result<Vec<LogEntry>, String> {
        let v = self.rpc("log.range", json!({ "task": task, "from": from, "to": to, "full": full })).await?;
        serde_json::from_value(v).map_err(|e| e.to_string())
    }
    pub async fn view_parts(&self, task: i64) -> Vec<ViewPart> {
        self.rpc("view.parts", json!({ "task": task })).await.ok().and_then(|v| serde_json::from_value(v).ok()).unwrap_or_default()
    }
    pub async fn cview_parts(&self, task: i64) -> Vec<ViewPart> {
        self.rpc("cview.parts", json!({ "task": task })).await.ok().and_then(|v| serde_json::from_value(v).ok()).unwrap_or_default()
    }
    pub async fn request_detail(&self, id: &str) -> Option<RequestDetail> {
        self.rpc("request.get", json!({ "id": id })).await.ok().and_then(|v| RequestDetail::parse(&v))
    }
    pub async fn node_children(&self, t: i64, l: u32, i: i64) -> (Option<Arc<LogEntry>>, Vec<NodeKid>) {
        match self.rpc("node.children", json!({ "t": t, "l": l, "i": i })).await {
            Ok(v) => parse_node_children(&v),
            Err(_) => (None, vec![]),
        }
    }
    pub async fn search(&self, task: i64, q: &str) -> Vec<SearchHit> {
        self.rpc("search", json!({ "task": task, "q": q })).await.ok().and_then(|v| serde_json::from_value(v).ok()).unwrap_or_default()
    }
    pub async fn render(&self, task: i64) -> Option<Rendered> {
        self.rpc("render", json!({ "task": task })).await.ok().and_then(|v| Rendered::parse(&v))
    }
    pub async fn breakdown(&self, task: i64, subtree: bool) -> Breakdown {
        let mut p = json!({ "task": task });
        if subtree {
            p["scope"] = json!("subtree");
        }
        self.rpc("metrics.breakdown", p).await.map(|v| Breakdown::parse(&v)).unwrap_or_default()
    }
    pub async fn models_list(&self) -> Vec<ModelRow> {
        self.rpc("models.list", json!({})).await.ok().and_then(|v| v.as_array().map(|a| a.iter().filter_map(ModelRow::parse).collect())).unwrap_or_default()
    }
    pub async fn refresh_catalog(&self) -> Result<(), String> {
        self.rpc("catalog.refresh", json!({})).await.map(|_| ())
    }

    /// an image from the hub's database (GET /blob/<sha>)
    pub async fn blob(&self, sha: &str) -> Result<(String, Vec<u8>), String> {
        if self.conn.path() == Some(PathKind::Lan) {
            // on lan: the pinned connection with the ticket; anything wrong there → the tunnel below
            if let Ok(r) = self.conn.lan_get(&format!("/engine/blob/{sha}?ws={}", self.ws)).await {
                if r.status == 200 {
                    let mt = if r.content_type.is_empty() { "image/png".to_string() } else { r.content_type };
                    return Ok((mt, r.body.to_vec()));
                }
            }
        }
        let url = format!("{}/engine/blob/{sha}?ws={}", self.conn.cfg.endpoint, self.ws);
        let tok = self.token.lock().clone();
        let r = self.conn.http.get(url).header("Origin", self.conn.cfg.origin()).bearer_auth(tok).send().await.map_err(|e| e.to_string())?;
        if !r.status().is_success() {
            return Err(format!("blob {sha}: {}", r.status()));
        }
        let mt = r.headers().get("content-type").and_then(|v| v.to_str().ok()).unwrap_or("image/png").to_string();
        let bytes = r.bytes().await.map_err(|e| e.to_string())?;
        Ok((mt, bytes.to_vec()))
    }

    // ---- dictation: a second socket per dictation (app/stt/serve.ts, app/ui/dictation.ts) ----------------------------
    /// Start dictating into `task`'s composer: the microphone (or OPTCHAT_FAKE_MIC's WAV) streams pcm16 16 kHz mono in
    /// ~100 ms chunks; chunks made before the socket is open wait and go out once it is.
    pub fn dictate_start(self: &Arc<Self>, task: i64) {
        if let Some(prev) = self.dict.lock().take() {
            prev.stop(); // a new dictation finishes the previous one (the hub does the same on its side)
        }
        let s = DictSession::start(self.clone(), task);
        *self.dict.lock() = Some(s);
    }
    /// stop listening and keep the text (the final text comes as Done)
    /// A second stop (the final text never came) or a stop with no session left ends it at once on screen.
    pub fn dictate_stop(&self) {
        let cur = self.dict.lock().clone();
        match cur {
            Some(s) if !s.stopped.load(Ordering::SeqCst) => {
                self.emit(HubEvent::Dictation { task: s.task, ev: DictEvent::State("finishing".into()) });
                s.stop();
            }
            Some(s) => {
                // already stopped and still waiting for its final text: give up waiting (what was said stays)
                s.stop();
                *self.dict.lock() = None;
                self.emit(HubEvent::Dictation { task: s.task, ev: DictEvent::State("idle".into()) });
            }
            None => self.emit(HubEvent::Dictation { task: self.current_task(), ev: DictEvent::State("idle".into()) }),
        }
    }
    fn dict_ended(&self, s: &Arc<DictSession>) {
        let mut g = self.dict.lock();
        if g.as_ref().map(|x| Arc::ptr_eq(x, s)).unwrap_or(false) {
            *g = None;
        }
    }
}

/// One dictation: the mic thread feeds a channel; a task owns the /stt socket.
pub struct DictSession {
    task: i64,
    tx: mpsc::UnboundedSender<WsMsg>,
    stopped: AtomicBool,
    mic: Mutex<Option<crate::mic::Mic>>,
    fake_stop: Arc<AtomicBool>,
}
impl DictSession {
    fn start(client: Arc<Client>, task: i64) -> Arc<DictSession> {
        let (tx, mut rx) = mpsc::unbounded_channel::<WsMsg>();
        let s = Arc::new(DictSession { task, tx, stopped: AtomicBool::new(false), mic: Mutex::new(None), fake_stop: Arc::new(AtomicBool::new(false)) });
        let emit = {
            let c = client.clone();
            move |ev: DictEvent| c.emit(HubEvent::Dictation { task, ev })
        };
        emit(DictEvent::State("connecting".into()));
        // the microphone starts at the press; its chunks queue in the channel until the socket is open
        let on_chunk: Arc<dyn Fn(crate::mic::MicChunk) + Send + Sync> = {
            let tx = s.tx.clone();
            let e = emit.clone();
            Arc::new(move |c: crate::mic::MicChunk| {
                e(DictEvent::Level((c.level.sqrt() * 2.0).min(1.0)));
                let _ = tx.send(WsMsg::binary(c.pcm));
            })
        };
        if let Ok(wav) = std::env::var("OPTCHAT_FAKE_MIC") {
            // tests: a spoken clip plays as the microphone, then silence until stopped
            let stop = s.fake_stop.clone();
            std::thread::spawn(move || {
                let data = crate::media::wav_pcm(&std::fs::read(&wav).unwrap_or_default());
                let mut k = 0;
                while !stop.load(Ordering::SeqCst) {
                    let chunk: Vec<u8> = if k < data.len() { data[k..(k + 3200).min(data.len())].to_vec() } else { vec![0u8; 3200] };
                    k += 3200;
                    let samples: Vec<i16> = chunk.chunks(2).map(|b| i16::from_le_bytes([b[0], *b.get(1).unwrap_or(&0)])).collect();
                    let rms = (samples.iter().map(|x| (*x as f32 / 32768.0).powi(2)).sum::<f32>() / samples.len().max(1) as f32).sqrt();
                    on_chunk(crate::mic::MicChunk { pcm: chunk, level: rms });
                    std::thread::sleep(Duration::from_millis(100));
                }
            });
        } else {
            match crate::mic::Mic::start(on_chunk) {
                Ok(m) => *s.mic.lock() = Some(m),
                Err(e) => {
                    emit(DictEvent::Error(format!("microphone failed: {e}")));
                    s.stopped.store(true, Ordering::SeqCst);
                }
            }
        }
        let me = s.clone();
        tokio::spawn(async move {
            let url = format!("{}/engine/stt?ws={}&task={task}", client.conn.cfg.ws_base(), client.ws);
            let tok = client.token.lock().clone();
            let ws = match client.conn.connect_with(&url, Some(&tok)).await {
                Ok(ws) => ws,
                Err(e) => {
                    emit(DictEvent::Error(format!("dictation could not connect: {e}")));
                    me.release();
                    emit(DictEvent::Done { text: String::new(), service: None, audio_ms: 0 });
                    client.dict_ended(&me);
                    return;
                }
            };
            let (mut w, mut r) = ws.split();
            let writer = tokio::spawn(async move {
                while let Some(m) = rx.recv().await {
                    if w.send(m).await.is_err() {
                        break;
                    }
                }
            });
            if me.stopped.load(Ordering::SeqCst) {
                let _ = me.tx.send(WsMsg::text(json!({ "type": "stop" }).to_string()));
            }
            let mut done = false;
            while let Ok(Some(msg)) = tokio::time::timeout(Duration::from_secs(120), r.next()).await {
                let Ok(WsMsg::Text(t)) = msg else {
                    if matches!(msg, Ok(WsMsg::Close(_)) | Err(_)) {
                        break;
                    }
                    continue;
                };
                let Ok(m) = serde_json::from_str::<Value>(t.as_str()) else { continue };
                match m["type"].as_str() {
                    Some("ready") if !me.stopped.load(Ordering::SeqCst) => emit(DictEvent::State("listening".into())),
                    Some("text") => emit(DictEvent::Text(m["text"].as_str().unwrap_or("").to_string())),
                    Some("error") => emit(DictEvent::Error(m["message"].as_str().unwrap_or("").to_string())),
                    Some("done") => {
                        done = true;
                        emit(DictEvent::Done { text: m["text"].as_str().unwrap_or("").to_string(), service: m["service"].as_str().map(String::from), audio_ms: m["audioMs"].as_u64().unwrap_or(0) });
                        break;
                    }
                    _ => {}
                }
            }
            writer.abort();
            me.release();
            if !done {
                emit(DictEvent::Done { text: String::new(), service: None, audio_ms: 0 });
            }
            client.dict_ended(&me);
        });
        s
    }
    fn release(&self) {
        self.fake_stop.store(true, Ordering::SeqCst);
        if let Some(m) = self.mic.lock().take() {
            m.stop();
        }
    }
    /// stop listening: the mic stops, the hub finishes and sends the final text
    pub fn stop(&self) {
        if self.stopped.swap(true, Ordering::SeqCst) {
            return;
        }
        self.release();
        let _ = self.tx.send(WsMsg::text(json!({ "type": "stop" }).to_string()));
        let _ = self.task;
    }
}

#[cfg(test)]
mod engine_token_tests {
    use super::*;
    #[::core::prelude::v1::test]
    fn parses_oc1() {
        use base64::Engine;
        let p = base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(br#"{"v":1,"urls":["https://tunnel.example","https://chat.reframe.dev"],"id":"ab12"}"#);
        let t = parse_engine_token(&format!("oc1.{p}.s3cret")).unwrap();
        assert_eq!(t.urls, vec!["https://tunnel.example".to_string(), "https://chat.reframe.dev".to_string()]);
        assert_eq!(t.id, "ab12");
        assert!(parse_engine_token("oct_plain").is_err());
        assert!(parse_engine_token(&format!("oc1.{p}.")).is_err());
    }
}
