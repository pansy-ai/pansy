// P0 spike (docs/plans/phone.md): drive Claude Code through the official
// Agent SDK the way pansy-remote would, on Node or on pansy's Bun. One
// process holds the session: a turn that needs a tool approved
// (canUseTool), a model switch (setModel), an interrupted turn
// (interrupt), then a second query that resumes the session.
//
// Runs with a scratch CLAUDE_CONFIG_DIR and folder, pointed at pansy's
// gateway as `pansy run claude` points it, with the user's own `claude`:
// none of the user's Claude Code files are read or written.
//
//   PANSY_SDK=/dir/with/node_modules node spikes/remote/claude.mjs [model] [model2]
//   PANSY_SDK=… ~/.config/pansy/bun/<v>/bun-linux-x64/bun spikes/remote/claude.mjs

import { execFileSync } from "node:child_process";
import { mkdtempSync, mkdirSync, writeFileSync, existsSync, realpathSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { pathToFileURL } from "node:url";

const sdkDir = process.env.PANSY_SDK ?? "/tmp/p0/sdk";
const { query } = await import(pathToFileURL(join(sdkDir, "node_modules/@anthropic-ai/claude-agent-sdk/sdk.mjs")).href);

const model = process.argv[2] ?? "nofx/claude-fable-5";
const model2 = process.argv[3] ?? "nofx/claude-fable-5-1";
const runtime = typeof Bun !== "undefined" ? `bun ${Bun.version}` : `node ${process.version}`;
const claude = realpathSync(execFileSync("bash", ["-lc", "command -v claude"]).toString().trim().replace(/command-wrappers\/bin\/claude$/, "installs/claude/latest/claude"));
const scratch = mkdtempSync(join(tmpdir(), "pansy-p0-claude-"));
const config = join(scratch, "claude-config");
const work = join(scratch, "work");
mkdirSync(config);
mkdirSync(work);
writeFileSync(join(work, "README.md"), "# scratch\n");

const env = {
  ...process.env,
  CLAUDE_CONFIG_DIR: config,
  ANTHROPIC_BASE_URL: process.env.PANSY_GATEWAY ?? "http://127.0.0.1:7267",
  ANTHROPIC_AUTH_TOKEN: "pansy-p0",
  ANTHROPIC_MODEL: model,
  ANTHROPIC_SMALL_FAST_MODEL: model,
  ANTHROPIC_DEFAULT_HAIKU_MODEL: model,
  CLAUDE_CODE_DISABLE_NONESSENTIAL_TRAFFIC: "1",
};
delete env.ANTHROPIC_API_KEY;

const log = (...a) => console.log(new Date().toISOString().slice(11, 23), ...a);
const results = { runtime };

// Messages fed in one at a time, as a phone would send them.
function inbox() {
  const queue = [];
  let wake = null, closed = false;
  return {
    push(text) { queue.push({ type: "user", message: { role: "user", content: text }, parent_tool_use_id: null, session_id: "" }); wake?.(); },
    close() { closed = true; wake?.(); },
    async *[Symbol.asyncIterator]() {
      for (;;) {
        if (queue.length) { yield queue.shift(); continue; }
        if (closed) return;
        await new Promise(r => (wake = r));
        wake = null;
      }
    },
  };
}

const canUseTool = async (tool, input, { signal }) => {
  log("APPROVAL", tool, JSON.stringify(input).slice(0, 200));
  results.approval = (results.approval ?? []).concat(tool);
  return { behavior: "allow", updatedInput: input };
};

const options = {
  cwd: work, env, model, pathToClaudeCodeExecutable: claude, permissionMode: "default",
  canUseTool, includePartialMessages: true, settingSources: [],
};

async function main() {
  log(runtime, "claude", claude, "scratch", scratch, "model", model);
  const box = inbox();
  const q = query({ prompt: box, options });
  let session, turns = 0, interrupted = false;
  const step = async () => {
    turns++;
    if (turns === 1) {
      await q.setModel(model2);
      log("setModel", model2);
      results.setModel = "ok";
      box.push("Count slowly from 1 to 300, one number per line, nothing else.");
      setTimeout(async () => { interrupted = true; await q.interrupt(); log("interrupt sent"); }, 3000);
    } else box.close();
  };
  box.push("Use the Bash tool to run `touch approved.txt` in the current folder, then reply with the single word done.");
  let streamed = 0;
  try { for await (const m of q) {
    if (m.type === "system" && m.subtype === "init") { session = m.session_id; log("init session", session, "model", m.model, "permissionMode", m.permissionMode); }
    else if (m.type === "stream_event") { if (m.event?.type === "content_block_delta" && m.event.delta?.text) { streamed++; if (streamed < 40) process.stdout.write(m.event.delta.text); } }
    else if (m.type === "result") {
      console.log();
      log("result", m.subtype, "turn", turns + 1, "is_error", m.is_error, "model usage", Object.keys(m.modelUsage ?? {}).join(","));
      results[`turn${turns + 1}`] = m.subtype + (interrupted && turns === 1 ? " (after interrupt)" : "");
      await step();
    }
  } } catch (e) {
    // The SDK throws when the input ends on an error result, which an
    // interrupted turn is: pansy-remote must treat that as the turn's end.
    if (!interrupted) throw e;
    log("query ended after the interrupt:", String(e.message).slice(0, 120));
    results.endAfterInterrupt = "throws";
  }
  results.approvedFile = existsSync(join(work, "approved.txt"));
  results.streamedDeltas = streamed;

  // Resume in a new query, as after pansy restarts.
  const box2 = inbox();
  const q2 = query({ prompt: box2, options: { ...options, resume: session } });
  box2.push("What file did you create earlier? Answer with the file name only.");
  for await (const m of q2) {
    if (m.type === "system" && m.subtype === "init") log("resumed session", m.session_id, m.session_id === session ? "(same id)" : "(new id)");
    if (m.type === "assistant") for (const c of m.message.content ?? []) if (c.type === "text") log("answer:", c.text.trim());
    if (m.type === "result") { results.resume = m.subtype; box2.close(); }
  }
  log("RESULTS", JSON.stringify(results));
}

main().catch(e => { log("FAILED", e.stack ?? e); log("RESULTS", JSON.stringify(results)); process.exit(1); });
setTimeout(() => { log("TIMEOUT", JSON.stringify(results)); process.exit(2); }, 240_000).unref?.();
