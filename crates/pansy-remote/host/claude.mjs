// pansy-remote's Claude Code host (crates/pansy-remote/src/claude.rs): the
// Agent SDK's query() on pansy's Bun, spoken to over stdin and stdout in
// JSON-RPC lines, one message a line, as rpc.rs speaks to Codex.
//
//   bun claude.mjs <dir the SDK is installed in>
//
// pansy asks:
//   start {options}                   → {} once Claude Code has answered
//   send {text, uuid, model?, effort?} → {} once the message is queued
//   interrupt {}  mode {mode}  stop {}
// it is told:
//   started {uuid}       a message went to Claude Code
//   message {…}          the SDK's messages: system init, assistant, user,
//                        result, and stream events (message_start, text)
//   resolved {request}   an approval was answered, or given up
//   ended {error, carried}  a query ended; `carried` go to the next one
// and asked, to answer with the SDK's PermissionResult:
//   canUseTool {tool, input, suggestions, title, reason, blockedPath, toolUseID}
//
// One query holds the session across turns, fed through an async iterable.
// When it ends (after an interrupt, or Claude Code exiting), the next
// message starts another that resumes the session.

import { join } from "node:path";
import { pathToFileURL } from "node:url";

const sdk = process.argv[2];
const { query } = await import(
  pathToFileURL(join(sdk, "node_modules/@anthropic-ai/claude-agent-sdk/sdk.mjs")).href
);

const out = (m) => process.stdout.write(JSON.stringify(m) + "\n");
const notify = (method, params) => out({ method, params });

// --- Messages fed in one at a time ------------------------------------------------

function inbox() {
  const queue = [];
  let wake = null;
  let closed = false;
  return {
    push(m) {
      queue.push(m);
      wake?.();
    },
    close() {
      closed = true;
      wake?.();
    },
    // What was never given to the query.
    drain() {
      closed = true;
      return queue.splice(0);
    },
    async *[Symbol.asyncIterator]() {
      for (;;) {
        if (closed) return;
        if (queue.length) {
          used = true;
          yield queue.shift();
          continue;
        }
        await new Promise((r) => (wake = r));
        wake = null;
      }
    },
  };
}

// --- The session ------------------------------------------------------------------

let base = null; // the options pansy started with
let session = ""; // Claude Code's session id
let used = false; // whether the session has had a message, so exists to resume
let q = null;
let box = null;
const now = { model: undefined, effort: undefined, mode: undefined };

function open() {
  const options = { ...base, canUseTool, includePartialMessages: true };
  delete options.sessionId;
  delete options.resume;
  if (used) options.resume = session;
  else options.sessionId = session;
  if (now.model) options.model = now.model;
  if (now.effort) options.effort = now.effort;
  if (now.mode) options.permissionMode = now.mode;
  options.stderr = (text) => process.stderr.write(text);
  const mine = inbox();
  const query_ = query({ prompt: mine, options });
  q = query_;
  box = mine;
  pump(query_, mine);
  return query_;
}

async function pump(query_, mine) {
  let error = "";
  try {
    for await (const m of query_) forward(m);
  } catch (e) {
    // The SDK throws when its input ends on an error result, as an
    // interrupted turn's is: the turn's end all the same.
    error = String(e?.message ?? e);
  }
  if (q === query_) {
    q = null;
    box = null;
  }
  const carried = mine.drain();
  notify("ended", { error, carried: carried.map((m) => m.uuid) });
  if (carried.length) {
    open();
    for (const m of carried) box.push(m);
  }
}

function forward(m) {
  if (m.type === "stream_event") {
    const e = m.event;
    if (m.parent_tool_use_id) return;
    if (e?.type === "message_start" || (e?.type === "content_block_delta" && e.delta?.type === "text_delta"))
      notify("message", m);
    return;
  }
  if (m.type === "system" && m.subtype !== "init") return;
  if (["system", "assistant", "user", "result"].includes(m.type)) notify("message", m);
}

// --- Approvals --------------------------------------------------------------------

let approvals = 0;
const waiting = new Map(); // request → resolve

function canUseTool(tool, input, opts) {
  const id = `a${++approvals}`;
  return new Promise((resolve) => {
    waiting.set(id, resolve);
    opts.signal?.addEventListener("abort", () => {
      if (!waiting.delete(id)) return;
      notify("resolved", { request: id });
      resolve({ behavior: "deny", message: "The turn was stopped." });
    });
    out({
      id,
      method: "canUseTool",
      params: {
        tool,
        input,
        suggestions: opts.suggestions ?? [],
        title: opts.title ?? "",
        reason: opts.decisionReason ?? opts.description ?? "",
        blockedPath: opts.blockedPath ?? "",
        toolUseID: opts.toolUseID ?? "",
      },
    });
  });
}

// --- What pansy asks --------------------------------------------------------------

const handlers = {
  async start(p) {
    base = p.options;
    session = base.sessionId ?? base.resume;
    used = !!base.resume;
    await open().initializationResult();
    return {};
  },
  async send(p) {
    if (p.model) {
      if (q) await q.setModel(p.model);
      now.model = p.model;
    }
    if (p.effort) {
      if (q) await q.applyFlagSettings({ effortLevel: p.effort });
      now.effort = p.effort;
    }
    if (!q) open();
    notify("started", { uuid: p.uuid });
    box.push({
      type: "user",
      message: { role: "user", content: p.text },
      parent_tool_use_id: null,
      session_id: session,
      uuid: p.uuid,
    });
    return {};
  },
  async interrupt() {
    if (q) await q.interrupt();
    return {};
  },
  async mode(p) {
    if (q) await q.setPermissionMode(p.mode);
    now.mode = p.mode;
    return {};
  },
  async stop() {
    box?.close();
    setTimeout(() => process.exit(0), 50);
    return {};
  },
};

async function handle(m) {
  if (m.method === undefined) {
    // pansy's answer to an approval
    const resolve = waiting.get(m.id);
    if (!resolve) return;
    waiting.delete(m.id);
    notify("resolved", { request: m.id });
    resolve(m.result);
    return;
  }
  const handler = handlers[m.method];
  try {
    if (!handler) throw new Error(`no method ${m.method}`);
    out({ id: m.id, result: await handler(m.params ?? {}) });
  } catch (e) {
    out({ id: m.id, error: { message: String(e?.message ?? e) } });
  }
}

let rest = "";
process.stdin.setEncoding("utf8");
process.stdin.on("data", (chunk) => {
  rest += chunk;
  let at;
  while ((at = rest.indexOf("\n")) >= 0) {
    const line = rest.slice(0, at).trim();
    rest = rest.slice(at + 1);
    if (!line) continue;
    let m;
    try {
      m = JSON.parse(line);
    } catch {
      continue;
    }
    handle(m);
  }
});
process.stdin.on("end", () => process.exit(0));
