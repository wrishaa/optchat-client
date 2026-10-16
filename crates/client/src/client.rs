//! The app's connections to OptChat engines (spec-workspaces §2, §4) (app/ws/serve.ts, lib/hub.ts): ONE websocket, like the browser's page.
//! A snapshot of the task on screen arrives on connect and after every `open`; then the hub's events; commands go up
//! (send / cancel / stopTool / settings / global / control / spawn / approve) and rpc calls come back by id.
//! Reconnects with backoff (0.4 s doubling to 5 s) and resnapshots. Images come from GET /blob/<sha>; each dictation
//! is a second, short-lived socket to /stt. Every URL carries `db=t<id>` on a test instance.
//! TLS: the platform trust store through native-tls, verification always on. Linux builds also honor Fedora's
//! OpenSSL system crypto policy through the local native-tls patch.
use futures_util::{SinkExt, StreamExt};
use optchat_proto::api::*;
use optchat_proto::rates::Per1M;
use optchat_proto::types::*;
use parking_lot::Mutex;
use serde_json::{json, Value};
use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, AtomicI64, AtomicU64, Ordering};
use std::sync::Arc;
use std::time::Duration;
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
    if let Some(local) = ok.iter().find(|u| crate::tokens::is_local(u)) {
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
}

impl EngineConn {
    pub fn start(cfg: EngineCfg, socket_token: Option<String>, events: mpsc::UnboundedSender<Routed>) -> Arc<EngineConn> {
        let c = Arc::new(EngineConn {
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

    async fn run(self: Arc<Self>) {
        let mut retry: u32 = 0;
        let mut first = true;
        while !self.stopped.load(Ordering::SeqCst) {
            self.emit_all(|| HubEvent::Conn(Conn::Connecting));
            match self.connect_with(&format!("{}/engine/ws", self.cfg.ws_base()), self.socket_token.as_deref()).await {
                Ok(ws) => {
                    retry = 0;
                    if !first {
                        self.reconnects.fetch_add(1, Ordering::SeqCst);
                    }
                    first = false;
                    self.online.store(true, Ordering::SeqCst);
                    self.emit(None, HubEvent::EngineOnline(true));
                    self.emit(None, HubEvent::EngineError(String::new()));
                    let (mut w, mut r) = ws.split();
                    let (tx, mut rx) = mpsc::unbounded_channel::<WsMsg>();
                    // every connected workspace again: each answers hello + snapshot
                    for (ws, (task, token)) in self.subs.lock().iter() {
                        let _ = tx.send(WsMsg::text(json!({ "type": "connect", "ws": ws, "task": task, "token": token }).to_string()));
                    }
                    *self.out.lock() = Some(tx);
                    let writer = tokio::spawn(async move {
                        while let Some(m) = rx.recv().await {
                            if w.send(m).await.is_err() {
                                break;
                            }
                        }
                        let _ = w.close().await;
                    });
                    while let Some(msg) = r.next().await {
                        match msg {
                            Ok(WsMsg::Text(t)) => self.on_message(t.as_str()),
                            Ok(WsMsg::Close(_)) | Err(_) => break,
                            _ => {}
                        }
                    }
                    *self.out.lock() = None;
                    writer.abort();
                    for (_, tx) in self.rpcs.lock().drain() {
                        let _ = tx.send(Err("disconnected from the engine".into()));
                    }
                    for (_, tx) in self.controls.lock().drain() {
                        let _ = tx.send((false, "disconnected from the engine".into(), None));
                    }
                    self.online.store(false, Ordering::SeqCst);
                    self.emit(None, HubEvent::EngineOnline(false));
                    self.emit_all(|| HubEvent::Conn(Conn::Closed));
                }
                Err(e) => {
                    self.online.store(false, Ordering::SeqCst);
                    self.emit(None, HubEvent::EngineOnline(false));
                    self.emit_all(|| HubEvent::Conn(Conn::Closed));
                    self.emit(None, HubEvent::EngineError(e));
                }
            }
            if self.stopped.load(Ordering::SeqCst) {
                break;
            }
            // 0.4 s, doubling, at most 4 s: an engine that comes back is seen within ~5 s
            let ms = (400u64 << retry.min(4)).min(4000);
            retry += 1;
            tokio::time::sleep(Duration::from_millis(ms)).await;
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
