//! The app's model (app/ui/state.ts): a mirror of the hub's state for the task on screen plus the global bits,
//! fed by the hub's websocket messages (`HubEvent`s), and the commands the views send (on the client's runtime, never
//! on the UI thread).
use crate::client::{ws_key, Client, EngineCfg, EngineConn, Routed};
use gpui::{App, AppContext, Context, Entity, EventEmitter, Task};
use optchat_proto::api::*;
use optchat_proto::types::*;
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, HashMap, VecDeque};
use std::sync::Arc;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum View {
    Tasks,
    Memory,
    Context,
    Log,
    Models,
    Notifications,
    Settings,
    Workspaces,
}
impl View {
    pub const ALL: [View; 8] = [View::Tasks, View::Memory, View::Context, View::Log, View::Models, View::Notifications, View::Settings, View::Workspaces];
    pub fn label(&self) -> &'static str {
        match self {
            View::Tasks => "Tasks",
            View::Memory => "Memory",
            View::Context => "Context",
            View::Log => "Log",
            View::Models => "Models",
            View::Notifications => "Notifications",
            View::Settings => "Settings",
            View::Workspaces => "Workspaces",
        }
    }
    pub fn icon(&self) -> &'static str {
        match self {
            View::Tasks => "list-tree",
            View::Memory => "brain",
            View::Context => "scan-text",
            View::Log => "scroll-text",
            View::Models => "cpu",
            View::Notifications => "bell",
            View::Settings => "settings",
            View::Workspaces => "layers",
        }
    }
}

/// min ms between redraws caused by engine events (0: every batch at once); see `set_engine_redraw_gap`
static PUMP_MIN_MS: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);

/// coalesce engine-driven redraws (the tv: each frame is ~80 ms of its cpu + gpu, and a key press waits behind one)
pub fn set_engine_redraw_gap(ms: u64) {
    PUMP_MIN_MS.store(ms, std::sync::atomic::Ordering::Relaxed);
}

/// A connection: a workspace on an engine (its token is in the Keychain under the same key).
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct ConnPref {
    pub endpoint: String,
    pub ws: String,
}
impl ConnPref {
    pub fn key(&self) -> String {
        ws_key(&self.endpoint, &self.ws)
    }
}

/// what each workspace remembers (its tabs, the task on screen, the sidebar's view)
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(default)]
pub struct WsPrefs {
    pub view: View,
    pub side_open: bool,
    pub task_scope_all: bool,
    pub show_ended: bool,
    pub settings_tab_global: bool,
    pub tabs: Vec<i64>,
    pub current: i64,
}
impl Default for WsPrefs {
    fn default() -> Self {
        WsPrefs { view: View::Tasks, side_open: true, task_scope_all: false, show_ended: true, settings_tab_global: false, tabs: vec![0], current: 0 }
    }
}

/// the app's settings (Application Support/…/prefs.json): engines and connections, theme, zoom, window; the
/// workspace-level ones (tabs, view, …) are the active workspace's, the others' are in `ws`
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(default)]
pub struct Prefs {
    pub engines: Vec<EngineCfg>,
    pub connections: Vec<ConnPref>,
    pub active: Option<String>,
    pub ws: BTreeMap<String, WsPrefs>,
    /// window bounds [x, y, w, h] in points, restored at launch
    pub window: Option<[f32; 4]>,
    pub view: View,
    pub side_open: bool,
    pub side_w: f32,
    pub task_scope_all: bool,
    pub show_ended: bool,
    pub settings_tab_global: bool,
    pub theme: Option<String>,
    pub follow_system: bool,
    pub tabs: Vec<i64>,
    pub current: i64,
    pub font_size: f32,
}
impl Default for Prefs {
    fn default() -> Self {
        Prefs { engines: vec![], connections: vec![], active: None, ws: BTreeMap::new(), window: None, view: View::Tasks, side_open: true, side_w: 320.0, task_scope_all: false, show_ended: true, settings_tab_global: false, theme: None, follow_system: true, tabs: vec![0], current: 0, font_size: 14.0 }
    }
}

#[derive(Clone, Debug)]
pub struct ToolState {
    pub call_id: String,
    pub turn_id: String,
    pub name: String,
    pub input: serde_json::Value,
    pub running: bool,
    pub started_at: Option<String>,
    pub result: Option<ToolResult>,
}

#[derive(Clone, Debug, Default)]
pub struct Live {
    pub turn_id: String,
    pub text: String,
    pub thinking: String,
}

/// Everything about the task on screen (replaced on every snapshot).
#[derive(Clone, Default)]
pub struct Cur {
    pub loaded: bool,
    pub log: Vec<Arc<LogEntry>>,
    pub t: i64,
    pub g: i64,
    pub view: Option<ViewInfo>,
    pub nodes: usize,
    pub running: Option<TurnRecord>,
    pub queue: Vec<String>,
    pub turns: HashMap<String, TurnRecord>,
    pub requests: HashMap<String, RequestRecord>,
    pub live: BTreeMap<String, Live>,
    pub tools: HashMap<String, ToolState>,
    pub settings: Option<Settings>,
    pub tool_list: Vec<ToolSummary>,
}

#[derive(Clone, Debug, Default)]
pub struct Dict {
    pub task: i64,
    pub state: String, // idle | connecting | listening | finishing
    pub level: f32,
    pub text: Option<String>,
    pub done: Option<String>,
    pub seq: u64,
    /// dictations finished (done events), for UI tests
    pub done_count: u64,
    /// a dictation error shows in the composer for 6 s (engine errors stay, like the web app)
    pub error: Option<(String, std::time::Instant)>,
}

/// A connected workspace's state while another one is on screen (swapped with AppState's fields on a switch; its
/// events keep being applied to it, quietly).
pub struct WsData {
    pub hub: Option<Arc<Client>>,
    pub conn: Conn,
    pub version: String,
    pub build: String,
    pub instance: String,
    pub computer_available: bool,
    pub tasks: BTreeMap<i64, TaskInfo>,
    pub current: i64,
    pub cur: Cur,
    pub models: Vec<ModelInfo>,
    pub global: GlobalSettings,
    pub busy: Vec<String>,
    pub failed: BTreeMap<String, String>,
    pub approvals: Vec<Approval>,
    pub server_log: VecDeque<String>,
    pub errors: Vec<String>,
    pub follow: bool,
    pub selected_request: Option<String>,
    pub pinned_request: bool,
    pub notices: Vec<Notice>,
    pub notices_loaded: bool,
    pub wp: WsPrefs,
}
impl WsData {
    pub fn new(hub: Option<Arc<Client>>, wp: WsPrefs) -> WsData {
        WsData {
            hub,
            conn: Conn::Connecting,
            version: String::new(),
            build: String::new(),
            instance: String::new(),
            computer_available: false,
            tasks: BTreeMap::new(),
            current: wp.current,
            cur: Cur::default(),
            models: vec![],
            global: GlobalSettings::default(),
            busy: vec![],
            failed: BTreeMap::new(),
            approvals: vec![],
            server_log: VecDeque::new(),
            errors: vec![],
            follow: true,
            selected_request: None,
            pinned_request: false,
            notices: vec![],
            notices_loaded: false,
            wp,
        }
    }
}

/// What the app knows of every connected workspace, on screen or not (the workspace menu, the dots).
#[derive(Clone, Debug)]
pub struct WsMeta {
    pub endpoint: String,
    pub ws: String,
    pub unread: Unread,
    pub conn: Conn,
    pub disabled: Option<String>,
    /// no token for it (in the Keychain): connect again
    pub no_token: bool,
    /// the engine refused or dropped it (no token that reaches it, token revoked): connect again with a token
    pub refused: Option<String>,
}

/// An engine in the workspace menu: online (a socket or the public list answers), its workspaces.
#[derive(Clone, Debug)]
pub struct EngineView {
    pub cfg: EngineCfg,
    pub online: Option<bool>,
    pub workspaces: Vec<WsEntry>,
    pub error: Option<String>,
    /// token-scoped engines: workspaces the token does not reach (refused; tried again when the list changes)
    pub out_of_scope: std::collections::HashSet<String>,
}

pub struct AppState {
    /// the workspace on screen: `<endpoint>|<name>` (empty: none connected)
    pub active: String,
    pub bg: HashMap<String, WsData>,
    pub meta: BTreeMap<String, WsMeta>,
    pub engines: Vec<EngineView>,
    pub conns: Vec<Arc<EngineConn>>,
    pub notices: Vec<Notice>,
    pub notices_loaded: bool,
    /// applying a background workspace's event: no UI events, no timing
    quiet: bool,
    local_seq: u64,
    pub ws_switches: Vec<f64>,
    pub hub: Option<Arc<Client>>,
    /// the websocket's state (status bar: "disconnected · reconnecting")
    pub conn: Conn,
    pub version: String,
    pub build: String,
    pub instance: String,
    pub computer_available: bool,
    pub rt: tokio::runtime::Handle,
    pub error: Option<String>,
    pub tasks: BTreeMap<i64, TaskInfo>,
    pub current: i64,
    pub cur: Cur,
    pub models: Vec<ModelInfo>,
    pub global: GlobalSettings,
    pub busy: Vec<String>,
    pub failed: BTreeMap<String, String>,
    pub node_tick: u64,
    pub approvals: Vec<Approval>,
    pub server_log: VecDeque<String>,
    pub errors: Vec<String>,
    pub dict: Dict,
    pub prefs: Prefs,
    pub prefs_path: String,
    pub feed_rev: u64,  // bumps whenever feed rows may change
    pub open_seq: u64,  // task switches
    pub follow: bool,   // autoscroll
    pub selected_request: Option<String>,
    pub pinned_request: bool,
    pub stats: Stats,
    tx: Option<tokio::sync::mpsc::UnboundedSender<Routed>>,
    _pump: Option<Task<()>>,
    _poll: Option<Task<()>>,
    /// Add engine in progress (the token's addresses are being tried) and its last error
    pub adding_engine: bool,
    pub add_engine_error: Option<String>,
}

#[derive(Clone, Debug, Default)]
pub struct Stats {
    pub launch_ms: f64,
    pub first_frame_ms: Option<f64>,
    pub loaded_ms: Option<f64>,
    /// task switches: from load_task to the first render with the task's data, ms
    pub switch_t0: Option<std::time::Instant>,
    pub switches: Vec<f64>,
    /// the same switches until their turns and requests (two rpcs after the snapshot) are in, ms
    pub switches_full: Vec<f64>,
    pub events: u64,
}

pub enum StateEvent {
    Opened(i64),
    SpawnedOpen(i64),
    DictText,
    Appended,
    /// another workspace is on screen
    WsSwitched,
}
impl EventEmitter<StateEvent> for AppState {}

impl AppState {
    pub fn new(rt: tokio::runtime::Handle, prefs_path: String) -> AppState {
        let prefs: Prefs = std::fs::read_to_string(&prefs_path).ok().and_then(|s| serde_json::from_str(&s).ok()).unwrap_or_default();
        AppState {
            active: String::new(),
            bg: HashMap::new(),
            meta: BTreeMap::new(),
            engines: prefs.engines.iter().map(|c| EngineView { cfg: c.clone(), online: None, workspaces: vec![], error: None, out_of_scope: Default::default() }).collect(),
            conns: vec![],
            notices: vec![],
            notices_loaded: false,
            quiet: false,
            local_seq: 0,
            ws_switches: vec![],
            hub: None,
            conn: Conn::Connecting,
            version: String::new(),
            build: String::new(),
            instance: String::new(),
            computer_available: false,
            rt,
            error: None,
            tasks: BTreeMap::new(),
            current: prefs.current,
            cur: Cur::default(),
            models: vec![],
            global: GlobalSettings::default(),
            busy: vec![],
            failed: BTreeMap::new(),
            node_tick: 0,
            approvals: vec![],
            server_log: VecDeque::new(),
            errors: vec![],
            dict: Dict { state: "idle".into(), ..Default::default() },
            prefs,
            prefs_path,
            feed_rev: 0,
            open_seq: 0,
            follow: true,
            selected_request: None,
            pinned_request: false,
            stats: Stats::default(),
            tx: None,
            _pump: None,
            _poll: None,
            adding_engine: false,
            add_engine_error: None,
        }
    }
    fn wp(&self) -> WsPrefs {
        let p = &self.prefs;
        WsPrefs { view: p.view, side_open: p.side_open, task_scope_all: p.task_scope_all, show_ended: p.show_ended, settings_tab_global: p.settings_tab_global, tabs: p.tabs.clone(), current: p.current }
    }
    fn set_wp(&mut self, w: WsPrefs) {
        let p = &mut self.prefs;
        p.view = w.view;
        p.side_open = w.side_open;
        p.task_scope_all = w.task_scope_all;
        p.show_ended = w.show_ended;
        p.settings_tab_global = w.settings_tab_global;
        p.tabs = w.tabs;
        p.current = w.current;
    }
    pub fn save_prefs(&self) {
        let mut p = self.prefs.clone();
        if !self.active.is_empty() {
            p.ws.insert(self.active.clone(), self.wp());
        }
        for (k, d) in &self.bg {
            p.ws.insert(k.clone(), d.wp.clone());
        }
        if let Ok(s) = serde_json::to_string_pretty(&p) {
            let _ = std::fs::write(&self.prefs_path, s);
        }
    }

    /// Start the engines' connections from the prefs (a token per connection from the Keychain), and apply what
    /// they send: the active workspace's events, the background ones' quietly, batched per frame.
    pub fn start(&mut self, tx: tokio::sync::mpsc::UnboundedSender<Routed>, mut rx: tokio::sync::mpsc::UnboundedReceiver<Routed>, cx: &mut Context<Self>) {
        self.tx = Some(tx);
        // the local engine on this Mac connects by itself (its local client token reaches every workspace)
        // (tests: OPTCHAT_LOCAL_ENGINE names a fake local engine instead)
        let local_ep = std::env::var("OPTCHAT_LOCAL_ENGINE").unwrap_or_else(|_| "https://chat.reframe.dev".into());
        let local = local_ep.as_str();
        if crate::tokens::local_token(local).is_some() {
            match self.prefs.engines.iter_mut().find(|e| e.endpoint == local) {
                Some(e) => e.auto = true,
                None => self.prefs.engines.push(EngineCfg { endpoint: local.into(), name: "This Mac".into(), urls: vec![local.into()], auto: true }),
            }
            self.engines = self.prefs.engines.iter().map(|c| EngineView { cfg: c.clone(), online: None, workspaces: vec![], error: None, out_of_scope: Default::default() }).collect();
        }
        let active = self.prefs.active.clone();
        let wp_active = self.wp();
        for c in self.prefs.connections.clone() {
            let key = c.key();
            let wp = if Some(&key) == active.as_ref() { wp_active.clone() } else { self.prefs.ws.get(&key).cloned().unwrap_or_default() };
            let token = crate::tokens::get(&key).or_else(|| crate::tokens::local_token(&c.endpoint));
            self.attach_ws(&c.endpoint, &c.ws, wp, token, cx);
        }
        for ep in self.engines.iter().filter(|e| e.cfg.auto).map(|e| e.cfg.endpoint.clone()).collect::<Vec<_>>() {
            if self.engine_token(&ep).is_some() {
                self.conn_for(&ep);
            }
        }
        if let Some(a) = active.filter(|a| self.bg.contains_key(a)) {
            self.switch_ws(&a, cx);
        } else if let Some(first) = self.prefs.connections.first().map(|c| c.key()) {
            self.switch_ws(&first, cx);
        }
        self.stats.switch_t0 = Some(std::time::Instant::now());
        self.open_seq += 1;
        self._pump = Some(cx.spawn(async move |this, cx| {
            while let Some(first) = rx.recv().await {
                let mut batch = vec![first];
                while let Ok(e) = rx.try_recv() {
                    batch.push(e);
                    if batch.len() > 2000 {
                        break;
                    }
                }
                let ok = this.update(cx, |s, cx| {
                    for (endpoint, ws, e) in batch {
                        s.route(endpoint, ws, e, cx);
                    }
                    cx.notify();
                });
                if ok.is_err() {
                    break;
                }
                // a slow device (the tv) redraws for engine traffic at most this often: what arrives meanwhile is
                // applied as one batch. The first event after a quiet spell still shows at once.
                let gap = PUMP_MIN_MS.load(std::sync::atomic::Ordering::Relaxed);
                if gap > 0 {
                    crate::timer::Timer::after(std::time::Duration::from_millis(gap)).await;
                }
            }
        }));
        // engines without a socket: their public workspace list every 5 s (online / offline, what is available)
        self._poll = Some(cx.spawn(async move |this, cx| loop {
            let Ok((rt, cfgs)) = this.update(cx, |s, _| (s.rt.clone(), s.engines.iter().filter(|e| !s.conns.iter().any(|c| c.cfg.endpoint == e.cfg.endpoint && c.has_subs())).map(|e| e.cfg.clone()).collect::<Vec<_>>())) else { break };
            for cfg in cfgs {
                let c2 = cfg.clone();
                // a token-scoped engine is asked with its token: only the token's workspaces (no token: every public name)
                let tok = if c2.auto { crate::tokens::get(&engine_token_key(&c2.endpoint)) } else { None };
                let r = rt.spawn(async move { crate::client::list_workspaces_with(&c2, tok.as_deref()).await }).await.unwrap_or_else(|e| Err(e.to_string()));
                let _ = this.update(cx, |s, cx| {
                    if let Some(e) = s.engines.iter_mut().find(|e| e.cfg.endpoint == cfg.endpoint) {
                        match r {
                            Ok(list) => {
                                e.online = Some(true);
                                e.workspaces = list;
                                e.error = None;
                            }
                            Err(err) => {
                                e.online = Some(false);
                                e.error = Some(err);
                            }
                        }
                    }
                    cx.notify();
                });
            }
            crate::timer::Timer::after(std::time::Duration::from_secs(5)).await;
        }));
    }

    /// the engine's one socket (every connected workspace of it rides on it, each with its own token)
    fn conn_for(&mut self, endpoint: &str) -> Option<Arc<EngineConn>> {
        if let Some(c) = self.conns.iter().find(|c| c.cfg.endpoint == endpoint) {
            return Some(c.clone());
        }
        let cfg = self.engines.iter().find(|e| e.cfg.endpoint == endpoint)?.cfg.clone();
        let tx = self.tx.clone()?;
        let _g = self.rt.enter();
        // a token-scoped engine's socket opens with its token: the engine then lists only the token's workspaces
        // (the local engine's own token goes with each connect instead: a rotated file is picked up per workspace)
        let socket_token = if cfg.auto { crate::tokens::get(&engine_token_key(endpoint)) } else { None };
        let c = EngineConn::start(cfg, socket_token, tx);
        self.conns.push(c.clone());
        Some(c)
    }

    /// a connection as a background workspace (subscribed at once); without a token it stays listed, unconnected
    fn attach_ws(&mut self, endpoint: &str, ws: &str, wp: WsPrefs, token: Option<String>, _cx: &mut Context<Self>) {
        let key = ws_key(endpoint, ws);
        let hub = token.as_deref().and_then(|t| self.conn_for(endpoint).map(|c| Client::new(c, ws, wp.current, t)));
        self.meta.insert(key.clone(), WsMeta { endpoint: endpoint.into(), ws: ws.into(), unread: Unread::default(), conn: Conn::Connecting, disabled: None, no_token: hub.is_none(), refused: None });
        self.bg.insert(key, WsData::new(hub, wp));
    }

    /// spec-workspaces §6: add an engine with only a token (oc1: it carries the engine's addresses; the first that
    /// answers wins) and an optional name (else the host it connected through). The token goes to the Keychain;
    /// every workspace in its scope then connects by itself.
    pub fn add_engine_token(&mut self, name: &str, token: &str, cx: &mut Context<Self>) {
        let token = token.trim().to_string();
        let parsed = match crate::client::parse_engine_token(&token) {
            Ok(p) => p,
            Err(e) => {
                self.add_engine_error = Some(e);
                cx.notify();
                return;
            }
        };
        self.adding_engine = true;
        self.add_engine_error = None;
        cx.notify();
        let name = name.trim().to_string();
        let urls = parsed.urls.clone();
        let tok = token.clone();
        let (txr, rxr) = tokio::sync::oneshot::channel();
        self.rt.spawn(async move {
            let _ = txr.send(crate::client::pick_url(&urls, &tok).await);
        });
        cx.spawn(async move |this, cx| {
            let r = rxr.await.unwrap_or_else(|_| Err("cancelled".into()));
            let _ = this.update(cx, move |s, cx| {
            s.adding_engine = false;
            let url = match r {
                Ok(u) => u,
                Err(e) => {
                    s.add_engine_error = Some(e);
                    return;
                }
            };
            if let Some(e) = s.engines.iter().find(|e| e.cfg.endpoint == url) {
                // the same engine again: the new token replaces the old one
                let ep = e.cfg.endpoint.clone();
                s.remove_engine(&ep, cx);
            }
            if let Err(e) = crate::tokens::set(&engine_token_key(&url), &token) {
                s.add_engine_error = Some(e);
                return;
            }
            let host = url.split("://").nth(1).unwrap_or(&url).trim_end_matches('/').to_string();
            let cfg = EngineCfg { endpoint: url.clone(), name: if name.is_empty() { host } else { name.clone() }, urls: parsed.urls.clone(), auto: true };
            s.prefs.engines.push(cfg.clone());
            s.engines.push(EngineView { cfg, online: None, workspaces: vec![], error: None, out_of_scope: Default::default() });
            s.save_prefs();
            s.conn_for(&url);
            cx.notify();
            });
        })
        .detach();
    }

    /// the token a token-scoped engine connects its workspaces with: its own (Keychain), else the local engine's
    pub fn engine_token(&self, endpoint: &str) -> Option<String> {
        if !self.engines.iter().any(|e| e.cfg.endpoint == endpoint && e.cfg.auto) {
            return None;
        }
        crate::tokens::get(&engine_token_key(endpoint)).or_else(|| crate::tokens::local_token(endpoint))
    }

    /// a token-scoped engine's list: connect every running workspace not connected yet (the token's scope decides:
    /// refused ones are out of scope and stay hidden)
    fn auto_connect(&mut self, endpoint: &str, list: &[WsEntry], cx: &mut Context<Self>) {
        let Some(token) = self.engine_token(endpoint) else { return };
        if let Some(ev) = self.engines.iter_mut().find(|e| e.cfg.endpoint == endpoint) {
            ev.out_of_scope.clear(); // a changed list: try again (a token's scope may have grown)
        }
        for w in list.iter().filter(|w| w.running) {
            let key = ws_key(endpoint, &w.name);
            if self.meta.contains_key(&key) {
                continue;
            }
            let wp = self.prefs.ws.get(&key).cloned().unwrap_or_default();
            self.attach_ws(endpoint, &w.name, wp, Some(token.clone()), cx);
            if self.active.is_empty() || self.prefs.active.as_deref() == Some(key.as_str()) && self.active != key {
                self.switch_ws(&key, cx);
            }
        }
    }

    /// a token-scoped engine's workspace the token does not reach (refused, or removed from the token): gone
    fn drop_out_of_scope(&mut self, key: &str, cx: &mut Context<Self>) {
        let Some(m) = self.meta.get(key).cloned() else { return };
        if key == self.active {
            let next = self.meta.keys().find(|k| *k != key).cloned();
            match next {
                Some(n) => self.switch_ws(&n, cx),
                None => {
                    let mut d = WsData::new(None, WsPrefs::default());
                    self.swap_ws(&mut d);
                    self.active = String::new();
                    self.bg.insert(key.to_string(), d);
                    self.feed_rev += 1;
                    self.open_seq += 1;
                    cx.emit(StateEvent::WsSwitched);
                }
            }
        }
        if let Some(d) = self.bg.remove(key) {
            if let Some(h) = d.hub {
                h.disconnect();
            }
        }
        self.meta.remove(key);
        if let Some(ev) = self.engines.iter_mut().find(|e| e.cfg.endpoint == m.endpoint) {
            ev.out_of_scope.insert(m.ws.clone());
        }
        cx.notify();
    }

    /// Add an engine (endpoint, name); its workspaces show in the menu once its public list answers.
    pub fn add_engine(&mut self, endpoint: &str, name: &str, cx: &mut Context<Self>) -> Result<String, String> {
        let endpoint = EngineCfg::normalize(endpoint)?;
        if self.engines.iter().any(|e| e.cfg.endpoint == endpoint) {
            return Err(format!("{endpoint} is already added"));
        }
        let name = if name.trim().is_empty() { endpoint.split("://").nth(1).unwrap_or(&endpoint).to_string() } else { name.trim().to_string() };
        let cfg = EngineCfg { endpoint: endpoint.clone(), name, ..Default::default() };
        self.prefs.engines.push(cfg.clone());
        self.engines.push(EngineView { cfg, online: None, workspaces: vec![], error: None, out_of_scope: Default::default() });
        self.save_prefs();
        cx.notify();
        Ok(endpoint)
    }
    pub fn remove_engine(&mut self, endpoint: &str, cx: &mut Context<Self>) {
        for k in self.meta.values().filter(|m| m.endpoint == endpoint).map(|m| ws_key(&m.endpoint, &m.ws)).collect::<Vec<_>>() {
            self.disconnect_ws(&k, cx);
        }
        self.prefs.engines.retain(|e| e.endpoint != endpoint);
        self.engines.retain(|e| e.cfg.endpoint != endpoint);
        crate::tokens::delete(&engine_token_key(endpoint));
        self.conns.retain(|c| {
            let keep = c.cfg.endpoint != endpoint;
            if !keep {
                c.shutdown();
            }
            keep
        });
        self.save_prefs();
        cx.notify();
    }

    /// Connect a workspace of an engine with its client token (kept in the Keychain); it shows at once.
    /// `token` empty: the local engine's own client token (tokens::local_token), when there is one
    pub fn connect_ws(&mut self, endpoint: &str, ws: &str, token: &str, cx: &mut Context<Self>) -> Result<(), String> {
        let key = ws_key(endpoint, ws);
        let typed = token.trim().to_string();
        let token = if typed.is_empty() { crate::tokens::local_token(endpoint).ok_or_else(|| "a client token is needed".to_string())? } else { typed.clone() };
        if self.meta.contains_key(&key) {
            // reconnect with a new token (after a refusal)
            self.disconnect_ws(&key, cx);
        }
        if !typed.is_empty() {
            crate::tokens::set(&key, &typed)?;
        }
        let token = token.as_str();
        let c = ConnPref { endpoint: endpoint.into(), ws: ws.into() };
        if !self.prefs.connections.contains(&c) {
            self.prefs.connections.push(c);
        }
        let wp = self.prefs.ws.get(&key).cloned().unwrap_or_default();
        self.attach_ws(endpoint, ws, wp, Some(token.to_string()), cx);
        self.switch_ws(&key, cx);
        self.save_prefs();
        Ok(())
    }

    /// Stop showing a workspace: unsubscribe, forget its token; another connected one comes on screen.
    pub fn disconnect_ws(&mut self, key: &str, cx: &mut Context<Self>) {
        if key == self.active {
            let next = self.meta.keys().find(|k| *k != key).cloned();
            match next {
                Some(n) => self.switch_ws(&n, cx),
                None => {
                    // nothing else: an empty screen
                    let mut d = WsData::new(None, WsPrefs::default());
                    self.swap_ws(&mut d);
                    self.active = String::new();
                    self.bg.insert(key.to_string(), d);
                    self.feed_rev += 1;
                    self.open_seq += 1;
                    cx.emit(StateEvent::WsSwitched);
                }
            }
        }
        if let Some(d) = self.bg.remove(key) {
            if let Some(h) = d.hub {
                h.disconnect();
            }
        }
        self.meta.remove(key);
        self.prefs.connections.retain(|c| c.key() != key);
        self.prefs.ws.remove(key);
        crate::tokens::delete(key);
        // a socket with nothing left on it closes
        self.conns.retain(|c| {
            let keep = c.has_subs();
            if !keep {
                c.shutdown();
            }
            keep
        });
        self.save_prefs();
        cx.notify();
    }

    fn swap_ws(&mut self, d: &mut WsData) {
        self.swap_ws_keep(d, false)
    }
    fn swap_ws_keep(&mut self, d: &mut WsData, keep_panel: bool) {
        use std::mem::swap;
        swap(&mut self.hub, &mut d.hub);
        swap(&mut self.conn, &mut d.conn);
        swap(&mut self.version, &mut d.version);
        swap(&mut self.build, &mut d.build);
        swap(&mut self.instance, &mut d.instance);
        swap(&mut self.computer_available, &mut d.computer_available);
        swap(&mut self.tasks, &mut d.tasks);
        swap(&mut self.current, &mut d.current);
        swap(&mut self.cur, &mut d.cur);
        swap(&mut self.models, &mut d.models);
        swap(&mut self.global, &mut d.global);
        swap(&mut self.busy, &mut d.busy);
        swap(&mut self.failed, &mut d.failed);
        swap(&mut self.approvals, &mut d.approvals);
        swap(&mut self.server_log, &mut d.server_log);
        swap(&mut self.errors, &mut d.errors);
        swap(&mut self.follow, &mut d.follow);
        swap(&mut self.selected_request, &mut d.selected_request);
        swap(&mut self.pinned_request, &mut d.pinned_request);
        swap(&mut self.notices, &mut d.notices);
        swap(&mut self.notices_loaded, &mut d.notices_loaded);
        let mine = self.wp();
        // the Workspaces panel stays on screen across a switch (it is where the switch was made)
        let panel = self.prefs.view == View::Workspaces && self.prefs.side_open;
        let theirs = std::mem::replace(&mut d.wp, mine);
        self.set_wp(theirs);
        if panel && keep_panel {
            self.prefs.view = View::Workspaces;
            self.prefs.side_open = true;
        }
    }

    /// Show another connected workspace: its state is already here (kept live in the background), so this is a swap.
    pub fn switch_ws(&mut self, key: &str, cx: &mut Context<Self>) {
        if key == self.active {
            return;
        }
        let Some(mut d) = self.bg.remove(key) else { return };
        let t0 = std::time::Instant::now();
        self.swap_ws_keep(&mut d, true);
        let old = std::mem::replace(&mut self.active, key.to_string());
        if !old.is_empty() {
            self.bg.insert(old, d);
        }
        self.prefs.active = Some(key.to_string());
        self.feed_rev += 1;
        self.open_seq += 1;
        self.dict = Dict { state: "idle".into(), ..Default::default() };
        if !self.cur.loaded {
            self.stats.switch_t0 = Some(t0);
        }
        self.ws_switches.push(t0.elapsed().as_secs_f64() * 1000.0);
        self.save_prefs();
        cx.emit(StateEvent::WsSwitched);
        if self.cur.loaded {
            cx.emit(StateEvent::Opened(self.current));
        }
        cx.notify();
    }

    /// An event from an engine: engine-level ones update the menu; a workspace's go to it (on screen or not).
    fn route(&mut self, endpoint: String, ws: Option<String>, e: HubEvent, cx: &mut Context<Self>) {
        let Some(ws) = ws else {
            if let Some(ev) = self.engines.iter_mut().find(|x| x.cfg.endpoint == endpoint) {
                match e {
                    HubEvent::EngineOnline(_) => ev.online = Some(self.conns.iter().any(|c| c.cfg.endpoint == endpoint && c.online.load(std::sync::atomic::Ordering::SeqCst))),
                    HubEvent::EngineWorkspaces(list) => {
                        // a workspace enabled again: subscribe to it again
                        for w in &list {
                            let (name, running) = (&w.name, w.running);
                            let key = ws_key(&endpoint, name);
                            if running && self.meta.get(&key).is_some_and(|m| m.disabled.is_some()) {
                                if let Some(m) = self.meta.get_mut(&key) {
                                    m.disabled = None;
                                }
                                let h = if key == self.active { self.hub.clone() } else { self.bg.get(&key).and_then(|d| d.hub.clone()) };
                                if let Some(h) = h {
                                    h.resubscribe();
                                }
                            }
                        }
                        ev.workspaces = list.clone();
                        self.auto_connect(&endpoint, &list, cx);
                    }
                    HubEvent::EngineError(m) => ev.error = if m.is_empty() { None } else { Some(m) },
                    _ => {}
                }
            }
            return;
        };
        let key = ws_key(&endpoint, &ws);
        // a token-scoped engine: a workspace its token does not reach (or no longer: removed from the token) is gone
        if let HubEvent::Refused(_) = &e {
            if crate::tokens::get(&key).is_none() && self.engine_token(&endpoint).is_some() {
                let rotated = crate::tokens::local_token(&endpoint).filter(|t| {
                    let h = if key == self.active { self.hub.clone() } else { self.bg.get(&key).and_then(|d| d.hub.clone()) };
                    h.is_some_and(|h| !h.uses_token(t))
                });
                if rotated.is_none() {
                    self.drop_out_of_scope(&key, cx);
                    return;
                }
            }
        }
        // refused while on the local engine's token, which was rotated since: connect again with the new one
        if let HubEvent::Refused(_) = &e {
            if crate::tokens::get(&key).is_none() {
                if let Some(t) = crate::tokens::local_token(&endpoint) {
                    let h = if key == self.active { self.hub.clone() } else { self.bg.get(&key).and_then(|d| d.hub.clone()) };
                    if let Some(h) = h.filter(|h| !h.uses_token(&t)) {
                        h.retoken(&t);
                        return;
                    }
                }
            }
        }
        if let Some(m) = self.meta.get_mut(&key) {
            match &e {
                HubEvent::Notices { unread, .. } => m.unread = *unread,
                HubEvent::Snapshot(s) => m.unread = s.notices,
                HubEvent::Conn(c) => m.conn = *c,
                HubEvent::Disabled(r) => m.disabled = Some(r.clone()),
                HubEvent::Refused(r) => {
                    m.refused = Some(r.clone());
                    m.conn = Conn::Closed;
                }
                HubEvent::Hello { .. } => m.refused = None,
                _ => {}
            }
        } else {
            return; // a workspace disconnected meanwhile
        }
        if key == self.active {
            self.apply(e, cx);
        } else if let Some(mut d) = self.bg.remove(&key) {
            self.swap_ws(&mut d);
            let (rev, seq) = (self.feed_rev, self.open_seq);
            self.quiet = true;
            self.apply(e, cx);
            self.quiet = false;
            self.feed_rev = rev;
            self.open_seq = seq;
            self.swap_ws(&mut d);
            self.bg.insert(key, d);
        }
    }

    /// unread notices of the workspace on screen by level (the engine's, plus this app's own unread ones)
    pub fn unread(&self) -> Unread {
        let mut u = self.meta.get(&self.active).map(|m| m.unread).unwrap_or_default();
        for n in self.notices.iter().filter(|n| n.id.starts_with("local-") && n.read_at.is_none()) {
            match n.level.as_str() {
                "ok" => u.ok += 1,
                "error" => u.error += 1,
                "warn" => u.warn += 1,
                _ => u.info += 1,
            }
        }
        u
    }
    /// unread errors and warnings in workspaces not on screen (the dot on the workspace icon)
    pub fn background_alert(&self) -> bool {
        self.meta.iter().any(|(k, m)| *k != self.active && m.unread.error + m.unread.warn > 0)
    }
    /// something this app did (a failed command, a spawned task) as a notice of the workspace on screen
    pub fn local_notice(&mut self, level: &str, text: impl Into<String>) {
        self.local_seq += 1;
        let n = Notice { id: format!("local-{}", self.local_seq), at: chrono::Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Millis, true), level: level.into(), task: Some(self.current), key: None, text: text.into(), count: Some(1), read_at: None };
        self.notices.insert(0, n);
    }
    /// the notifications list from the engine (latest 500), merged with this app's own
    pub fn load_notices(&mut self, cx: &mut Context<Self>) {
        let key = self.active.clone();
        self.run(cx, |h| async move { h.notices(None, 500).await }, move |s, list, _| {
            if s.active != key {
                return;
            }
            let locals: Vec<Notice> = s.notices.drain(..).filter(|n| n.id.starts_with("local-")).collect();
            s.notices = list;
            s.notices.extend(locals);
            s.notices.sort_by(|a, b| b.at.cmp(&a.at));
            s.notices_loaded = true;
        });
    }
    /// mark read: these ids (local ones here, the others on the engine), or every one of a level, or all
    pub fn read_notices(&mut self, ids: Option<Vec<String>>, level: Option<String>, cx: &mut Context<Self>) {
        let now = chrono::Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Millis, true);
        for n in self.notices.iter_mut().filter(|n| n.read_at.is_none()) {
            let hit = match (&ids, &level) {
                (Some(i), _) => i.contains(&n.id),
                (None, Some(l)) => &n.level == l,
                (None, None) => true,
            };
            if hit && n.id.starts_with("local-") {
                n.read_at = Some(now.clone());
            }
        }
        let all = ids.is_none() && level.is_none();
        let remote: Option<Vec<String>> = ids.map(|i| i.into_iter().filter(|x| !x.starts_with("local-")).collect());
        if remote.as_ref().is_some_and(|r| r.is_empty()) {
            cx.notify();
            return;
        }
        if let Some(h) = &self.hub {
            h.read_notices(remote, level.as_deref(), all);
        }
        cx.notify();
    }

    pub fn hub(&self) -> Option<Arc<Client>> {
        self.hub.clone()
    }

    /// Run a hub call (a command or an rpc) on the client's runtime; `done` gets the result back on the UI thread.
    pub fn run<R: Send + 'static, F: std::future::Future<Output = R> + Send + 'static>(&self, cx: &mut Context<Self>, f: impl FnOnce(Arc<Client>) -> F + Send + 'static, done: impl FnOnce(&mut AppState, R, &mut Context<Self>) + 'static) {
        let Some(hub) = self.hub.clone() else { return };
        let (tx, rx) = tokio::sync::oneshot::channel();
        self.rt.spawn(async move {
            let r = f(hub).await;
            let _ = tx.send(r);
        });
        cx.spawn(async move |this, cx| {
            if let Ok(r) = rx.await {
                let _ = this.update(cx, |s, cx| {
                    done(s, r, cx);
                    cx.notify();
                });
            }
        })
        .detach();
    }

    /// A command whose outcome is a notice of the workspace (an error always; `ok_notice`: success too).
    pub fn command<F: std::future::Future<Output = Result<String, String>> + Send + 'static>(&self, cx: &mut Context<Self>, ok_notice: bool, f: impl FnOnce(Arc<Client>) -> F + Send + 'static) {
        let key = self.active.clone();
        self.run(cx, f, move |s, r, cx| {
            let (level, text) = match r {
                Ok(m) if ok_notice && !m.is_empty() => ("ok", m),
                Ok(_) => return,
                Err(e) => ("error", e),
            };
            s.notice_in(&key, level, text, cx);
        });
    }
    /// a local notice for workspace `key` (on screen or not)
    pub fn notice_in(&mut self, key: &str, level: &str, text: String, _cx: &mut Context<Self>) {
        if key == self.active {
            self.local_notice(level, text);
        } else if let Some(mut d) = self.bg.remove(key) {
            self.swap_ws(&mut d);
            self.local_notice(level, text);
            self.swap_ws(&mut d);
            self.bg.insert(key.to_string(), d);
        }
    }

    pub fn load_task(&mut self, id: i64, cx: &mut Context<Self>) {
        self.current = id;
        self.cur = Cur::default();
        self.open_seq += 1;
        self.feed_rev += 1;
        self.follow = true;
        self.pinned_request = false;
        self.selected_request = None;
        if !self.prefs.tabs.contains(&id) {
            self.prefs.tabs.push(id);
        }
        self.prefs.current = id;
        self.save_prefs();
        // the hub answers "open" with this task's snapshot (applied in `apply`)
        self.stats.switch_t0 = Some(std::time::Instant::now());
        if let Some(c) = &self.hub {
            c.open(id);
        }
        cx.notify();
    }

    /// older entries from the hub before the loaded ones (the page's olderLog); returns how many were added
    pub fn prepend_older(&mut self, task: i64, entries: Vec<LogEntry>) -> usize {
        if task != self.current {
            return 0;
        }
        let first = self.cur.log.first().map(|e| e.i).unwrap_or(i64::MAX);
        let mut older: Vec<Arc<LogEntry>> = entries.into_iter().filter(|e| e.i < first).map(Arc::new).collect();
        older.sort_by_key(|e| e.i);
        let n = older.len();
        if n > 0 {
            older.extend(self.cur.log.drain(..));
            self.cur.log = older;
            self.feed_rev += 1;
        }
        n
    }
    /// a full entry in place of its slim copy (the page's fullEntry)
    pub fn replace_entry(&mut self, task: i64, e: LogEntry) {
        if task != self.current {
            return;
        }
        if let Some(x) = self.cur.log.iter_mut().find(|x| x.i == e.i) {
            *x = Arc::new(e);
            self.feed_rev += 1;
        }
    }

    pub fn open_task(&mut self, id: i64, cx: &mut Context<Self>) {
        if id < 0 {
            return;
        }
        if id == self.current && self.cur.loaded {
            return;
        }
        self.load_task(id, cx);
    }
    pub fn close_tab(&mut self, id: i64, cx: &mut Context<Self>) {
        let k = self.prefs.tabs.iter().position(|t| *t == id).unwrap_or(0);
        self.prefs.tabs.retain(|t| *t != id);
        if self.prefs.tabs.is_empty() {
            self.prefs.tabs.push(0);
        }
        self.save_prefs();
        if id == self.current {
            let next = self.prefs.tabs[k.min(self.prefs.tabs.len() - 1)];
            self.load_task(next, cx);
        }
        cx.notify();
    }

    fn apply_snapshot(&mut self, s: Snapshot) {
        self.version = s.version.clone();
        self.build = s.build.clone();
        self.instance = s.instance.clone();
        self.computer_available = s.computer_available;
        self.global = s.global.clone();
        self.models = s.models.clone();
        self.busy = s.busy.clone();
        self.failed = s.failed.clone();
        self.approvals = s.approvals.clone();
        if self.server_log.is_empty() {
            self.server_log = s.server_log.iter().cloned().collect();
        }
        self.tasks = s.tasks.iter().cloned().map(|i| (i.task.id, i)).collect();
        let mut turns: HashMap<String, TurnRecord> = s.turns.into_iter().map(|t| (t.id.clone(), t)).collect();
        if let Some(r) = &s.running {
            turns.insert(r.id.clone(), r.clone());
        }
        let mut tools: HashMap<String, ToolState> = HashMap::new();
        for t in s.running_tools {
            tools.insert(t.call_id.clone(), ToolState { call_id: t.call_id, turn_id: t.turn_id, name: t.name, input: t.input, running: true, started_at: Some(t.started_at), result: None });
        }
        // tools that finished before a reconnect stay (the page keeps done tool states too)
        if self.cur.loaded {
            for (k, t) in self.cur.tools.drain() {
                if !t.running {
                    tools.entry(k).or_insert(t);
                }
            }
        }
        self.cur = Cur {
            loaded: true,
            log: s.log,
            t: s.t,
            g: s.g,
            view: Some(s.view),
            nodes: s.nodes_count,
            running: s.running,
            queue: s.queue,
            turns,
            requests: s.requests.into_iter().map(|r| (r.id.clone(), r)).collect(),
            live: s.live.into_iter().map(|l| (l.request_id, Live { turn_id: l.turn_id, text: l.text, thinking: l.thinking })).collect(),
            tools,
            settings: Some(s.settings),
            tool_list: s.tools,
        };
        self.feed_rev += 1;
    }

    fn apply(&mut self, e: HubEvent, cx: &mut Context<Self>) {
        self.stats.events += 1;
        let cur = self.current;
        match e {
            HubEvent::Engine(ev) => match ev {
                EngineEvent::Log { task, entry } => {
                    if task == cur && self.cur.loaded && !self.cur.log.iter().rev().take(4).any(|e| e.i == entry.i) {
                        self.cur.t = self.cur.t.max(entry.i + 1);
                        self.cur.log.push(entry);
                        self.feed_rev += 1;
                        if !self.quiet {
                            cx.emit(StateEvent::Appended);
                        }
                    }
                }
                EngineEvent::Node { task, .. } => {
                    if task == cur {
                        self.cur.nodes += 1;
                    }
                    self.node_tick += 1;
                }
                EngineEvent::View { task, view } => {
                    if task == cur {
                        self.cur.g = view.g;
                        self.cur.view = Some(view);
                    }
                }
                EngineEvent::Compactor { busy, failed } => {
                    self.busy = busy;
                    self.failed = failed;
                }
                EngineEvent::Turn { task, turn } => {
                    if task != cur {
                        return;
                    }
                    let active = turn.status == "waiting" || turn.status == "running";
                    if active {
                        self.cur.running = Some(turn.clone());
                    } else if self.cur.running.as_ref().map(|r| r.id == turn.id).unwrap_or(false) {
                        self.cur.running = None;
                        self.cur.live.retain(|_, v| v.turn_id != turn.id);
                    }
                    self.cur.turns.insert(turn.id.clone(), turn);
                    self.feed_rev += 1;
                }
                EngineEvent::Request { task, request } => {
                    if task != cur {
                        return;
                    }
                    if request.status != "running" {
                        self.cur.live.remove(&request.id);
                    }
                    self.cur.requests.insert(request.id.clone(), request);
                    self.feed_rev += 1;
                }
                EngineEvent::Delta { task, turn_id, request_id, thinking, text } => {
                    if task != cur {
                        return;
                    }
                    let l = self.cur.live.entry(request_id).or_insert_with(|| Live { turn_id, ..Default::default() });
                    if thinking {
                        l.thinking.push_str(&text);
                    } else {
                        l.text.push_str(&text);
                    }
                    self.feed_rev += 1;
                }
                EngineEvent::Tool { task, turn_id, call_id, name, input, running, result, started_at } => {
                    if task != cur {
                        return;
                    }
                    let prev = self.cur.tools.get(&call_id).and_then(|t| t.started_at.clone());
                    self.cur.tools.insert(call_id.clone(), ToolState { call_id, turn_id, name, input, running, started_at: started_at.or(prev), result });
                    self.feed_rev += 1;
                }
                EngineEvent::Task { info } => {
                    let id = info.task.id;
                    if id == cur {
                        self.cur.settings = Some(info.task.settings.clone());
                        self.feed_rev += 1;
                    }
                    self.tasks.insert(id, info);
                }
                EngineEvent::Inbox { task, queue } => {
                    if task == cur {
                        self.cur.queue = queue;
                    }
                }
                EngineEvent::Status { .. } => {}
                EngineEvent::Error { task, message } => {
                    let m = match task {
                        Some(t) => format!("#{t}: {message}"),
                        None => message,
                    };
                    self.errors.push(m);
                    if self.errors.len() > 5 {
                        self.errors.remove(0);
                    }
                }
            },
            HubEvent::ServerLog(l) => {
                self.server_log.push_back(l);
                while self.server_log.len() > 300 {
                    self.server_log.pop_front();
                }
            }
            HubEvent::Approval(a) => {
                self.approvals.retain(|x| x.id != a.id);
                self.approvals.push(a);
            }
            HubEvent::ApprovalDone(id) => self.approvals.retain(|x| x.id != id),
            HubEvent::Models(m) => self.models = m,
            HubEvent::Global(g) => self.global = g,
            HubEvent::Dictation { .. } if self.quiet => {}
            HubEvent::Dictation { task, ev } => {
                self.dict.task = task;
                match ev {
                    DictEvent::State(s) => self.dict.state = s,
                    DictEvent::Level(l) => self.dict.level = l,
                    DictEvent::Text(t) => {
                        self.dict.text = Some(t);
                        self.dict.seq += 1;
                        if !self.quiet {
                            cx.emit(StateEvent::DictText);
                        }
                    }
                    DictEvent::Error(m) => {
                        self.dict.error = Some((m, std::time::Instant::now()));
                        cx.spawn(async move |this, cx| {
                            crate::timer::Timer::after(std::time::Duration::from_millis(6050)).await;
                            let _ = this.update(cx, |_, cx| cx.notify());
                        })
                        .detach();
                    }
                    DictEvent::Done { text, .. } => {
                        self.dict.done_count += 1;
                        self.dict.state = "idle".into();
                        self.dict.level = 0.0;
                        self.dict.done = Some(text.clone());
                        self.dict.text = Some(text);
                        self.dict.seq += 1;
                        if !self.quiet {
                            cx.emit(StateEvent::DictText);
                        }
                    }
                }
            }
            HubEvent::Notice { ok, message, spawned } => {
                if !message.is_empty() {
                    self.local_notice(if ok { "ok" } else { "error" }, message);
                }
                if let Some(id) = spawned {
                    if !self.quiet {
                        cx.emit(StateEvent::SpawnedOpen(id));
                    }
                }
            }
            HubEvent::Refused(reason) => {
                self.conn = Conn::Closed;
                self.local_notice("error", format!("the engine refused this workspace's connection: {reason}"));
            }
            HubEvent::Ready | HubEvent::EngineOnline(_) | HubEvent::EngineWorkspaces(_) | HubEvent::EngineError(_) | HubEvent::Hello { .. } => {}
            HubEvent::Disabled(reason) => {
                self.conn = Conn::Closed;
                self.local_notice("warn", format!("this workspace was disconnected by the engine: {reason}"));
            }
            HubEvent::Notices { changed, .. } => {
                for n in changed {
                    match self.notices.iter_mut().find(|x| x.id == n.id) {
                        Some(x) => *x = n,
                        None => self.notices.insert(0, n),
                    }
                }
                self.notices.sort_by(|a, b| b.at.cmp(&a.at));
                self.notices.truncate(600);
            }
            HubEvent::Conn(c) => {
                self.conn = c;
                if c == Conn::Closed {
                    self.cur.live.clear();
                }
            }
            HubEvent::Settings { task, settings, global, tools } => {
                if task == cur {
                    self.cur.settings = Some(settings);
                    if let Some(t) = tools {
                        self.cur.tool_list = t;
                    }
                }
                if let Some(g) = global {
                    self.global = g;
                }
            }
            HubEvent::Snapshot(snap) => {
                // the server is the truth after a (re)connect or an open; another task's snapshot is a stale answer,
                // unless the task asked for is gone (the hub then shows the main chat)
                let mut cur = cur;
                if snap.task != cur {
                    if snap.tasks.iter().any(|t| t.task.id == cur) {
                        return;
                    }
                    cur = snap.task;
                    self.current = cur;
                    self.prefs.current = cur;
                }
                self.prefs.tabs.retain(|t| snap.tasks.iter().any(|x| x.task.id == *t));
                if !self.prefs.tabs.contains(&cur) {
                    self.prefs.tabs.push(cur);
                }
                let first = !self.cur.loaded;
                let history = snap.history;
                self.apply_snapshot(*snap);
                if self.quiet {
                    return;
                }
                if let Some(t0) = self.stats.switch_t0 {
                    if std::env::var("OPTCHAT_TIMING").is_ok() {
                        eprintln!("open #{cur}: snapshot applied at {:.1} ms", t0.elapsed().as_secs_f64() * 1000.0);
                    }
                    if self.stats.loaded_ms.is_none() {
                        self.stats.loaded_ms = crate::LAUNCH.get().map(|t| t.elapsed().as_secs_f64() * 1000.0);
                    }
                }
                if first {
                    cx.emit(StateEvent::Opened(cur));
                }
                if history {
                    // an engine client's snapshot carries the task's turns and requests
                    if let Some(t0) = self.stats.switch_t0 {
                        self.stats.switches_full.push(t0.elapsed().as_secs_f64() * 1000.0);
                    }
                    return;
                }
                // its turns and requests (the foot chips, the step tables), as the page loads them after a snapshot
                let seq = self.open_seq;
                let t0 = self.stats.switch_t0;
                self.run(cx, move |h| async move { (h.turns_recent(cur, 80).await, h.requests_recent(cur, false, 300).await) }, move |s, (turns, requests), _| {
                    if s.open_seq != seq || s.current != cur {
                        return;
                    }
                    if let Some(t0) = t0 {
                        let ms = t0.elapsed().as_secs_f64() * 1000.0;
                        if std::env::var("OPTCHAT_TIMING").is_ok() {
                            eprintln!("open #{cur}: turns + requests in at {ms:.1} ms");
                        }
                        s.stats.switches_full.push(ms);
                    }
                    for t in turns {
                        s.cur.turns.entry(t.id.clone()).or_insert(t);
                    }
                    for r in requests {
                        s.cur.requests.entry(r.id.clone()).or_insert(r);
                    }
                    s.feed_rev += 1;
                });
            }
        }
    }

    // ---- derived ---------------------------------------------------------------------------------------------
    pub fn info(&self) -> Option<&TaskInfo> {
        self.tasks.get(&self.current)
    }
    pub fn children_of(&self, id: i64) -> Vec<&TaskInfo> {
        self.tasks.values().filter(|x| x.task.parent == Some(id)).collect()
    }
    pub fn ancestors(&self, id: i64) -> Vec<&TaskInfo> {
        let mut out = vec![];
        let mut x = self.tasks.get(&id);
        while let Some(t) = x {
            match t.task.parent.and_then(|p| self.tasks.get(&p)) {
                Some(p) => {
                    out.insert(0, p);
                    x = Some(p);
                }
                None => break,
            }
        }
        out
    }
    pub fn model(&self, id: &str) -> Option<&ModelInfo> {
        self.models.iter().find(|m| m.id == id)
    }
    pub fn last_turn(&self) -> Option<&TurnRecord> {
        self.cur.turns.values().max_by(|a, b| a.started_at.cmp(&b.started_at))
    }
    pub fn latest_request(&self) -> Option<String> {
        self.cur.requests.values().filter(|r| r.purpose == "chat").max_by(|a, b| a.started_at.cmp(&b.started_at)).map(|r| r.id.clone())
    }
    pub fn shown_request(&self) -> Option<String> {
        if self.pinned_request {
            self.selected_request.clone()
        } else {
            self.latest_request()
        }
    }
}

pub fn sub(x: Option<&TaskInfo>, metric: &str) -> i64 {
    x.and_then(|i| i.metrics.get(metric)).and_then(|m| m.get("*")).map(|v| v.subtree).unwrap_or(0)
}
pub fn sub_k(x: Option<&TaskInfo>, metric: &str, kind: &str) -> i64 {
    x.and_then(|i| i.metrics.get(metric)).and_then(|m| m.get(kind)).map(|v| v.subtree).unwrap_or(0)
}
pub fn own(x: Option<&TaskInfo>, metric: &str) -> i64 {
    x.and_then(|i| i.metrics.get(metric)).and_then(|m| m.get("*")).map(|v| v.own).unwrap_or(0)
}
pub fn own_k(x: Option<&TaskInfo>, metric: &str, kind: &str) -> i64 {
    x.and_then(|i| i.metrics.get(metric)).and_then(|m| m.get(kind)).map(|v| v.own).unwrap_or(0)
}
pub fn task_name(t: &TaskRecord) -> String {
    if t.parent.is_none() { "main chat".into() } else { t.title.clone() }
}

pub fn new_state(cx: &mut App, rt: tokio::runtime::Handle, prefs_path: String) -> Entity<AppState> {
    cx.new(|_| AppState::new(rt, prefs_path))
}

/// the Keychain key of a token-scoped engine's token
pub fn engine_token_key(endpoint: &str) -> String {
    format!("engine|{endpoint}")
}
