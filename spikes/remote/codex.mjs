// P0 spike (docs/plans/phone.md): drive `codex app-server` the way
// pansy-remote would. Starts a thread, answers a command approval, steers
// and interrupts a turn, then resumes the thread in a fresh process.
//
// Runs in a scratch CODEX_HOME and folder, pointed at pansy's gateway as
// `pansy run codex` points it (crates/pansy-agents/agents/codex.toml), so
// none of the user's Codex files are read or written.
//
//   node spikes/remote/codex.mjs [model]      (default: default/gpt-6.1-terra)

import { spawn, execFileSync } from "node:child_process";
import { mkdtempSync, mkdirSync, writeFileSync, existsSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";

const model = process.argv[2] ?? "default/gpt-6.1-terra";
const gateway = process.env.PANSY_GATEWAY ?? "http://127.0.0.1:7267/v1";
const scratch = mkdtempSync(join(tmpdir(), "pansy-p0-codex-"));
const home = join(scratch, "codex-home");
const work = join(scratch, "work");
mkdirSync(home);
mkdirSync(work);
writeFileSync(join(work, "README.md"), "# scratch\n");
execFileSync("git", ["init", "-q"], { cwd: work });
execFileSync("git", ["-c", "user.email=p0@pansy", "-c", "user.name=p0", "commit", "-qam", "init", "--allow-empty"], { cwd: work });
execFileSync("git", ["add", "."], { cwd: work });

const args = [
  "app-server",
  "-c", `model=${JSON.stringify(model)}`,
  "-c", 'model_provider="pansy"',
  "-c", 'model_providers.pansy.name="Pansy"',
  "-c", `model_providers.pansy.base_url=${JSON.stringify(gateway)}`,
  "-c", 'model_providers.pansy.wire_api="responses"',
  "-c", 'model_providers.pansy.experimental_bearer_token="pansy-p0"',
];

const log = (...a) => console.log(new Date().toISOString().slice(11, 23), ...a);

function server() {
  const p = spawn("codex", args, { env: { ...process.env, CODEX_HOME: home }, stdio: ["pipe", "pipe", "pipe"] });
  let next = 1, buf = "";
  const pending = new Map();
  const handlers = [];
  p.stderr.on("data", d => { const s = String(d).trim(); if (s) log("stderr", s.slice(0, 300)); });
  p.stdout.on("data", d => {
    buf += d;
    let i;
    while ((i = buf.indexOf("\n")) >= 0) {
      const line = buf.slice(0, i); buf = buf.slice(i + 1);
      if (!line.trim()) continue;
      const m = JSON.parse(line);
      if (m.id !== undefined && (m.result !== undefined || m.error !== undefined) && pending.has(m.id)) {
        const { ok, no } = pending.get(m.id); pending.delete(m.id);
        m.error ? no(new Error(JSON.stringify(m.error))) : ok(m.result);
      } else for (const h of handlers) h(m);
    }
  });
  const send = m => p.stdin.write(JSON.stringify(m) + "\n");
  return {
    p,
    request: (method, params) => new Promise((ok, no) => { const id = next++; pending.set(id, { ok, no }); send({ id, method, params }); }),
    notify: (method, params) => send({ method, params }),
    respond: (id, result) => send({ id, result }),
    on: h => handlers.push(h),
    stop: () => p.kill(),
  };
}

const text = t => [{ type: "text", text: t, text_elements: [] }];
const seen = new Set();
const results = {};

function watch(s, until) {
  return new Promise(done => {
    s.on(m => {
      if (!m.method) return;
      if (!seen.has(m.method)) { seen.add(m.method); log("first", m.method); }
      if (m.method === "item/agentMessage/delta") process.stdout.write(m.params.delta ?? "");
      if (m.method.endsWith("/requestApproval")) {
        log("APPROVAL", m.method, JSON.stringify(m.params).slice(0, 400));
        results.approval = m.method;
        s.respond(m.id, { decision: "accept" });
      }
      if (until(m)) done(m);
    });
  });
}

async function main() {
  log("scratch", scratch, "model", model);
  const s = server();
  const init = await s.request("initialize", { clientInfo: { name: "pansy-p0", title: "pansy P0", version: "0.0.0" }, capabilities: { experimentalApi: true, requestAttestation: false } });
  log("initialize", JSON.stringify(init).slice(0, 200));
  s.notify("initialized", {});

  const started = await s.request("thread/start", { cwd: work, approvalPolicy: "untrusted", sandbox: "workspace-write" });
  const threadId = started.thread.id;
  log("thread", threadId, "model", started.model, "provider", started.modelProvider);

  // 1. A turn that needs a command approved.
  let completed = watch(s, m => m.method === "turn/completed");
  const t1 = await s.request("turn/start", { threadId, input: text("Run the shell command `touch approved.txt` in the current folder, then reply with the single word done.") });
  log("turn 1", t1.turn.id);
  const c1 = await completed;
  console.log();
  log("turn 1 completed:", c1.params.turn.status, "approved.txt exists:", existsSync(join(work, "approved.txt")));
  results.turn1 = c1.params.turn.status;
  results.approvedFile = existsSync(join(work, "approved.txt"));

  // 2. Steer, then interrupt, a long turn.
  completed = watch(s, m => m.method === "turn/completed");
  const t2 = await s.request("turn/start", { threadId, input: text("Count slowly from 1 to 200, one number per line, with no other text.") });
  log("turn 2", t2.turn.id);
  await new Promise(r => setTimeout(r, 2500));
  try {
    const st = await s.request("turn/steer", { threadId, expectedTurnId: t2.turn.id, input: text("Actually stop at 20.") });
    log("steer ok", JSON.stringify(st).slice(0, 200)); results.steer = "ok";
  } catch (e) { log("steer failed", e.message); results.steer = e.message; }
  await new Promise(r => setTimeout(r, 1500));
  const ir = await s.request("turn/interrupt", { threadId, turnId: t2.turn.id });
  log("interrupt", JSON.stringify(ir));
  const c2 = await completed;
  console.log();
  log("turn 2 completed:", c2.params.turn.status);
  results.turn2 = c2.params.turn.status;
  s.stop();

  // 3. Resume the thread in a new process, as after pansy restarts.
  const s2 = server();
  await s2.request("initialize", { clientInfo: { name: "pansy-p0", title: null, version: "0.0.0" }, capabilities: { experimentalApi: true, requestAttestation: false } });
  s2.notify("initialized", {});
  const resumed = await s2.request("thread/resume", { threadId });
  log("resumed", resumed.thread.id, "turns", resumed.thread.turns?.length);
  completed = watch(s2, m => m.method === "turn/completed");
  await s2.request("turn/start", { threadId, input: text("What file did you create earlier? Answer with the file name only.") });
  const c3 = await completed;
  console.log();
  log("turn 3 completed:", c3.params.turn.status);
  results.resume = c3.params.turn.status;
  s2.stop();

  log("notifications seen:", [...seen].sort().join(", "));
  log("RESULTS", JSON.stringify(results));
}

main().catch(e => { log("FAILED", e.stack); process.exit(1); });
setTimeout(() => { log("TIMEOUT", JSON.stringify(results)); process.exit(2); }, 240_000).unref();
