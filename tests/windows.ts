// Native Windows smoke test, using only an isolated fake engine and temporary files.
// Tests app handlers and GPUI dispatch; does not claim native mouse/IME or visual QA.
// Build first: cargo build -p optchat-windows
// Run: deno run -A tests/windows.ts
// To test a package, set OPTCHAT_TEST_BINARY to its extracted/installed optchat.exe.
import { dirname, resolve } from "node:path";
import { fileURLToPath } from "node:url";

if (Deno.build.os !== "windows") throw new Error("This suite requires Windows");
const root = resolve(dirname(fileURLToPath(import.meta.url)), "..");
const appBinary = Deno.env.get("OPTCHAT_TEST_BINARY") ??
  `${root}/target/debug/optchat.exe`;
const dir = await Deno.makeTempDir({ prefix: "OptChat Windows Zoë smoke " });
const probes = `${dir}/probes.json`;
// Exercise a self-contained executable in a path with spaces and non-ASCII text.
const inPlace = Deno.env.get("OPTCHAT_TEST_IN_PLACE") === "1";
const isolatedBinary = inPlace ? appBinary : `${dir}/OptChat.exe`;
if (!inPlace) await Deno.copyFile(appBinary, isolatedBinary);
const listener = Deno.listen({ hostname: "127.0.0.1", port: 0 });
const port = (listener.addr as Deno.NetAddr).port;
listener.close();
const endpoint = `http://127.0.0.1:${port}`;
const sleep = (ms: number) => new Promise((r) => setTimeout(r, ms));
let app: Deno.ChildProcess | undefined;
let seq = 0;
let checks = 0;
const token = `oc1.${
  btoa(JSON.stringify({ v: 1, urls: [endpoint], id: "windows-smoke" }))
    .replaceAll("+", "-").replaceAll("/", "_").replace(/=+$/, "")
}.s3cret`;

function launch() {
  const inherited = Object.fromEntries(
    Object.entries(Deno.env.toObject()).filter(([key]) =>
      !key.toUpperCase().startsWith("OPTCHAT_") && key.toUpperCase() !== "PATH"
    ),
  );
  return new Deno.Command(isolatedBinary, {
    cwd: dir,
    clearEnv: true,
    env: {
      ...inherited,
      OPTCHAT_PREFS_DIR: dir,
      OPTCHAT_TOKEN_DIR: dir,
      OPTCHAT_LOCAL_TOKEN_FILE: `${dir}/no-local-token`,
      OPTCHAT_TEST_NO_GLOBAL: "1",
      OPTCHAT_TEST_HOOKS: "1",
      OPTCHAT_PROBES: probes,
      // App startup must not depend on Rust, MSVC, Deno, Git or CMake on PATH.
      PATH: `${Deno.env.get("SystemRoot")}\\System32`,
    },
    stdout: "null",
    stderr: "inherit",
  }).spawn();
}

async function quit() {
  await cmd({ op: "key", keys: "ctrl+q" }, false);
  const exited = await Promise.race([
    app!.status,
    sleep(8_000).then(() => undefined),
  ]);
  if (!exited?.success) throw new Error("Ctrl+Q did not exit cleanly");
  app = undefined;
  console.log("ok  Ctrl+Q exits cleanly");
  checks++;
}

async function waitFor(
  test: () => unknown | Promise<unknown>,
  label: string,
  timeout = 30_000,
) {
  const until = Date.now() + timeout;
  while (Date.now() < until) {
    if (await test()) {
      console.log(`ok  ${label}`);
      checks++;
      return;
    }
    await sleep(100);
  }
  const last = state();
  throw new Error(`Timed out: ${label}; state=${
    JSON.stringify({
      pid: last?.pid,
      frame: last?.frame,
      ready: last?.ready,
      loaded: last?.loaded,
      engines: last?.engines?.length,
      connected: last?.connected?.length,
      lastCmd: last?.lastCmd,
    })
  }`);
}
function state() {
  try {
    return JSON.parse(Deno.readTextFileSync(`${probes}.state.json`));
  } catch {
    return undefined;
  }
}
async function cmd(command: Record<string, unknown>, awaitAck = true) {
  const id = ++seq;
  await Deno.writeTextFile(
    `${probes}.cmd.tmp`,
    JSON.stringify([{ ...command, seq: id }]),
  );
  await Deno.rename(`${probes}.cmd.tmp`, `${probes}.cmd`);
  if (awaitAck) {
    await waitFor(() => state()?.lastCmd?.seq === id, `command ${command.op}`);
  }
}
const engine = new Deno.Command(Deno.execPath(), {
  args: [
    "run",
    "--allow-net=127.0.0.1",
    `${root}/tests/lib/fake-engine.ts`,
    String(port),
    "alpha",
    "beta",
  ],
  stdout: "null",
  stderr: "inherit",
}).spawn();

try {
  await waitFor(async () => {
    try {
      return (await fetch(`${endpoint}/engine/workspaces`)).ok;
    } catch {
      return false;
    }
  }, "fake engine listening");
  app = launch();
  await waitFor(
    () => state()?.frame > 0 && state()?.engines?.length === 0,
    "empty first-run profile launches without developer tools on PATH",
    60_000,
  );
  await cmd({ op: "addEngine", token, name: "Windows smoke test" });
  await waitFor(
    () => state()?.ready && state()?.loaded,
    "native window connected and chat loaded",
    60_000,
  );
  await waitFor(
    () => state()?.lastTalk === "Hello from the fake engine.",
    "fake chat rendered",
  );
  await waitFor(
    () => state()?.connected?.length === 2 && state()?.engineSockets === 1,
    "two workspaces share one engine socket",
  );
  await waitFor(
    () => state()?.rss > 0 && state()?.frame > 0,
    "Windows memory probe and rendered frames",
  );
  if (Deno.readTextFileSync(`${dir}/prefs.json`).includes(token)) {
    throw new Error("Token leaked into preferences");
  }
  console.log("ok  test tokens stay out of preferences");
  checks++;
  await cmd({ op: "key", keys: "ctrl+l" });
  await cmd({ op: "type", text: "Windows Zoë বাংলা\nsecond line" });
  await waitFor(
    () => state()?.draft === "Windows Zoë বাংলা\nsecond line",
    "composer receives Unicode and multiline text",
  );
  await cmd({ op: "key", keys: "ctrl+Home" });
  await cmd({ op: "type", text: "start " });
  await cmd({ op: "key", keys: "ctrl+End" });
  await cmd({ op: "type", text: " end" });
  await waitFor(
    () => state()?.draft === "start Windows Zoë বাংলা\nsecond line end",
    "Ctrl+Home/End navigate the entire composer",
  );
  await cmd({ op: "key", keys: "ctrl+a" });
  await cmd({ op: "key", keys: "Backspace" });
  await waitFor(
    () => state()?.draft === "",
    "Ctrl+A selects all in the composer",
  );
  await cmd({ op: "key", keys: "ctrl+k" });
  await waitFor(
    () => state()?.overlay === "palette",
    "Ctrl+K opens the command palette",
  );
  await cmd({ op: "key", keys: "Escape" });
  await waitFor(() => !state()?.overlay, "Escape dismisses the palette");
  await cmd({ op: "switch", ws: "beta" });
  await waitFor(
    () => state()?.workspace === "beta" && state()?.loaded,
    "switch to beta",
  );
  await cmd({ op: "switch", ws: "alpha" });
  await waitFor(
    () => state()?.workspace === "alpha" && state()?.loaded,
    "switch back to alpha",
  );
  await cmd({ op: "act", name: "t-drop" });
  await waitFor(
    () => state()?.reconnects > 0 && state()?.ready && state()?.loaded,
    "engine reconnect restores the chat",
  );
  await cmd({ op: "loadNotices" });
  await waitFor(() => state()?.noticesTop?.length === 2, "engine notices load");
  await cmd({ op: "readAll" });
  await waitFor(
    () => state()?.noticesTop?.every((n: { read: boolean }) => n.read),
    "marking notices read reaches the engine",
  );
  await quit();
  await Deno.remove(`${probes}.state.json`);
  app = launch();
  await waitFor(
    () => state()?.ready && state()?.loaded && state()?.connected?.length === 2,
    "relaunch restores connections using saved preferences and isolated test tokens",
    60_000,
  );
  await quit();
  console.log(`${checks} Windows smoke checks passed`);
} finally {
  if (app) {
    try {
      app.kill("SIGTERM");
    } catch { /* already exited */ }
    await app.status;
  }
  try {
    engine.kill("SIGTERM");
  } catch { /* already exited */ }
  await engine.status;
  await Deno.remove(dir, { recursive: true });
}
