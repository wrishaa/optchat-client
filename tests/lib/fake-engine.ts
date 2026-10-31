// A fake OptChat engine on 127.0.0.1 for UI tests that need no real workspace (the Workspaces panel): the public list
// (GET /engine/workspaces, the first one marked default), and /engine/ws speaking just enough of the protocol (one
// socket, `connect {ws, task, token}` → hello + an empty main chat snapshot with unread notices, rpc answers,
// disconnect). Token "bad" is refused like the engine does (error "no token that reaches workspace …"); any other is
// let in. GET /test/revoke?ws=<name> drops that workspace from every socket ("disconnected", reason "token revoked").
// Nothing here is the real engine. Workspace names come from argv (never hard-coded in the app).
//   deno run -A tests/lib/fake-engine.ts <port> <name[:off]>...
// `--big`: a perf fixture instead of the one-line chat: a main chat of 1,770 entries (markdown answers with headings,
// lists, code blocks and tables, user messages, notes) and five child tasks of 60–200 entries, deterministic.
// `--folds`: a main chat with runs of internal messages (tool calls, child reports) between user and agent messages.
// Workspace display settings (snapshot `ui`, rpc ui.get / ui.set, event "ui"); `--no-ui` acts like an engine from
// before them (no `ui` in the snapshot, "unknown rpc ui.set").
const big = Deno.args.includes("--big");
const foldsFix = Deno.args.includes("--folds");
const noUi = Deno.args.includes("--no-ui");
const uiOf = new Map<string, { collapseInternal?: boolean }>();
const port = Number(Deno.args[0] ?? 18790);
const list = Deno.args.slice(1).filter((a) => !a.startsWith("--")).map((a, i) => ({ name: a.split(":")[0], running: !a.endsWith(":off"), ...(i === 0 ? { default: true } : {}) }));
const sockets = new Set<WebSocket>();
let sttHang = false;
// spec-workspaces §6 tokens: oc1.<b64url {v,urls,id}>.<secret>; this engine's oc1 tokens reach `scope`
// (--scope a,b; default every workspace), changed live by GET /test/scope?set=a,b
const scopeArg = Deno.args.find((a) => a.startsWith("--scope="));
let scope = new Set<string>(scopeArg ? scopeArg.slice(8).split(",").filter(Boolean) : list.map((w) => w.name));
const subsOf = new Map<WebSocket, Set<string>>();
const reaches = (token: string | undefined, ws: string) => !!token && token !== "bad" && (!token.startsWith("oc1.") || (token.endsWith(".s3cret") && scope.has(ws)));
const settings = { model: "claude-haiku-5-5", effort: "low", compactorModel: "claude-haiku-5-5", compactorEffort: "low", cwd: "/tmp", tools: { core: true, computer: false, mcp: false }, computerApproval: "ask", viewBudget: 128000, instructions: "", depth: 2 };
const now = new Date().toISOString();
const notices = (ws: string) => [
  { id: `${ws}-n1`, at: now, level: "warn", task: 0, key: "k1", text: `${ws}: a test warning from the fake engine`, count: 1, readAt: null },
  { id: `${ws}-n2`, at: now, level: "info", task: 0, key: "k2", text: `${ws}: settings: saved`, count: 1, readAt: null },
];
const read = new Set<string>();
const unread = (ws: string) => {
  const u = { ok: 0, info: 0, warn: 0, error: 0 } as Record<string, number>;
  for (const n of notices(ws)) if (!read.has(n.id)) u[n.level]++;
  return u;
};
const TASKS = big ? [0, 1, 2, 3, 4, 5] : foldsFix ? [0, 1, 2, 3] : [0];
const TITLES: Record<number, string> = foldsFix ? { 1: "scheduled jobs", 2: "heartbeat", 3: "deploy check" } : {};
const at = (min: number) => new Date(Date.parse(now) - (40 - min) * 60_000).toISOString();
const foldsLog = () => {
  const L: any[] = [];
  const add = (kind: string, text: string, min: number, turnId?: string, meta?: Record<string, unknown>) => L.push({ i: L.length, kind, size: text.length, date: at(min), text, ...(turnId ? { turnId } : {}), ...(meta ? { meta } : {}) });
  const tool = (turnId: string, min: number, name: string, input: unknown, out: string, err = false) => {
    const callId = `c${L.length}`;
    add("tool", `${name} ${JSON.stringify(input)}`, min, turnId, { name, callId });
    add("echo", out, min, turnId, { callId, ...(err ? { isError: true } : {}) });
  };
  add("user", "check on the scheduled jobs and the heartbeat, and run the deploy check", 0);
  add("talk", "On it: I'll spawn three tasks and look at the crontab myself.", 1, "t1");
  tool("t1", 1, "task_spawn", { mode: "handoff", title: "check scheduled jobs" }, "#1 spawned");
  tool("t1", 2, "task_spawn", { mode: "handoff", title: "watch the heartbeat" }, "#2 spawned");
  tool("t1", 2, "task_spawn", { mode: "handoff", title: "run the deploy check" }, "#3 spawned");
  tool("t1", 3, "bash", { command: "crontab -l" }, "*/5 * * * * /usr/local/bin/heartbeat\n0 3 * * * /usr/local/bin/backup");
  add("talk", "All three are running; I'll report back when they reply.", 4, "t1");
  add("task", "#2 status: started the heartbeat watch", 5);
  add("task", "#1 talk: found 6 scheduled jobs; all ran in the last 24 h", 8);
  add("task", "#2 talk: heartbeat ok (12 beats in 60 min)", 11);
  add("task", "#1 talk: backup job took 41 min, up from 12; flagged it", 14);
  add("task", "#3 error: deploy check failed: timeout after 300 s", 16);
  add("task", "#2 talk: heartbeat ok (24 beats)", 18);
  add("user", "what did they find?", 20);
  add("talk", "Here's where things stand:\n\n- **scheduled jobs**: 6 jobs, all ran; the backup got slower (41 min vs 12)\n- **heartbeat**: healthy\n- **deploy check**: timed out; I'll retry it", 21, "t2");
  tool("t2", 22, "task_tell", { id: 3, message: "retry with a 600 s timeout" }, "sent");
  add("talk", "Retrying the deploy check with a longer timeout.", 22, "t2");
  return L;
};
const foldsTurns = () => ["t1", "t2"].map((id, b) => ({ id, task: 0, startedBy: "user", backend: "claude-code", model: "claude-opus-5-5", effort: "medium", status: "done", userIndexes: [], requestIds: [], usage: { input: 48400, cacheRead: 46000, cacheWrite: 0, output: 305 }, cost: { input: 0.01, cacheRead: 0.02, cacheWrite: 0, output: 0.005, total: 0.035, priced: true }, startedAt: at(b ? 21 : 1), doneAt: at(b ? 22 : 4) }));
const SIZES: Record<number, number> = { 0: 1770, 1: 200, 2: 60, 3: 140, 4: 90, 5: 180 };
const para = (k: number) => `Paragraph ${k}: the summary tree keeps the view under budget while older turns fold into nodes; **bold** text, \`inline code\` and a [link](https://example.com/${k}).`;
const answer = (k: number) => [
  `## Step ${k}`, para(k), "", `- item one for ${k}`, `- item two with \`code\``, `- item three`, "",
  k % 3 === 0 ? "```rust\nfn main() {\n    let x = " + k + ";\n    println!(\"{}\", x * 2);\n}\n```" : para(k + 1),
  k % 5 === 0 ? "\n| a | b |\n|---|---|\n| " + k + " | " + (k * 2) + " |" : "",
].join("\n");
// blocks of 7: a user message, then a turn (t<task>-<block>): talk, a bash tool call + its output, talk…
const entry = (task: number, i: number) => {
  const turnId = `t${task}-${Math.floor(i / 7)}`;
  if (i % 7 === 0) { const text = `Question ${i} for task ${task}: what changes next?`; return { i, kind: "user", size: text.length, date: now, text }; }
  if (i % 7 === 3) { const text = `bash {"command":"ls -la /tmp/project-${i}"}`; return { i, kind: "tool", size: text.length, date: now, text, turnId, meta: { name: "bash", callId: `c${task}-${i}` } }; }
  if (i % 7 === 4) { const text = `total ${i}\ndrwxr-xr-x  5 user staff 160 src\n-rw-r--r--  1 user staff 2${i} Cargo.toml`; return { i, kind: "echo", size: text.length, date: now, text, turnId, meta: { callId: `c${task}-${i - 1}` } }; }
  const text = i % 13 === 0 ? `note ${i}` : answer(i);
  return { i, kind: i % 13 === 0 ? "note" : "talk", size: text.length, date: now, text, turnId };
};
const turnsOf = (task: number, n: number) => Array.from({ length: Math.ceil(n / 7) }, (_, b) => ({
  id: `t${task}-${b}`, task, startedBy: "user", backend: "claude-code", model: "claude-opus-5-5", effort: "medium", status: "done",
  userIndexes: [b * 7], requestIds: [], usage: { input: 48400 + b, cacheRead: 46000, cacheWrite: 0, output: 305 }, cost: { input: 0.01, cacheRead: 0.02, cacheWrite: 0, output: 0.005, total: 0.035, priced: true },
  startedAt: new Date(Date.parse(now) - 8800).toISOString(), doneAt: now,
}));
const logs: Record<number, any[]> = {};
for (const t of TASKS) logs[t] = foldsFix ? (t === 0 ? foldsLog() : [{ i: 0, kind: "talk", size: 20, date: now, text: `child ${t} at work.` }]) : big ? Array.from({ length: SIZES[t] }, (_, i) => entry(t, i)) : [{ i: 0, kind: "talk", size: 40, date: now, text: "Hello from the fake engine." }];
const info = (t: number) => ({ task: { id: t, parent: t === 0 ? null : 0, title: t === 0 ? "" : TITLES[t] ?? `child task ${t}`, mode: t === 0 ? "root" : "handoff", status: "idle", settings, createdAt: now, startedBy: "user" }, running: false, waiting: false, queue: 0, metrics: big ? { usd: { "*": { own: 51_390_000 / (t + 1), subtree: t === 0 ? 268_040_000 : 2_560_000 / t }, chat: { own: 0, subtree: t === 0 ? 51_390_000 : 0 }, compact: { own: 0, subtree: t === 0 ? 160_860_000 : 0 } }, "time.active": { "*": { own: 0, subtree: t === 0 ? 21_480_000 : 120_000 } }, "tokens.in": { "*": { own: 0, subtree: t === 0 ? 373_690_000 : 12_000 } } } : {} });
const snapshot = (ws: string, task0: number) => {
  const task = TASKS.includes(task0) ? task0 : 0;
  const log = logs[task];
  return {
  type: "snapshot", ws, version: "fake", build: "fake", task, instance: ws,
  tasks: TASKS.map(info),
  global: { maxTurns: 4, dictation: "claude" }, settings, models: [], tools: [], computerAvailable: false,
  log: log.slice(-120), T: log.length, G: log.length,
  view: { parts: [], bytes: 0, T: log.length, G: log.length, batching: false, low: 0, high: 0, compaction: { lines: 0, bytes: 0 } },
  nodesCount: 0, compactor: { busy: [], failed: {} }, running: null, queue: [], approvals: [], serverLog: [], turns: big ? turnsOf(task, log.length).slice(-40) : [], requests: [], notices: unread(ws),
  ...(foldsFix && task === 0 ? { turns: foldsTurns() } : {}),
  ...(noUi ? {} : { ui: uiOf.get(ws) ?? {} }),
  };
};

Deno.serve({ hostname: "127.0.0.1", port, onListen: () => console.log(`fake engine on ${port}`) }, (req) => {
  const url = new URL(req.url);
  if (url.pathname === "/engine/workspaces") return Response.json(list);
  if (url.pathname === "/test/scope") {
    const next = new Set((url.searchParams.get("set") ?? "").split(",").filter(Boolean));
    for (const [sock, subs] of subsOf) {
      for (const ws of [...subs]) if (!next.has(ws) && scope.has(ws)) { subs.delete(ws); sock.send(JSON.stringify({ type: "disconnected", ws, reason: "removed from token" })); }
    }
    scope = next;
    for (const sock of sockets) sock.send(JSON.stringify({ type: "workspaces", workspaces: list }));
    return new Response("ok");
  }
  if (url.pathname === "/test/revoke") {
    for (const s of sockets) s.send(JSON.stringify({ type: "disconnected", ws: url.searchParams.get("ws"), reason: "token revoked" }));
    return new Response("ok");
  }
  if (url.pathname === "/test/stt-hang") { sttHang = url.searchParams.get("on") === "1"; return new Response("ok"); }
  if (url.pathname === "/engine/stt") {
    // dictation: ready, then words while audio comes in, done on stop (a fixed sentence)
    const { socket, response } = Deno.upgradeWebSocket(req);
    const words = "ok let it finish and then ask the child to check the arc renderer".split(" ");
    let n = 0, chunks = 0;
    socket.onopen = () => socket.send(JSON.stringify({ type: "ready" }));
    socket.onmessage = (e) => {
      if (typeof e.data === "string") {
        const m = JSON.parse(e.data);
        if (m.type === "stop" && sttHang) return; // a final text that never comes
        if (m.type === "stop") { socket.send(JSON.stringify({ type: "done", text: words.slice(0, Math.max(n, 3)).join(" "), service: "fake", audioMs: chunks * 100 })); socket.close(); }
        return;
      }
      chunks++;
      if (chunks % 3 === 0 && n < words.length) { n++; socket.send(JSON.stringify({ type: "text", text: words.slice(0, n).join(" ") })); }
    };
    return response;
  }
  if (url.pathname !== "/engine/ws") return new Response("not found", { status: 404 });
  const { socket, response } = Deno.upgradeWebSocket(req);
  sockets.add(socket);
  subsOf.set(socket, new Set());
  socket.onclose = () => { sockets.delete(socket); subsOf.delete(socket); };
  const send = (m: unknown) => { try { socket.send(JSON.stringify(m)); } catch { /* closed */ } };
  socket.onopen = () => send({ type: "workspaces", workspaces: list });
  socket.onmessage = (e) => {
    const m = JSON.parse(String(e.data));
    const ws = m.ws;
    if (m.type === "connect" && !reaches(m.token, ws)) {
      send({ type: "error", ws, message: `no token that reaches workspace ${ws}` });
    } else if (m.type === "connect" || m.type === "open") {
      if (m.type === "connect") { subsOf.get(socket)?.add(ws); send({ type: "hello", ws, version: "fake", build: "fake", running: true }); }
      send(snapshot(ws, m.task ?? 0));
    } else if (m.type === "disconnect") send({ type: "disconnected", ws });
    else if (m.type === "rpc" && (m.method === "ui.get" || m.method === "ui.set")) {
      if (noUi) send({ type: "rpc", ws, id: m.id, error: `unknown rpc ${m.method}` });
      else {
        if (m.method === "ui.set" && typeof m.params?.patch?.collapseInternal === "boolean") {
          uiOf.set(ws, { ...uiOf.get(ws), collapseInternal: m.params.patch.collapseInternal });
          for (const [sock, subs] of subsOf) if (subs.has(ws)) sock.send(JSON.stringify({ type: "ui", ws, ui: uiOf.get(ws) }));
        }
        send({ type: "rpc", ws, id: m.id, result: uiOf.get(ws) ?? {} });
      }
    } else if (m.type === "rpc") send({ type: "rpc", ws, id: m.id, result: m.method === "notices" ? notices(ws).map((n) => ({ ...n, readAt: read.has(n.id) ? now : null })) : m.method === "log.range" ? (logs[m.params?.task ?? 0] ?? []).filter((e) => e.i >= (m.params?.from ?? 0) && e.i < (m.params?.to ?? 1e9)) : [] });
    else if (m.type === "notices.read") {
      for (const n of notices(ws)) if (m.all || (m.ids ?? []).includes(n.id) || n.level === m.level) read.add(n.id);
      send({ type: "notices", ws, notices: notices(ws).map((n) => ({ ...n, readAt: read.has(n.id) ? now : null })), unread: unread(ws) });
    } else if (m.id !== undefined) send({ type: "controlDone", ws, id: m.id, ok: true, message: "saved" });
  };
  return response;
});
