// Many finished tasks (spec: the phone hung on a workspace with thousands of done/canceled tasks): the phone layout
// against a REAL engine workspace with 3,600+ done/canceled tasks (a test engine, never the live one), headless.
//   OC_ENGINE=http://127.0.0.1:8833 OC_WS=perf OC_TOKEN_FILE=<local-client-token> OC_TAG=after deno run -A tests/done-tasks.ts
// Checks: launch timing, done tasks hidden by default (Tasks screen: All Tasks), "Show done" loads them a page at a
// time, opening a done task works, hiding again, live updates (a task spawned on the engine shows up).
// Screenshots: shots/done-<tag>-*.jpg. Prints one JSON line of numbers at the end.
import { App, check, failed, SHOTS } from "./lib/app.ts";
import { sleep } from "./lib/cua.ts";

const ENGINE = Deno.env.get("OC_ENGINE") ?? "http://127.0.0.1:8833";
const WS = Deno.env.get("OC_WS") ?? "perf";
const TAG = Deno.env.get("OC_TAG") ?? "after";
const token = Deno.readTextFileSync(Deno.env.get("OC_TOKEN_FILE")!).trim();
const dir = await Deno.makeTempDir({ prefix: "optchat-done-" });
Deno.writeTextFileSync(`${dir}/tokens.json`, JSON.stringify({ [WS]: token }));
const t0 = Date.now();
const a = await App.launch({ bare: true, headless: true, dir, env: { OPTCHAT_TEST_HOOKS: "1", OPTCHAT_TIMING: "1", OPTCHAT_ENGINE: ENGINE, OPTCHAT_ENGINE_NAME: "perf", OPTCHAT_CONNECT: WS, OPTCHAT_TEST_TOKENS: `${dir}/tokens.json` } });
const st = () => a.state() ?? {};
const tp = () => st().tasksPanel ?? {};
const out: Record<string, unknown> = { tag: TAG };
// OC_PAUSE=1: at each shot, wait for an outside screenshotter (Computer Use) instead of shot.sh, which needs the
// screen-recording permission: writes /tmp/oc-done-step (the name), waits for /tmp/oc-done-go (90 s at most)
const shot = async (name: string) => {
  if (!Deno.env.get("OC_PAUSE")) { const p = await a.shot(`done-${TAG}-${name}`); console.log("  shot", p); return; }
  await a.cmd({ op: "nudge" });
  await sleep(500);
  try { Deno.removeSync("/tmp/oc-done-go"); } catch { /* */ }
  Deno.writeTextFileSync("/tmp/oc-done-step", `done-${TAG}-${name}`);
  for (let i = 0; i < 900; i++) { try { Deno.statSync("/tmp/oc-done-go"); break; } catch { await sleep(100); } }
  try { Deno.removeSync("/tmp/oc-done-go"); } catch { /* */ }
  console.log("  shot", name);
};
try {
  await a.cmd({ op: "resize", w: 400, h: 860 });
  await a.waitFor(() => st().ready && st().loaded, 60000, "loaded");
  out.wallLoadedMs = Date.now() - t0;
  out.loadedMs = st().loadedMs;
  out.firstFrameMs = st().firstFrameMs;
  out.tasksInClient = st().tasks;
  await a.waitFor(() => st().narrow && !!a.probe("phone.chat"), 8000, "phone layout");
  await sleep(1000);
  out.rssAfterLoad = st().rss;
  await shot("01-chat");

  // the Tasks screen, All Tasks: the done ones hidden by default
  const tTasks = Date.now();
  await a.click("phone.tab.tasks");
  await a.waitFor(() => !!a.probe("trow.0"), 30000, "tasks screen");
  out.tasksScreenMs = Date.now() - tTasks;
  const tAll = Date.now();
  if (a.probe("phone.seg.all")) await a.click("phone.seg.all");
  else await a.clickAt(110, 160); // an older build: no probe on the segment
  await sleep(300);
  out.allTasksMs = Date.now() - tAll;
  // scrolling the list: each test command returns when the app has handled it (a hung main thread shows here)
  const tScroll = Date.now();
  for (let i = 0; i < 6; i++) await a.cmd({ op: "scroll", x: 200, y: 500, dy: i < 3 ? -1500 : 1500 });
  out.scroll6Ms = Date.now() - tScroll;
  await a.cmd({ op: "scroll", x: 200, y: 500, dy: 20000 });
  await sleep(500);
  const rows0 = Object.keys(a.allProbes()).filter((k) => /^trow\.\d+$/.test(k)).length;
  out.rowsDefault = tp().rows ?? rows0;
  out.framesTasks = st().frames;
  check(!!a.probe("phone.showDone") ? (tp().showDone === false) : true, `Tasks · All Tasks: done tasks hidden by default (rows ${out.rowsDefault}, tasks in the client ${st().tasks})`);
  await shot("02-tasks-default");

  if (a.probe("phone.showDone")) {
    const tShow = Date.now();
    await a.click("phone.showDone");
    await a.waitFor(() => tp().showDone === true && tp().endedLoading === false, 20000, "done page loaded");
    out.showDoneMs = Date.now() - tShow;
    await sleep(500);
    out.rowsShowDone = tp().rows;
    out.tasksAfterPage = tp().tasks;
    check((tp().rows ?? 0) > (out.rowsDefault as number), `Show done: ${tp().rows} rows (${tp().tasks} tasks loaded, more: ${tp().endedMore}) in ${out.showDoneMs} ms`);
    await shot("03-show-done");
    // the next page
    if (tp().endedMore) {
      for (let i = 0; i < 80; i++) {
        const p = a.probe("tasks.moreDone");
        if (p && p[1] > 120 && p[1] < 780) break;
        await a.cmd({ op: "scroll", x: 200, y: 500, dy: -2500 });
        await sleep(150);
      }
      out.moreButtonAt = a.probe("tasks.moreDone");
      await sleep(300);
      await shot("04-load-more-button");
      const before = tp().rows;
      await a.click("tasks.moreDone");
      await a.waitFor(() => tp().endedLoading === false && tp().rows > before, 20000, "second page");
      out.tasksAfterPage2 = tp().tasks;
      out.rowsPage2 = tp().rows;
      check(tp().rows > before, `Load older done tasks: ${before} -> ${tp().rows} rows (${tp().tasks} tasks in the client)`);
      await shot("05-page-2");
    }
    // open a done task from the list (a tap on its row): its chat loads; the tab stays after the next snapshot
    const done = Object.keys(a.allProbes()).filter((k) => /^trow\.\d+$/.test(k)).filter((k) => { const p = a.probe(k)!; return p[1] > 260 && p[1] < 780; }).map((k) => Number(k.slice(5))).filter((id) => st().ws?.taskStatus?.[String(id)] === "done").sort((x, y) => y - x)[0];
    if (done) {
      const tOpen = Date.now();
      await a.click(`trow.${done}`);
      await a.waitFor(() => st().current === done && st().loaded, 15000, `open done #${done}`);
      out.openDoneMs = Date.now() - tOpen;
      check(st().status === "done", `tapped done task #${done}: its chat loads in ${out.openDoneMs} ms (status ${st().status}, screen ${st().phone?.screen})`);
      await sleep(800);
      await shot("06-done-task-open");
      await a.cmd({ op: "open", task: 0 });
      await a.waitFor(() => st().current === 0 && st().loaded, 15000, "back to main");
      check((st().tabs ?? []).includes(done), `the done task's tab stays after the next snapshot (tabs ${JSON.stringify(st().tabs)})`);
    }
    // hide again
    if (st().phone?.screen !== "tasks") await a.click("phone.tab.tasks");
    await sleep(600);
    out.screenBeforeHide = st().phone?.screen;
    await a.click("phone.showDone");
    await a.waitFor(() => tp().showDone === false, 5000, "hidden again");
    await sleep(400);
    out.rowsHiddenAgain = tp().rows;
    check(tp().rows === out.rowsDefault || (tp().rows as number) <= (out.rowsDefault as number) + 1, `Show done off: back to ${tp().rows} rows`);
    await shot("07-hidden-again");
  }

  // live updates: a task spawned on the engine shows up in the list (never on a real workspace: OC_READONLY=1)
  if (Deno.env.get("OC_READONLY")) { out.rssEnd = st().rss; Deno.writeTextFileSync("/tmp/oc-done-step", "end"); throw "readonly"; }
  const ws = new WebSocket(ENGINE.replace(/^http/, "ws") + "/engine/ws", { headers: { Authorization: `Bearer ${token}`, Origin: ENGINE } } as never);
  await new Promise((r) => ws.onopen = r);
  ws.send(JSON.stringify({ type: "connect", ws: WS, task: 0, token, tasks: "live" }));
  await sleep(800);
  const spawned = new Promise<number>((r) => ws.onmessage = (e) => { const m = JSON.parse(e.data); if (m.type === "controlDone" && m.spawned) r(m.spawned); });
  ws.send(JSON.stringify({ type: "spawn", ws: WS, task: 0, id: 9, mode: "handoff", title: `live update check ${TAG}`, instruction: "seeded live check", start: false }));
  const id = await spawned;
  ws.close();
  await a.waitFor(() => !!a.probe(`trow.${id}`), 8000, `live: #${id} in the list`).catch(() => {});
  check(!!a.probe(`trow.${id}`), `live update: task #${id} spawned on the engine appears in Tasks`);
  out.liveSpawned = id;
  await shot("08-live-update");
  out.rssEnd = st().rss;
  Deno.writeTextFileSync("/tmp/oc-done-step", "end");
  out.frames = st().frames;
} catch (e) {
  if (e !== "readonly") check(false, `error: ${e}`);
} finally {
  try { out.stderrTiming = Deno.readTextFileSync(`${dir}/stderr.txt`).split("\n").filter((l) => /ms|snapshot|ended|tasks view/.test(l)).slice(-15); } catch { /* */ }
  console.log(JSON.stringify(out));
  await a.quit().catch(() => {});
}
Deno.exit(failed() ? 1 : 0);
