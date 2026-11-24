// A scripted home pansy: protocol v1 (docs/plans/phone-protocol.md) with
// canned machines, threads, models, git, devices and usage, and a fake agent that
// streams an answer, asks to run a command and finishes. Plain TypeScript
// with no platform API but timers: mock/server.ts serves it over HTTP and a
// WebSocket (Bun), test/harness.tsx puts it behind the app's transport.
// It checks no DPoP proof and no token: a stand-in to try the app with.
import type { Socket } from "../net/transport";
import type { Decision, Device, Diff, DiffFile, Environment, Event, GitStatus, Models, Project, Seq, ThreadSummary, Totals, Turn, Usage } from "../net/types";

export type Client = { send(msg: unknown): void; shell: boolean; threads: Set<string>; deviceId: string };
export type HttpResult = { status: number; body?: unknown; headers?: Record<string, string> };
type Call = { method: string; params: Record<string, unknown> };

const ago = (s: number) => new Date(Date.now() - s * 1000).toISOString();
const err = (code: string, message: string) => Object.assign(new Error(message), { code });
const rand = () => Math.random().toString(36).slice(2, 10);

/** Everything a device may do: pairing gives all of it (as T3 does). */
const SCOPES = ["access:read", "access:write", "orchestration:read", "orchestration:operate", "source-control:write", "filesystem:read", "terminal:read", "terminal:operate", "settings:write", "providers:manage"];

const PROJECTS: Project[] = [
  { folder: "/home/me/pansy", name: "pansy", git: true, branch: "main" },
  { folder: "/home/me/ojbk", name: "ojbk", git: true, branch: "main" },
  { folder: "/home/me/umiro", name: "umiro", git: true, branch: "dev" },
  { folder: "/home/me/notes", name: "notes", git: false },
];

const MODELS: Models = {
  groups: [
    { id: "group/daily", name: "daily", members: 3, routing: "order" },
    { id: "group/cheap", name: "cheap", members: 4, routing: "least" },
  ],
  providers: [
    { id: "deepseek", name: "DeepSeek", models: [{ id: "deepseek/v4", name: "DeepSeek V4", context: 128_000, reasoning: true }, { id: "deepseek/v4-flash", name: "DeepSeek V4 Flash", context: 128_000 }] },
    { id: "kimi", name: "Kimi", models: [{ id: "kimi/k3", name: "Kimi K3", context: 256_000, reasoning: true, vision: true }] },
    { id: "codex", name: "ChatGPT subscription (codex/…)", models: [{ id: "codex/gpt-5.5", name: "GPT-5.5", context: 400_000, reasoning: true, vision: true }] },
  ],
};

/** The gateway's count of the last `days` days: the same canned week, again and again. */
function usageOf(days: number): Usage {
  const zero = (): Totals => ({ requests: 0, errors: 0, input: 0, output: 0, cacheRead: 0, cacheWrite: 0, reasoning: 0 });
  const add = (into: Totals, t: Totals) => {
    for (const k of Object.keys(into) as (keyof Totals)[]) into[k] += t[k];
  };
  const shares: [string, string, number][] = [
    ["deepseek/deepseek-v4", "codex", 0.46],
    ["kimi/kimi-k3", "claude-code", 0.27],
    ["codex/gpt-5.5", "codex", 0.19],
    ["anthropic/claude-sonnet-5.5", "claude-code", 0.08],
  ];
  const week = [1.0, 0.4, 0.75, 1.3, 0.2, 0.9, 1.1];
  const out: Usage = { days, total: zero(), byDay: {}, byModel: {}, byAgent: {} };
  for (let back = days - 1; back >= 0; back--) {
    const d = new Date(Date.now() - back * 86_400_000);
    const key = `${d.getFullYear()}-${String(d.getMonth() + 1).padStart(2, "0")}-${String(d.getDate()).padStart(2, "0")}`;
    const day = zero();
    for (const [model, agent, share] of shares) {
      const f = week[back % week.length] * share;
      const t: Totals = { requests: Math.round(180 * f), errors: model.startsWith("kimi") ? Math.round(4 * f) : 0, input: Math.round(2_400_000 * f), output: Math.round(160_000 * f), cacheRead: Math.round(1_800_000 * f), cacheWrite: 0, reasoning: Math.round(40_000 * f) };
      add(day, t);
      add((out.byModel[model] ??= zero()), t);
      add((out.byAgent[agent] ??= zero()), t);
    }
    out.byDay[key] = day;
    add(out.total, day);
  }
  return out;
}

const ANSWER = `花费从 \`usage::by_project\` 来，按月汇总；Sessions 的每个文件夹标题右边显示 \`¥12.40 本月\`。

改了这两处：

- \`desktop/ui/Sessions.tsx\`：文件夹标题加上花费
- \`crates/pansy-core/src/usage.rs\`：新的 \`by_project\`

\`\`\`rust
pub fn by_project(&self, month: Month) -> BTreeMap<PathBuf, Cost> {
    let mut out = BTreeMap::new();
    // …
    out
}
\`\`\`

还差一步：跑测试确认没改坏。`;

function file(path: string, status: string, hunks: DiffFile["hunks"]): DiffFile {
  let add = 0;
  let del = 0;
  for (const h of hunks) for (const l of h.lines) l.kind === "+" ? add++ : l.kind === "-" ? del++ : 0;
  return { path, status, add, del, hunks };
}

const SESSIONS = file("desktop/ui/Sessions.tsx", "modified", [
  {
    header: "@@ -142,9 +142,14 @@ function Folder(props)",
    lines: [
      { kind: " ", old: 142, new: 142, text: "  const t = useT();" },
      { kind: " ", old: 143, new: 143, text: "  return (" },
      { kind: "-", old: 144, text: "    <Row title={props.name}>" },
      { kind: "+", new: 144, text: "    <Row title={props.name} trailing={" },
      { kind: "+", new: 145, text: "      <Spend value={cost(props.folder)} />" },
      { kind: "+", new: 146, text: "    }>" },
      { kind: " ", old: 145, new: 147, text: "      {props.children}" },
      { kind: " ", old: 146, new: 148, text: "    </Row>" },
    ],
  },
]);
const USAGE = file("crates/pansy-core/src/usage.rs", "modified", [
  {
    header: "@@ -88,6 +88,21 @@ impl Usage",
    lines: [
      { kind: " ", old: 88, new: 88, text: "    }" },
      { kind: "+", new: 89, text: "    pub fn by_project(&self, month: Month)" },
      { kind: "+", new: 90, text: "        -> BTreeMap<PathBuf, Cost> {" },
      { kind: "+", new: 91, text: "        let mut out = BTreeMap::new();" },
      { kind: "-", old: 89, text: "    // TODO: by project" },
    ],
  },
]);
const I18N = file("desktop/ui/i18n.ts", "modified", [
  { header: "@@ -412,3 +412,5 @@ const ZH_HANS = {", lines: [{ kind: " ", old: 412, new: 412, text: '  "Spend": "花费",' }, { kind: "+", new: 413, text: '  "%s this month": "本月 %s",' }, { kind: "+", new: 414, text: '  "No spend yet": "还没有花费",' }] },
]);

function seedThreads(): ThreadSummary[] {
  const base = { workdir: "", source: "pansy" as const, mode: "full" as const, updatedAt: ago(60) };
  const t = (x: Partial<ThreadSummary> & Pick<ThreadSummary, "id" | "title" | "folder" | "agent" | "model" | "status">): ThreadSummary => ({ ...base, workdir: x.folder, ...x });
  return [
    t({ id: "t_cost", title: "给 Sessions 页加按项目的花费", folder: "/home/me/pansy", workdir: "/home/me/.pansy/worktrees/pansy/sessions-cost-3f2a", agent: "claude-code", model: "deepseek/v4", status: "approval", branch: "pansy/sessions-cost", worktree: true, base: "main", diff: { add: 48, del: 6 }, startedAt: ago(95), updatedAt: ago(20), session: "c1" }),
    t({ id: "t_sub", title: "修复 Codex 子 agent 的模型头", folder: "/home/me/pansy", agent: "codex", model: "group/daily", effort: "medium", status: "working", branch: "main", startedAt: ago(192), updatedAt: ago(5), queue: 1, diff: { add: 18, del: 3 } }),
    t({ id: "t_relay", title: "relay 的部署脚本要不要拆成两步？", folder: "/home/me/umiro", agent: "codex", model: "kimi/k3", status: "idle", branch: "dev", updatedAt: ago(1800) }),
    t({ id: "t_aur", title: "AUR 包的 PKGBUILD 校验和", folder: "/home/me/pansy", agent: "codex", model: "deepseek/v4", status: "failed", error: "cargo test：3 个测试失败（pansy-agents）", branch: "aur", updatedAt: ago(3600) }),
    t({ id: "t_ojbk", title: "ojbk：港股开盘的价差回测", folder: "/home/me/ojbk", agent: "claude-code", model: "deepseek/v4", status: "done", unread: true, pr: { number: 16, url: "https://github.com/example/ojbk/pull/16", state: "open" }, diff: { add: 214, del: 37 }, branch: "spread-backtest", updatedAt: ago(7200) }),
    t({ id: "s:claude-code:9f1e", title: "README 的截图换成深色", folder: "/home/me/pansy", agent: "claude-code", model: "claude-sonnet-5.5", status: "idle", source: "terminal", session: "9f1e", updatedAt: ago(86_400 + 3600) }),
    t({ id: "t_nsh", title: "NSH 0.6.1 的升级", folder: "/home/me/pansy", agent: "codex", model: "group/daily", status: "idle", archived: true, updatedAt: ago(86_400 * 3) }),
    t({ id: "t_dl", title: "下载页只给安装包", folder: "/home/me/pansy", agent: "codex", model: "deepseek/v4", status: "idle", archived: true, updatedAt: ago(86_400 * 3) }),
    t({ id: "t_arb", title: "Binance 和 Longbridge 的套利", folder: "/home/me/ojbk", agent: "claude-code", model: "kimi/k3", status: "idle", archived: true, updatedAt: ago(86_400 * 4) }),
    t({ id: "t_s1", title: "仿 T3 的初始化引导", folder: "/home/me/pansy", agent: "codex", model: "deepseek/v4", status: "idle", archived: true, updatedAt: ago(86_400 * 5) }),
    t({ id: "t_s2", title: "发布安装包到 omarchy", folder: "/home/me/pansy", agent: "codex", model: "deepseek/v4", status: "idle", archived: true, updatedAt: ago(86_400 * 5) }),
    t({ id: "t_s3", title: "Claude Desktop 支持", folder: "/home/me/pansy", agent: "codex", model: "deepseek/v4", status: "idle", archived: true, updatedAt: ago(86_400 * 6) }),
    t({ id: "t_s4", title: "Web 端下载页按系统给安装包", folder: "/home/me/pansy-site", agent: "codex", model: "deepseek/v4", status: "idle", archived: true, updatedAt: ago(86_400 * 6) }),
    t({ id: "t_s5", title: "提交 nsh-showcases 的 PR", folder: "/home/me/nsh-showcases", agent: "codex", model: "deepseek/v4", status: "idle", archived: true, updatedAt: ago(86_400 * 7) }),
    t({ id: "t_s6", title: "macOS 和 Windows 安装包随 tag 发布", folder: "/home/me/pansy", agent: "codex", model: "deepseek/v4", status: "idle", archived: true, updatedAt: ago(86_400 * 7) }),
    t({ id: "t_s7", title: "README 的 fork 文案", folder: "/home/me/pansy", agent: "codex", model: "deepseek/v4", status: "idle", archived: true, updatedAt: ago(86_400 * 8) }),
    t({ id: "t_s8", title: "Web 新增插件模块", folder: "/home/me/pansy-site", agent: "codex", model: "deepseek/v4", status: "idle", archived: true, updatedAt: ago(86_400 * 9) }),
    t({ id: "t_s9", title: "umiro 的中继压测", folder: "/home/me/umiro", agent: "codex", model: "deepseek/v4", status: "idle", archived: true, updatedAt: ago(86_400 * 10) }),
  ];
}

function seedHistory(): Record<string, Turn[]> {
  return {
    t_cost: [
      { role: "user", text: "Sessions 页按文件夹分组的地方，加上每个项目这个月花了多少钱，数字从 usage 里来。", at: ago(100) },
      { role: "reasoning", text: "用户要按项目看花费。usage/<day>.jsonl 里有 project 字段，pansy run 发出的请求才有。先看 Sessions.tsx 怎么分组。" },
      { role: "tool", name: "read", text: "读了 desktop/ui/Sessions.tsx", output: "desktop/ui/Sessions.tsx · 1–220 行", status: "completed" },
      { role: "tool", name: "search", text: '搜了 "project" · 14 处', output: "crates/pansy-core/src/usage.rs:88\ncrates/pansy-core/src/project.rs:12\ncrates/pansy-gateway/src/server.rs:1512", status: "completed" },
      { role: "tool", name: "edit", text: "改了 2 个文件 +48 −6", output: "desktop/ui/Sessions.tsx   +31 −4\ncrates/pansy-core/src/usage.rs  +17 −2", status: "completed" },
      { role: "assistant", text: ANSWER, at: ago(30) },
    ],
    t_sub: [
      { role: "user", text: "Codex 的子 agent 发请求时没带 pansy 的模型头，查一下", at: ago(400) },
      { role: "assistant", text: "找到了：`default_subagent_model` 只写进了 config，子 agent 的请求头里没有。", at: ago(300) },
      { role: "user", text: "好，顺手把 Codex 的 golden 也 bless 一下", at: ago(200) },
      { role: "tool", name: "read", text: "读了 crates/pansy-agents/agents/codex.toml", status: "completed" },
      { role: "tool", name: "read", text: "读了 crates/pansy-agents/src/run.rs", status: "completed" },
      { role: "tool", name: "edit", text: "改了 crates/pansy-agents/agents/codex.toml", status: "completed" },
      { role: "tool", name: "shell", text: "删了 agents/edits.golden.orig", input: "rm crates/pansy-agents/agents/edits.golden.orig", status: "completed" },
    ],
    t_relay: [
      { role: "user", text: "把 relay 的部署整理一下，现在一个脚本又建 Worker 又发证书", at: ago(2000) },
      { role: "assistant", text: "可以拆，但证书那一步要你的 Cloudflare 账号权限。拆成 `deploy.sh` 和 `certs.sh` 两个脚本行吗？", at: ago(1800) },
    ],
    t_aur: [
      { role: "user", text: "更新 PKGBUILD 的校验和", at: ago(3700) },
      { role: "tool", name: "shell", text: "跑了 cargo test -p pansy-agents", output: "test result: FAILED. 39 passed; 3 failed", status: "failed" },
    ],
    t_ojbk: [
      { role: "user", text: "回测一下港股开盘前 5 分钟的价差", at: ago(7400) },
      { role: "assistant", text: "回测做完了，开了 PR #16。开盘前 5 分钟的平均价差是 **0.38%**，扣掉费用后还剩 0.11%。", at: ago(7200) },
    ],
    "s:claude-code:9f1e": [
      { role: "user", text: "README 的截图换成深色的", at: ago(90_000) },
      { role: "assistant", text: "换好了：`assets/screenshots/` 下的 4 张图都是深色主题。", at: ago(89_000) },
    ],
  };
}

const sleep = (ms: number) => new Promise<void>((r) => setTimeout(r, ms));

type Run = { cancel: boolean; approval?: { request: string; resolve: (d: Decision) => void }; question?: { request: string; resolve: (answers: Record<string, string> | null) => void } };

export class FakeHome {
  readonly env: Environment;
  readonly threads = new Map<string, ThreadSummary>();
  readonly history = new Map<string, Turn[]>();
  readonly live = new Map<string, Seq<Event>[]>();
  readonly seqs = new Map<string, number>();
  readonly devices: Device[] = [];
  readonly clients = new Set<Client>();
  readonly queued = new Map<string, string[]>();
  readonly runs = new Map<string, Run>();
  readonly tickets = new Map<string, string>();
  readonly tokens = new Map<string, string>();
  /** Threads whose next push is rejected: a commit message saying "[push fails]". */
  readonly rejecting = new Set<string>();
  requireApproval = false;
  /** Every RPC, for tests. */
  onCall?: (call: Call) => void;
  private scale: number;

  constructor(options: { base: string; name?: string; scale?: number; requireApproval?: boolean }) {
    this.scale = options.scale ?? 1;
    this.requireApproval = options.requireApproval ?? false;
    this.env = {
      id: "m_5f2a0c",
      name: options.name ?? "家里的 Mac",
      version: "0.2.0",
      protocol: 1,
      capabilities: ["codex", "claude", "git", "github"],
      routes: [{ kind: "lan", url: options.base }],
      os: "macOS",
    };
    for (const t of seedThreads()) this.threads.set(t.id, t);
    for (const [id, h] of Object.entries(seedHistory())) this.history.set(id, h);
    this.devices.push({ id: "d_phone", name: "iPhone", platform: "ios", scopes: SCOPES, approval: "approved", createdAt: ago(86_400 * 2), lastSeen: ago(30) });
    this.devices.push({ id: "d_ipad", name: "Safari · iPad", platform: "web", scopes: ["orchestration:read"], approval: "approved", createdAt: ago(86_400 * 10), lastSeen: ago(86_400 * 3) });
    // the waiting approval of t_cost, and the turn t_sub is in
    this.emit("t_cost", { type: "turnStarted", turn: "turn_c2" });
    this.emit("t_cost", { type: "toolStarted", item: "i_test", kind: "commandExecution", title: "cargo test -p pansy-core usage::" });
    this.emit("t_cost", { type: "approval", request: "r_test", kind: "commandExecution", title: "cargo test", detail: "cargo test -p pansy-core usage::", reason: "确认按项目汇总的改动没有改坏用量的测试。" });
    this.runs.set("t_cost", { cancel: false, approval: { request: "r_test", resolve: (d) => void this.afterTest("t_cost", d) } });
    this.emit("t_sub", { type: "turnStarted", turn: "turn_s3" });
    this.emit("t_sub", { type: "toolStarted", item: "i_explore", kind: "subagent", title: "Explore · codex · deepseek/v4" });
    this.emit("t_sub", { type: "toolStarted", item: "i_bless", kind: "commandExecution", title: "PANSY_BLESS=1 cargo test -p pansy-agents" });
    this.runs.set("t_sub", { cancel: false });
    this.queued.set("t_sub", ["测完再看看 docs/compat.md 要不要重写"]);
  }

  private ms(n: number) {
    return sleep(n * this.scale);
  }

  // —— events

  private emit(threadId: string, event: Event) {
    const seq = (this.seqs.get(threadId) ?? 0) + 1;
    this.seqs.set(threadId, seq);
    const list = this.live.get(threadId) ?? [];
    list.push({ seq, event });
    if (list.length > 2000) list.shift();
    this.live.set(threadId, list);
    for (const c of this.clients) if (c.threads.has(threadId)) c.send({ event: "thread.event", data: { threadId, seq, event } });
  }

  private change(id: string, patch: Partial<ThreadSummary>) {
    const t = this.threads.get(id);
    if (!t) return;
    const next = { ...t, ...patch, updatedAt: new Date().toISOString() };
    this.threads.set(id, next);
    for (const c of this.clients) if (c.shell) c.send({ event: "shell.thread", data: next });
  }

  private broadcast(event: string, data: unknown) {
    for (const c of this.clients) c.send({ event, data });
  }

  // —— the scripted agent

  private async turn(threadId: string, text: string) {
    const run: Run = { cancel: false };
    this.runs.set(threadId, run);
    const stop = () => run.cancel;
    const turn = `turn_${rand()}`;
    this.emit(threadId, { type: "user", text });
    this.emit(threadId, { type: "turnStarted", turn });
    this.change(threadId, { status: "working", startedAt: new Date().toISOString(), unread: false });
    await this.ms(500);
    if (stop()) return this.stopped(threadId, turn);
    if (/问我|ask me/i.test(text)) return this.ask(threadId, turn, run);
    const read = `i_${rand()}`;
    this.emit(threadId, { type: "toolStarted", item: read, kind: "read", title: "读 desktop/ui/Sessions.tsx" });
    await this.ms(500);
    this.emit(threadId, { type: "toolFinished", item: read, kind: "read", title: "读了 desktop/ui/Sessions.tsx", status: "completed", output: "desktop/ui/Sessions.tsx · 1–220 行" });
    const sub = `i_${rand()}`;
    this.emit(threadId, { type: "toolStarted", item: sub, kind: "subagent", title: "Explore · codex · deepseek/v4" });
    const thought = `i_${rand()}`;
    this.emit(threadId, { type: "toolStarted", item: thought, kind: "reasoning", title: "Thinking" });
    this.emit(threadId, { type: "toolFinished", item: thought, kind: "reasoning", title: "Thinking", status: "completed", output: "**找 project 字段从哪来**\n\nusage/<day>.jsonl 里有 project，只有 pansy run 发出的请求才带。" });
    const grep = `i_${rand()}`;
    this.emit(threadId, { type: "toolStarted", item: grep, kind: "search", title: '搜 "project"' });
    await this.ms(600);
    this.emit(threadId, { type: "toolFinished", item: grep, kind: "search", title: '搜了 "project" · 14 处', status: "completed", output: "crates/pansy-core/src/usage.rs:88\ncrates/pansy-core/src/project.rs:12" });
    if (stop()) return this.stopped(threadId, turn);
    const msg = `i_${rand()}`;
    for (const chunk of ANSWER.match(/[\s\S]{1,12}/g) ?? []) {
      if (stop()) return this.stopped(threadId, turn);
      this.emit(threadId, { type: "delta", item: msg, text: chunk });
      await this.ms(60);
    }
    this.emit(threadId, { type: "message", item: msg, text: ANSWER });
    this.emit(threadId, { type: "toolFinished", item: sub, kind: "subagent", title: "Explore · codex · deepseek/v4", status: "completed" });
    this.change(threadId, { diff: { add: 48, del: 6 } });
    const cmd = `i_${rand()}`;
    this.emit(threadId, { type: "toolStarted", item: cmd, kind: "commandExecution", title: "cargo test -p pansy-core usage::" });
    const request = `r_${rand()}`;
    const decision = new Promise<Decision>((resolve) => (run.approval = { request, resolve }));
    this.emit(threadId, { type: "approval", request, kind: "commandExecution", title: "cargo test", detail: "cargo test -p pansy-core usage::", reason: "确认按项目汇总的改动没有改坏用量的测试。" });
    this.change(threadId, { status: "approval" });
    const d = await decision;
    if (stop()) return this.stopped(threadId, turn);
    await this.afterTest(threadId, d, cmd, turn);
  }

  /** A question, as Claude Code's AskUserQuestion comes. */
  private async ask(threadId: string, turn: string, run: Run) {
    const request = `q_${rand()}`;
    const answered = new Promise<Record<string, string> | null>((resolve) => (run.question = { request, resolve }));
    this.emit(threadId, {
      type: "question",
      request,
      questions: [{ question: "Which database?", header: "Storage", multiSelect: false, options: [{ label: "SQLite", description: "One file, nothing to run" }, { label: "Postgres", description: "A server" }] }],
    });
    this.change(threadId, { status: "approval" });
    const answers = await answered;
    if (run.cancel) return this.stopped(threadId, turn);
    const msg = `i_${rand()}`;
    const text = answers ? `Going with ${Object.values(answers).join(", ")}.` : "I'll pick one myself.";
    this.emit(threadId, { type: "message", item: msg, text });
    this.finish(threadId, turn, "completed");
  }

  /** What follows the test command's approval. */
  private async afterTest(threadId: string, d: Decision, cmd = "i_test", turn = "turn_c2") {
    const run = this.runs.get(threadId);
    if (run) run.approval = undefined;
    this.change(threadId, { status: "working" });
    const msg = `i_${rand()}`;
    if (d === "accept" || d === "acceptForSession") {
      await this.ms(800);
      this.emit(threadId, { type: "toolFinished", item: cmd, kind: "commandExecution", title: "cargo test -p pansy-core usage::", status: "completed", output: "running 42 tests\n…\ntest result: ok. 42 passed; 0 failed", exit: 0 });
      for (const chunk of ["测试都", "通过了（42 个）。", "可以提交了。"]) {
        this.emit(threadId, { type: "delta", item: msg, text: chunk });
        await this.ms(120);
      }
      this.emit(threadId, { type: "message", item: msg, text: "测试都通过了（42 个）。可以提交了。" });
    } else {
      this.emit(threadId, { type: "toolFinished", item: cmd, kind: "commandExecution", title: "cargo test -p pansy-core", status: "declined" });
      this.emit(threadId, { type: "message", item: msg, text: "好，不跑测试了。" });
    }
    this.finish(threadId, turn, "completed");
  }

  private finish(threadId: string, turn: string, status: string) {
    this.emit(threadId, { type: "turnFinished", turn, status });
    this.runs.delete(threadId);
    const next = this.queued.get(threadId)?.shift();
    this.change(threadId, { status: status === "completed" ? "done" : "idle", startedAt: undefined, unread: true, queue: this.queued.get(threadId)?.length ?? 0 });
    if (next) void this.turn(threadId, next);
  }

  private stopped(threadId: string, turn: string) {
    this.queued.set(threadId, []);
    this.finish(threadId, turn, "interrupted");
  }

  // —— HTTP

  /** Who a request is from: its DPoP token or its cookie. */
  private who(headers: Record<string, string>): string | undefined {
    const auth = headers.authorization ?? headers.Authorization;
    if (auth?.startsWith("DPoP ")) return this.tokens.get(auth.slice(5));
    const cookie = /pansy_session=([^;]+)/.exec(headers.cookie ?? "")?.[1];
    return cookie;
  }

  http(method: string, path: string, body: Record<string, unknown> | undefined, headers: Record<string, string> = {}): HttpResult {
    const route = `${method} ${path.replace(/\?.*$/, "")}`;
    if (route === "GET /.well-known/pansy/environment") return { status: 200, body: this.env };
    if (route === "POST /api/auth/pair") {
      const secret = String(body?.secret ?? "");
      if (!secret || secret === "000000" || /expired|bad|wrong/.test(secret)) return { status: 403, body: { error: { code: "forbidden", message: "pairing secret expired or unknown" } } };
      const id = `d_${rand()}`;
      const approval = this.requireApproval ? "pending" : "approved";
      this.devices.push({ id, name: String(body?.deviceName ?? "Phone"), platform: String(body?.platform ?? "web"), scopes: SCOPES, approval, createdAt: new Date().toISOString(), lastSeen: new Date().toISOString() });
      const code = approval === "pending" ? "K7Q-42M" : undefined;
      if (body?.cookie) return { status: 200, body: { deviceId: id, scopes: SCOPES, approval, code }, headers: { "set-cookie": `pansy_session=${id}; HttpOnly; SameSite=Strict; Path=/` } };
      const token = `tok_${rand()}`;
      this.tokens.set(token, id);
      return { status: 200, body: { deviceId: id, accessToken: token, expiresIn: 3600, scopes: SCOPES, approval, code } };
    }
    const device = this.devices.find((d) => d.id === this.who(headers));
    if (route === "POST /api/auth/refresh") {
      if (!device) return { status: 401, body: { error: { code: "unauthorized", message: "unknown session" } } };
      const token = `tok_${rand()}`;
      this.tokens.set(token, device.id);
      return { status: 200, body: { accessToken: token, expiresIn: 3600 } };
    }
    if (route === "GET /api/auth/me") {
      if (!device) return { status: 401, body: { error: { code: "unauthorized", message: "unknown session" } } };
      return { status: 200, body: { deviceId: device.id, name: device.name, scopes: device.scopes, approval: device.approval } };
    }
    if (route === "POST /api/auth/ws-ticket") {
      if (device?.approval === "pending") return { status: 403, body: { error: { code: "forbidden", message: "waiting for approval" } } };
      const ticket = `tk_${rand()}`;
      this.tickets.set(ticket, device?.id ?? "d_phone");
      return { status: 200, body: { ticket } };
    }
    return { status: 404, body: { error: { code: "not_found", message: route } } };
  }

  // —— the WebSocket

  /** The device a ticket was given to; the ticket works once. */
  takeTicket(ticket: string): string | undefined {
    const d = this.tickets.get(ticket);
    this.tickets.delete(ticket);
    return d;
  }

  connect(client: Client): () => void {
    this.clients.add(client);
    const d = this.devices.find((x) => x.id === client.deviceId);
    if (d) d.lastSeen = new Date().toISOString();
    return () => this.clients.delete(client);
  }

  async receive(client: Client, data: string) {
    let msg: { id: number; method: string; params?: Record<string, unknown> };
    try {
      msg = JSON.parse(data);
    } catch {
      return;
    }
    const params = msg.params ?? {};
    this.onCall?.({ method: msg.method, params });
    try {
      const result = await this.rpc(client, msg.method, params);
      client.send({ id: msg.id, result: result ?? {} });
    } catch (e) {
      client.send({ id: msg.id, error: { code: (e as { code?: string }).code ?? "internal", message: String((e as Error).message ?? e) } });
    }
  }

  private thread(params: Record<string, unknown>): ThreadSummary {
    const t = this.threads.get(String(params.threadId));
    if (!t) throw err("not_found", `no thread ${params.threadId}`);
    return t;
  }

  async rpc(client: Client, method: string, p: Record<string, unknown>): Promise<unknown> {
    switch (method) {
      case "shell.subscribe":
        client.shell = true;
        return { machine: { id: this.env.id, name: this.env.name }, threads: [...this.threads.values()] };
      case "thread.subscribe": {
        const t = this.thread(p);
        client.threads.add(t.id);
        // again after a reconnect: only what came after the last seq it saw
        const again = typeof p.after === "number";
        const after = again ? (p.after as number) : 0;
        const live = (this.live.get(t.id) ?? []).filter((e) => e.seq > after);
        return { thread: t, history: again ? [] : (this.history.get(t.id) ?? []), live, seq: this.seqs.get(t.id) ?? 0 };
      }
      case "thread.unsubscribe":
        client.threads.delete(String(p.threadId));
        return {};
      case "thread.create": {
        const folder = String(p.folder);
        const project = PROJECTS.find((x) => x.folder === folder);
        const ws = (p.workspace as { kind?: string; base?: string }) ?? { kind: "current" };
        const id = `t_${rand()}`;
        const text = String(p.text ?? "");
        const branch = ws.kind === "current" ? project?.branch : `pansy/${text.slice(0, 12).replace(/\W+/g, "-") || "task"}`;
        const t: ThreadSummary = {
          id,
          agent: String(p.agent ?? "codex"),
          title: text.split("\n")[0].slice(0, 40) || "新任务",
          folder,
          workdir: ws.kind === "worktree" ? `/home/me/.pansy/worktrees/${project?.name ?? "x"}/${id}` : folder,
          branch,
          worktree: ws.kind === "worktree",
          base: ws.base,
          model: String(p.model ?? (p.agent === "claude-code" ? "deepseek/v4" : "group/daily")),
          effort: p.effort as string | undefined,
          mode: (p.mode as ThreadSummary["mode"]) ?? "full",
          status: "idle",
          updatedAt: new Date().toISOString(),
          source: "pansy",
        };
        this.threads.set(id, t);
        this.history.set(id, []);
        this.broadcast("shell.thread", t);
        setTimeout(() => void this.turn(id, text), 50);
        return { threadId: id };
      }
      case "message.dispatch": {
        const t = this.thread(p);
        const text = String(p.text ?? "");
        const mode = String(p.dispatchMode ?? "start");
        const run = this.runs.get(t.id);
        if (t.source === "terminal") this.change(t.id, { source: "pansy", id: t.id });
        if (!run) {
          void this.turn(t.id, text);
        } else if (mode === "steer") {
          this.emit(t.id, { type: "user", text });
        } else if (mode === "restart") {
          run.cancel = true;
          run.approval?.resolve("cancel");
          setTimeout(() => void this.turn(t.id, text), 100);
        } else {
          const q = this.queued.get(t.id) ?? [];
          q.push(text);
          this.queued.set(t.id, q);
          this.change(t.id, { queue: q.length });
        }
        return {};
      }
      case "runtime-request.respond": {
        const t = this.thread(p);
        const run = this.runs.get(t.id);
        if (run?.question && run.question.request === p.requestId) {
          const resolve = run.question.resolve;
          run.question = undefined;
          this.emit(t.id, { type: "resolved", request: String(p.requestId) });
          resolve(p.answers ? (p.answers as Record<string, string>) : null);
          return {};
        }
        if (!run?.approval || run.approval.request !== p.requestId) throw err("not_found", "no such request");
        const resolve = run.approval.resolve;
        this.emit(t.id, { type: "resolved", request: String(p.requestId) });
        if (p.feedback) this.emit(t.id, { type: "user", text: String(p.feedback) });
        resolve(p.decision as Decision);
        return {};
      }
      case "run.interrupt": {
        const t = this.thread(p);
        const run = this.runs.get(t.id);
        if (run) {
          run.cancel = true;
          if (run.approval) {
            this.emit(t.id, { type: "resolved", request: run.approval.request });
            run.approval = undefined;
          }
          // seeded turns have no script running: end them here
          setTimeout(() => this.runs.get(t.id) === run && this.stopped(t.id, "turn"), 50);
        }
        return {};
      }
      case "thread.update": {
        const t = this.thread(p);
        const patch: Partial<ThreadSummary> = {};
        for (const k of ["title", "model", "effort", "mode", "pinned", "archived", "unread"] as const) if (p[k] !== undefined) (patch as Record<string, unknown>)[k] = p[k];
        if ("snoozedUntil" in p) patch.snoozedUntil = (p.snoozedUntil as string | null) ?? undefined;
        this.change(t.id, patch);
        return { thread: this.threads.get(t.id) };
      }
      case "thread.delete": {
        const t = this.thread(p);
        this.threads.delete(t.id);
        this.broadcast("shell.removed", { threadId: t.id });
        return {};
      }
      case "models.list":
        return MODELS;
      case "projects.list":
        return { projects: PROJECTS };
      case "usage.summary":
        return usageOf(Math.min(90, Math.max(1, Number(p.days ?? 7))));
      case "git.status": {
        const t = this.thread(p);
        const onDefault = !t.branch || t.branch === "main";
        const status: GitStatus = {
          branch: t.branch ?? "main",
          upstream: `origin/${t.branch ?? "main"}`,
          ahead: onDefault ? 0 : 2,
          behind: 1,
          defaultBranch: "main",
          onDefault,
          worktree: !!t.worktree,
          files: [SESSIONS, USAGE, I18N].map((f) => ({ path: f.path, status: "modified", add: f.add, del: f.del })),
          pr: t.pr,
        };
        return status;
      }
      case "git.diff": {
        this.thread(p);
        const files = ({ turn: [SESSIONS, USAGE], lastTurn: [USAGE], uncommitted: [SESSIONS, USAGE, I18N], base: [SESSIONS, USAGE, I18N] } as Record<string, DiffFile[]>)[String(p.scope)] ?? [];
        return { files } satisfies Diff;
      }
      case "git.generate":
        this.thread(p);
        await this.ms(400);
        return p.kind === "pr"
          ? { title: "Sessions: each folder's spend this month", body: "Adds `usage::by_project` and shows the month's spend by each folder on the Sessions page." }
          : { title: "Sessions: each folder's spend this month, from usage by project", body: "" };
      case "git.run": {
        const t = this.thread(p);
        const action = String(p.action);
        if (action === "pull") {
          this.broadcast("git.progress", { threadId: t.id, step: "pull", status: "done", detail: "Fast-forward" });
          return { pull: "Fast-forward" };
        }
        const steps = ({ commit: ["commit"], push: ["push"], commit_push: ["commit", "push"], create_pr: ["pr"], commit_push_pr: ["commit", "push", "pr"] } as Record<string, string[]>)[action] ?? [];
        if (steps.includes("commit") && String(p.message ?? "").includes("[push fails]")) this.rejecting.add(t.id);
        for (const step of steps) {
          this.broadcast("git.progress", { threadId: t.id, step, status: "running", detail: step === "push" ? "pre-push hook…" : step === "pr" ? `${t.model} 在写说明…` : undefined });
          await this.ms(700);
          if (step !== "commit" && this.rejecting.delete(t.id)) {
            this.broadcast("git.progress", { threadId: t.id, step: "push", status: "failed", detail: "! [rejected] (fetch first)" });
            throw err("conflict", "! [rejected] pansy/sessions-cost -> pansy/sessions-cost (fetch first)");
          }
          this.broadcast("git.progress", { threadId: t.id, step, status: "done", detail: step === "push" ? "pre-push 通过" : undefined });
        }
        const out: { commit?: string; pr?: { number: number; url: string } } = {};
        if (steps.includes("commit")) out.commit = "a1c9e04b7d";
        if (steps.includes("pr")) {
          out.pr = { number: 17, url: "https://github.com/example/pansy/pull/17" };
          this.change(t.id, { pr: { ...out.pr, state: "open" } });
        }
        return out;
      }
      case "devices.list":
        return { devices: this.devices.map((d) => ({ ...d, current: d.id === client.deviceId })), requireApproval: this.requireApproval };
      case "devices.revoke": {
        const at = this.devices.findIndex((d) => d.id === p.deviceId);
        if (at < 0) throw err("not_found", "no such device");
        this.devices.splice(at, 1);
        return {};
      }
      case "devices.approve": {
        const d = this.devices.find((x) => x.id === p.deviceId);
        if (!d) throw err("not_found", "no such device");
        d.approval = "approved";
        return {};
      }
      // not in protocol v1's table: the switch the phone's Settings needs
      case "devices.setRequireApproval":
        this.requireApproval = !!p.requireApproval;
        return {};
      case "pair.create": {
        const secret = rand() + rand();
        return { secret, url: `${this.env.routes[0].url}/#pair=${secret}`, expiresAt: new Date(Date.now() + 600_000).toISOString() };
      }
      default:
        throw err("bad_request", `no method ${method}`);
    }
  }

  /** A socket to this home inside the app, as the platform's WebSocket would be. */
  socket(url: string): Socket {
    const ticket = /[?&]ticket=([^&]+)/.exec(url)?.[1] ?? "";
    let open = true;
    const client: Client = {
      shell: false,
      threads: new Set(),
      deviceId: this.takeTicket(decodeURIComponent(ticket)) ?? "d_phone",
      send: (msg) => setTimeout(() => open && s.onmessage?.(JSON.stringify(msg)), 0),
    };
    let stop = () => {};
    const s: Socket = {
      send: (data) => void this.receive(client, data),
      close: () => {
        if (!open) return;
        open = false;
        stop();
        setTimeout(() => s.onclose?.("closed"), 0);
      },
      onopen: null,
      onmessage: null,
      onclose: null,
    };
    setTimeout(() => {
      stop = this.connect(client);
      s.onopen?.();
    }, 0);
    return s;
  }
}

