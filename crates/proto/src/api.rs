//! What the app consumes, parsed from the hub's websocket messages (lib/hub.ts, app/ui/state.ts): the snapshot of
//! the task on screen, the events that follow, and the shapes of the rpc results the panels show.
use crate::metrics::Totals;
use crate::rates::Rate;
use crate::types::*;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::{BTreeMap, HashMap};
use std::sync::Arc;

fn i(v: &Value) -> i64 {
    v.as_i64().or_else(|| v.as_f64().map(|f| f.round() as i64)).unwrap_or(0)
}
fn s(v: &Value) -> String {
    v.as_str().unwrap_or("").to_string()
}
fn opt_s(v: &Value) -> Option<String> {
    v.as_str().map(String::from)
}
fn from<T: serde::de::DeserializeOwned>(v: &Value) -> Option<T> {
    serde_json::from_value(v.clone()).ok()
}

// ---- the task tree ------------------------------------------------------------------------------------------------
#[derive(Clone, Debug)]
pub struct TaskInfo {
    pub task: TaskRecord,
    pub running: bool,
    pub waiting: bool,
    pub queue: usize,
    pub metrics: Totals,
}
impl TaskInfo {
    /// `{task, running, waiting, queue, metrics?}` (snapshot.tasks[] and the "task" event)
    pub fn parse(v: &Value) -> Option<TaskInfo> {
        Some(TaskInfo {
            task: from(&v["task"])?,
            running: v["running"].as_bool().unwrap_or(false),
            waiting: v["waiting"].as_bool().unwrap_or(false),
            queue: i(&v["queue"]).max(0) as usize,
            metrics: from(&v["metrics"]).unwrap_or_default(),
        })
    }
}

#[derive(Clone, Debug, Default)]
pub struct ViewInfo {
    pub parts: Vec<ViewPart>, // without text
    pub bytes: usize,
    pub t: i64,
    pub g: i64,
    pub batching: bool,
    pub low: usize,
    pub high: usize,
    pub compaction_lines: usize,
    pub compaction_bytes: usize,
}
impl ViewInfo {
    pub fn parse(v: &Value) -> ViewInfo {
        ViewInfo {
            parts: from(&v["parts"]).unwrap_or_default(),
            bytes: i(&v["bytes"]).max(0) as usize,
            t: i(&v["T"]),
            g: i(&v["G"]),
            batching: v["batching"].as_bool().unwrap_or(false),
            low: i(&v["low"]).max(0) as usize,
            high: i(&v["high"]).max(0) as usize,
            compaction_lines: i(&v["compaction"]["lines"]).max(0) as usize,
            compaction_bytes: i(&v["compaction"]["bytes"]).max(0) as usize,
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ToolSummary {
    pub name: String,
    pub group: String,
    pub description: String,
}

#[derive(Clone, Debug)]
pub struct LiveSnap {
    pub request_id: String,
    pub turn_id: String,
    pub text: String,
    pub thinking: String,
}
#[derive(Clone, Debug)]
pub struct ToolSnap {
    pub call_id: String,
    pub turn_id: String,
    pub name: String,
    pub input: Value,
    pub started_at: String,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Approval {
    pub id: String,
    #[serde(default)]
    pub message: String,
    #[serde(default)]
    pub meta: Value,
    #[serde(default)]
    pub task: Option<i64>,
}

/// Everything a page needs to show one task (hub.snapshot), plus the global bits it carries.
#[derive(Clone, Debug)]
pub struct Snapshot {
    pub task: i64,
    pub version: String,
    pub build: String,
    pub instance: String,
    pub tasks: Vec<TaskInfo>,
    pub global: GlobalSettings,
    pub settings: Settings,
    pub models: Vec<ModelInfo>,
    pub tools: Vec<ToolSummary>,
    pub computer_available: bool,
    pub log: Vec<Arc<LogEntry>>,
    pub t: i64,
    pub g: i64,
    pub view: ViewInfo,
    pub live: Vec<LiveSnap>,
    pub running_tools: Vec<ToolSnap>,
    pub nodes_count: usize,
    pub busy: Vec<String>,
    pub failed: BTreeMap<String, String>,
    pub running: Option<TurnRecord>,
    pub queue: Vec<String>,
    pub approvals: Vec<Approval>,
    pub server_log: Vec<String>,
    /// filled by turns.recent / requests.recent after the snapshot (the browser's "turnsLoaded")
    pub turns: Vec<TurnRecord>,
    pub requests: Vec<RequestRecord>,
    /// the engine sent turns + requests with the snapshot (engine clients): no follow-up rpcs needed
    pub history: bool,
    /// unread notices by level
    pub notices: Unread,
    /// how every client shows the workspace (hub kv `ui:settings`); None: an engine from before ui.get / ui.set
    pub ui: Option<UiSettings>,
}
impl Snapshot {
    pub fn parse(m: &Value) -> Option<Snapshot> {
        let live = m["live"].as_object().map(|o| o.iter().map(|(k, v)| LiveSnap { request_id: k.clone(), turn_id: s(&v["turnId"]), text: s(&v["text"]), thinking: s(&v["thinking"]) }).collect()).unwrap_or_default();
        let running_tools = m["runningTools"]
            .as_array()
            .map(|a| a.iter().map(|t| ToolSnap { call_id: s(&t["callId"]), turn_id: s(&t["turnId"]), name: s(&t["name"]), input: t["input"].clone(), started_at: s(&t["startedAt"]) }).collect())
            .unwrap_or_default();
        Some(Snapshot {
            task: i(&m["task"]),
            version: s(&m["version"]),
            build: s(&m["build"]),
            instance: s(&m["instance"]),
            tasks: m["tasks"].as_array().map(|a| a.iter().filter_map(TaskInfo::parse).collect()).unwrap_or_default(),
            global: from(&m["global"]).unwrap_or_default(),
            settings: from(&m["settings"])?,
            models: from(&m["models"]).unwrap_or_default(),
            tools: from(&m["tools"]).unwrap_or_default(),
            computer_available: m["computerAvailable"].as_bool().unwrap_or(false),
            log: m["log"].as_array().map(|a| a.iter().filter_map(|e| from::<LogEntry>(e).map(Arc::new)).collect()).unwrap_or_default(),
            t: i(&m["T"]),
            g: i(&m["G"]),
            view: ViewInfo::parse(&m["view"]),
            live,
            running_tools,
            nodes_count: i(&m["nodesCount"]).max(0) as usize,
            busy: from(&m["compactor"]["busy"]).unwrap_or_default(),
            failed: from(&m["compactor"]["failed"]).unwrap_or_default(),
            running: from(&m["running"]),
            queue: from(&m["queue"]).unwrap_or_default(),
            approvals: from(&m["approvals"]).unwrap_or_default(),
            server_log: from(&m["serverLog"]).unwrap_or_default(),
            turns: m["turns"].as_array().map(|a| a.iter().filter_map(|t| from::<TurnRecord>(t)).collect()).unwrap_or_default(),
            requests: m["requests"].as_array().map(|a| a.iter().filter_map(|t| from::<RequestRecord>(t)).collect()).unwrap_or_default(),
            history: m.get("turns").is_some_and(|t| t.is_array()),
            notices: from(&m["notices"]).unwrap_or_default(),
            ui: m.get("ui").filter(|v| v.is_object()).map(UiSettings::parse),
        })
    }
}

/// How every client of a workspace shows it (hub kv `ui:settings`, rpc ui.get / ui.set, event "ui"): only booleans,
/// absent = off. Older engines don't have it: the client keeps a device-local choice instead.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct UiSettings {
    /// runs of internal messages (tool calls, child reports) start folded into one line
    pub collapse_internal: bool,
}
impl UiSettings {
    pub fn parse(v: &Value) -> UiSettings {
        UiSettings { collapse_internal: v["collapseInternal"].as_bool().unwrap_or(false) }
    }
}

// ---- events ---------------------------------------------------------------------------------------------------------
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ToolResult {
    pub content: Vec<ResultPart>,
    #[serde(rename = "isError", default)]
    pub is_error: bool,
}

#[derive(Clone, Debug)]
pub enum EngineEvent {
    Log { task: i64, entry: Arc<LogEntry> },
    Node { task: i64 },
    View { task: i64, view: ViewInfo },
    Compactor { busy: Vec<String>, failed: BTreeMap<String, String> },
    Turn { task: i64, turn: TurnRecord },
    Request { task: i64, request: RequestRecord },
    Delta { task: i64, turn_id: String, request_id: String, thinking: bool, text: String },
    Tool { task: i64, turn_id: String, call_id: String, name: String, input: Value, running: bool, result: Option<ToolResult>, started_at: Option<String> },
    Task { info: TaskInfo },
    Inbox { task: i64, queue: Vec<String> },
    Status { status: String, detail: Option<String> },
    Error { task: Option<i64>, message: String },
}

#[derive(Clone, Debug)]
pub enum DictEvent {
    State(String), // connecting | listening | finishing | idle
    Level(f32),
    Text(String),
    Error(String),
    Done { text: String, service: Option<String>, audio_ms: u64 },
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Conn {
    Connecting,
    Open,
    Closed,
}

#[derive(Clone, Debug)]
pub enum HubEvent {
    Engine(EngineEvent),
    ServerLog(String),
    Approval(Approval),
    ApprovalDone(String),
    Models(Vec<ModelInfo>),
    Global(GlobalSettings),
    /// the settings of the task on screen (and its tool list) after a settings / global change
    Settings { task: i64, settings: Settings, global: Option<GlobalSettings>, tools: Option<Vec<ToolSummary>> },
    Dictation { task: i64, ev: DictEvent },
    Notice { ok: bool, message: String, spawned: Option<i64> },
    /// the connection's state (the status bar shows "disconnected · reconnecting")
    Conn(Conn),
    /// a snapshot of the task on screen (on connect and after "open")
    Snapshot(Box<Snapshot>),
    Ready,
    /// the workspace answered `connect` (engine version / build)
    Hello { version: String, build: String },
    /// the workspace was disabled on the engine (or its token no longer covers it): it is disconnected
    Disabled(String),
    /// notices changed (new, counted again, read) and the unread counts per level
    Notices { changed: Vec<Notice>, unread: Unread },
    /// engine-level: its socket is up / down
    EngineOnline(bool),
    /// engine-level: why the socket could not connect (unreachable, token refused)
    EngineError(String),
    /// engine-level: its workspaces (name, running)
    EngineWorkspaces(Vec<WsEntry>),
    /// the engine refused or dropped this workspace's connection (no token that reaches it, token revoked): the
    /// connection stays listed, unsubscribed, until connected again with a token
    Refused(String),
    /// the workspace's display settings changed (another client or this one set them)
    Ui(UiSettings),
}

/// A workspace in an engine's public list: name, running, and whether it is the engine's default one.
#[derive(Clone, Debug, Default, serde::Serialize, serde::Deserialize, PartialEq)]
#[serde(default)]
pub struct WsEntry {
    pub name: String,
    pub running: bool,
    pub default: bool,
}

/// A notice (lib/notices.ts): something the user should know about a workspace, kept on the engine with its read state.
#[derive(Clone, Debug, Default, serde::Serialize, serde::Deserialize, PartialEq)]
#[serde(default)]
pub struct Notice {
    pub id: String,
    pub at: String,
    /// ok | info | warn | error
    pub level: String,
    pub task: Option<i64>,
    pub key: Option<String>,
    pub text: String,
    pub count: Option<i64>,
    #[serde(rename = "readAt")]
    pub read_at: Option<String>,
}

#[derive(Clone, Copy, Debug, Default, serde::Serialize, serde::Deserialize, PartialEq)]
#[serde(default)]
pub struct Unread {
    pub ok: i64,
    pub info: i64,
    pub warn: i64,
    pub error: i64,
}
impl Unread {
    pub fn total(&self) -> i64 {
        self.ok + self.info + self.warn + self.error
    }
}

/// One server message → the app's event (rpc replies and snapshots are handled by the client itself).
pub fn parse_event(m: &Value) -> Option<HubEvent> {
    let task = || i(&m["task"]);
    let ev = |e: EngineEvent| Some(HubEvent::Engine(e));
    match m["type"].as_str()? {
        "log" => ev(EngineEvent::Log { task: task(), entry: Arc::new(from(&m["entry"])?) }),
        "node" => ev(EngineEvent::Node { task: task() }),
        "view" => ev(EngineEvent::View { task: task(), view: ViewInfo::parse(m) }),
        "compactor" => ev(EngineEvent::Compactor { busy: from(&m["busy"]).unwrap_or_default(), failed: from(&m["failed"]).unwrap_or_default() }),
        "turn" => ev(EngineEvent::Turn { task: task(), turn: from(&m["turn"])? }),
        "request" => ev(EngineEvent::Request { task: task(), request: from(&m["request"])? }),
        "delta" => ev(EngineEvent::Delta { task: task(), turn_id: s(&m["turnId"]), request_id: s(&m["requestId"]), thinking: m["kind"] == "thinking", text: s(&m["text"]) }),
        "tool" => ev(EngineEvent::Tool {
            task: task(),
            turn_id: s(&m["turnId"]),
            call_id: s(&m["callId"]),
            name: s(&m["name"]),
            input: m["input"].clone(),
            running: m["state"] != "done",
            result: from(&m["result"]),
            started_at: opt_s(&m["startedAt"]),
        }),
        "task" => ev(EngineEvent::Task { info: TaskInfo::parse(m)? }),
        "inbox" => ev(EngineEvent::Inbox { task: task(), queue: from(&m["queue"]).unwrap_or_default() }),
        "status" => ev(EngineEvent::Status { status: s(&m["status"]), detail: opt_s(&m["detail"]) }),
        "error" => ev(EngineEvent::Error { task: m["task"].as_i64(), message: s(&m["message"]) }),
        "serverLog" => Some(HubEvent::ServerLog(s(&m["line"]))),
        "approval" => Some(HubEvent::Approval(Approval { id: s(&m["id"]), message: s(&m["message"]), meta: m["meta"].clone(), task: m["task"].as_i64() })),
        "approvalDone" => Some(HubEvent::ApprovalDone(s(&m["id"]))),
        "ui" => Some(HubEvent::Ui(UiSettings::parse(&m["ui"]))),
        "models" => Some(HubEvent::Models(from(&m["models"]).unwrap_or_default())),
        "settings" => Some(HubEvent::Settings { task: task(), settings: from(&m["settings"])?, global: from(&m["global"]), tools: from(&m["tools"]) }),
        "controlDone" => Some(HubEvent::Notice { ok: m["ok"].as_bool().unwrap_or(false), message: s(&m["message"]), spawned: m["spawned"].as_i64() }),
        _ => None,
    }
}

// ---- rpc results ----------------------------------------------------------------------------------------------------
/// The catalog as last seen from each provider (spec-metrics §3), the `model` table row.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct CatalogModel {
    pub id: String,
    pub provider: String,
    pub backend: String,
    pub label: String,
    pub efforts: Vec<String>,
    #[serde(rename = "defaultEffort")]
    pub default_effort: String,
    #[serde(rename = "computerUse")]
    pub computer_use: bool,
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub context: Option<i64>,
    #[serde(rename = "seenAt", default)]
    pub seen_at: String,
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub hidden: Option<bool>,
}

#[derive(Clone, Debug)]
pub struct ModelRow {
    pub cat: CatalogModel,
    pub available: bool,
    pub rate: Option<Rate>,
    pub history: Vec<Rate>,
    pub spend_usd: i64,
    pub spend_calls: i64,
    pub spend_tokens: i64,
}
impl ModelRow {
    /// a models.list row: {...CatalogModel, available, rate, history, spend: {usd (µ$), calls, tokens}}
    pub fn parse(v: &Value) -> Option<ModelRow> {
        Some(ModelRow {
            cat: from(v)?,
            available: v["available"].as_bool().unwrap_or(false),
            rate: from(&v["rate"]),
            history: from(&v["history"]).unwrap_or_default(),
            spend_usd: i(&v["spend"]["usd"]),
            spend_calls: i(&v["spend"]["calls"]),
            spend_tokens: i(&v["spend"]["tokens"]),
        })
    }
}

#[derive(Clone, Debug, Default)]
pub struct Breakdown {
    pub tasks: usize,
    pub events: usize,
    pub by_kind: Vec<(String, HashMap<String, i64>)>,
    pub by_model: Vec<(String, HashMap<String, i64>)>,
    pub by_source: Vec<(String, HashMap<String, i64>)>,
}
impl Breakdown {
    pub fn parse(v: &Value) -> Breakdown {
        let group = |g: &Value| -> Vec<(String, HashMap<String, i64>)> {
            g.as_object().map(|o| o.iter().map(|(k, m)| (k.clone(), m.as_object().map(|mm| mm.iter().map(|(a, b)| (a.clone(), i(b))).collect()).unwrap_or_default())).collect()).unwrap_or_default()
        };
        Breakdown { tasks: i(&v["tasks"]).max(0) as usize, events: i(&v["events"]).max(0) as usize, by_kind: group(&v["byKind"]), by_model: group(&v["byModel"]), by_source: group(&v["bySource"]) }
    }
}

#[derive(Clone, Debug)]
pub struct RequestDetail {
    pub request: RequestRecord,
    /// (context id, cache mark, message, envelope children)
    pub context: Vec<(String, bool, Message, Option<Vec<Message>>)>,
    pub output: Vec<Message>,
    pub wire: Option<Value>,
}
impl RequestDetail {
    /// request.get: {request, context: [{contextId, mark, message, children?}], outputChildren, wire}
    pub fn parse(v: &Value) -> Option<RequestDetail> {
        let context = v["context"]
            .as_array()
            .map(|a| a.iter().filter_map(|c| Some((s(&c["contextId"]), c["mark"].as_bool().unwrap_or(false), from::<Message>(&c["message"])?, from::<Vec<Message>>(&c["children"])))).collect())
            .unwrap_or_default();
        Some(RequestDetail { request: from(&v["request"])?, context, output: from(&v["outputChildren"]).unwrap_or_default(), wire: if v["wire"].is_null() { None } else { Some(v["wire"].clone()) } })
    }
}

#[derive(Clone, Debug)]
pub struct NodeKid {
    pub t: i64,
    pub l: u32,
    pub i: i64,
    pub node: Option<TreeNode>,
    pub message: Option<Arc<LogEntry>>,
}
/// node.children: {message} for a message, {children: [{t, l, i, node, message?}]} for a line
pub fn parse_node_children(v: &Value) -> (Option<Arc<LogEntry>>, Vec<NodeKid>) {
    let msg = from::<LogEntry>(&v["message"]).map(Arc::new);
    let kids = v["children"]
        .as_array()
        .map(|a| a.iter().map(|k| NodeKid { t: i(&k["t"]), l: i(&k["l"]).max(0) as u32, i: i(&k["i"]), node: from(&k["node"]), message: from::<LogEntry>(&k["message"]).map(Arc::new) }).collect())
        .unwrap_or_default();
    (msg, kids)
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct SearchHit {
    pub t: i64,
    pub i: i64,
    pub kind: String,
    pub text: String,
    pub date: String,
}

#[derive(Clone, Debug)]
pub struct Rendered {
    pub text: String,
    pub header: String,
    pub blocks: usize,
    pub mark_index: i64,
    pub marked_chars: usize,
}
impl Rendered {
    pub fn parse(v: &Value) -> Option<Rendered> {
        Some(Rendered { text: v["text"].as_str()?.to_string(), header: s(&v["header"]), blocks: i(&v["blocks"]).max(0) as usize, mark_index: i(&v["markIndex"]), marked_chars: i(&v["markedChars"]).max(0) as usize })
    }
}

/// An image to upload with a message (the composer downscales it first): sent as {mediaType, data (base64), name, w, h}.
#[derive(Clone, Debug)]
pub struct ImageIn {
    pub media_type: String,
    pub bytes: Vec<u8>,
    pub name: String,
    pub w: u32,
    pub h: u32,
}

/// Budget limits in human units (control op "budget" / spawn's budget).
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct BudgetArgs {
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub usd: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub minutes: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub tokens: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub deadline: Option<String>,
    #[serde(rename = "deadlineMinutes", skip_serializing_if = "Option::is_none", default)]
    pub deadline_minutes: Option<f64>,
}

/// The text of a tool result's parts (images as markers).
pub fn result_text(content: &[ResultPart]) -> String {
    content
        .iter()
        .map(|p| match p {
            ResultPart::Text { text } => text.clone(),
            ResultPart::Image { media_type, .. } => format!("[image {media_type}]"),
        })
        .collect::<Vec<_>>()
        .join("\n")
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    #[test]
    fn events_from_the_wire() {
        let e = parse_event(&json!({"type": "delta", "task": 3, "turnId": "t", "requestId": "r", "kind": "thinking", "text": "hm"})).unwrap();
        assert!(matches!(e, HubEvent::Engine(EngineEvent::Delta { task: 3, thinking: true, .. })));
        let e = parse_event(&json!({"type": "controlDone", "id": 1, "ok": true, "message": "#4 spawned", "spawned": 4})).unwrap();
        assert!(matches!(e, HubEvent::Notice { ok: true, spawned: Some(4), .. }));
        let e = parse_event(&json!({"type": "tool", "task": 0, "turnId": "t", "callId": "c", "name": "bash", "input": {"command": "ls"}, "state": "done", "result": {"content": [{"kind": "text", "text": "x"}], "isError": false}})).unwrap();
        match e {
            HubEvent::Engine(EngineEvent::Tool { running, result: Some(r), .. }) => {
                assert!(!running);
                assert_eq!(result_text(&r.content), "x");
            }
            other => panic!("{other:?}"),
        }
        let b = Breakdown::parse(&json!({"tasks": 2, "events": 5, "byKind": {"chat": {"usd": 10.0}}, "byModel": {}, "bySource": {}}));
        assert_eq!(b.by_kind[0].1["usd"], 10);
    }
}
