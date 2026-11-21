// The phone app headless. Most tests run test/harness.tsx: the app with
// mock/fake.ts's scripted home behind its transport, paired already (a
// machine in Storage, cookie mode), its RPCs logged to Storage's
// `test.calls`. Pairing runs the app itself against stubbed HTTP.
import { expect, open, test, type App } from "nsh/test";

const BASE = "http://home.test:7268";
const SCOPES = ["access:read", "access:write", "orchestration:read", "orchestration:operate", "source-control:write"];
const MACHINE = {
  id: "m_5f2a0c",
  name: "家里的 Mac",
  capabilities: ["codex", "claude", "git"],
  routes: [{ kind: "lan", url: BASE }],
  deviceId: "d_phone",
  scopes: SCOPES,
  approval: "approved",
  auth: { mode: "cookie" },
};

const launch = (options: Record<string, unknown> = {}) => open("./test/harness.tsx", { storage: { "pansy.machines": JSON.stringify([MACHINE]) }, ...options });

type Call = { method: string; params: Record<string, unknown> };
const calls = (app: App): Call[] => JSON.parse(app.storage()["test.calls"] ?? "[]");
const called = (app: App, method: string, match: (p: Record<string, unknown>) => boolean = () => true) => calls(app).some((c) => c.method === method && match(c.params));

test("lists every machine's threads from the shell snapshot, on their shelves", async () => {
  const app = await launch();
  await app.waitFor("text:给 Sessions 页加按项目的花费");
  const texts = app.texts();
  expect(texts).toContain("In progress");
  expect(texts).toContain("Awaiting approval");
  expect(texts).toContain("Failed");
  expect(texts).toContain("cargo test：3 个测试失败（pansy-agents）");
  expect(texts).toContain("Opened in a terminal");
  expect(texts).toContain("deepseek/v4");
  // archived threads are folded under their shelf's head
  expect(texts).toContain("Archived (3)");
  expect(texts).not.toContain("NSH 0.6.1 的升级");
  await app.press("id:shelf-archived");
  await app.waitFor("text:NSH 0.6.1 的升级");
  expect(called(app, "shell.subscribe")).toBe(true);
});

test("filters the list by agent", async () => {
  const app = await launch();
  await app.waitFor("text:给 Sessions 页加按项目的花费");
  await app.press("id:filter-agent");
  await app.waitFor("id:pick-agent-codex");
  await app.press("id:pick-agent-codex");
  await app.waitUntil(() => !app.texts().includes("给 Sessions 页加按项目的花费"));
  expect(app.texts()).toContain("修复 Codex 子 agent 的模型头");
});

test("the approval card answers with runtime-request.respond, and the turn goes on", async () => {
  const app = await launch();
  await app.press("id:thread-t_cost");
  await app.waitFor("id:approval-card");
  expect(app.texts()).toContain("Run a command");
  expect(app.texts()).toContain("cargo test -p pansy-core usage::");
  await app.press("id:approve-once");
  await app.waitUntil(() => called(app, "runtime-request.respond", (p) => p.threadId === "t_cost" && p.requestId === "r_test" && p.decision === "accept"));
  await app.waitFor("text:测试都通过了（42 个）。可以提交了。");
  await app.waitUntil(() => !app.shows("id:approval-card"));
});

test("a line typed while an approval waits declines it with that line", async () => {
  const app = await launch();
  await app.press("id:thread-t_cost");
  await app.waitFor("id:approval-card");
  await app.type("id:composer-input", "先别跑，改用 cargo check");
  await app.press("id:send");
  await app.waitUntil(() => called(app, "runtime-request.respond", (p) => p.decision === "decline" && p.feedback === "先别跑，改用 cargo check"));
  await app.waitFor("text:好，不跑测试了。");
});

test("while a turn runs the composer steers it, held it queues, and stop interrupts", async () => {
  const app = await launch();
  await app.press("id:thread-t_sub");
  await app.waitFor("id:capsule");
  await app.waitFor("label:Steer");
  expect(app.texts()).toContain("Queue 1");
  await app.type("id:composer-input", "用 deepseek/v4 再试一次");
  await app.press("id:send");
  await app.waitUntil(() => called(app, "message.dispatch", (p) => p.threadId === "t_sub" && p.dispatchMode === "steer" && p.text === "用 deepseek/v4 再试一次"));
  await app.longPress("id:send");
  await app.waitFor("label:Queue");
  await app.type("id:composer-input", "然后更新 compat.md");
  await app.press("id:send");
  await app.waitUntil(() => called(app, "message.dispatch", (p) => p.dispatchMode === "queue" && p.text === "然后更新 compat.md"));
  await app.press("id:stop");
  await app.waitUntil(() => called(app, "run.interrupt", (p) => p.threadId === "t_sub"));
  await app.waitFor("label:Send");
});

test("a new message streams in, asks for approval and finishes", async () => {
  const app = await launch();
  await app.press("id:thread-t_relay");
  await app.waitFor("text:把 relay 的部署整理一下，现在一个脚本又建 Worker 又发证书");
  await app.type("id:composer-input", "拆吧");
  await app.press("id:send");
  await app.waitUntil(() => called(app, "message.dispatch", (p) => p.dispatchMode === "start"));
  await app.waitFor("id:approval-card", { timeout: 15_000 });
  expect(app.texts().some((t) => t.includes("by_project"))).toBe(true);
  await app.press("id:approve-session");
  await app.waitUntil(() => called(app, "runtime-request.respond", (p) => p.decision === "acceptForSession"));
  await app.waitFor("text:测试都通过了（42 个）。可以提交了。", { timeout: 15_000 });
});

test("a question is answered with the choice made", async () => {
  const app = await launch();
  await app.press("id:thread-t_relay");
  await app.waitFor("text:把 relay 的部署整理一下，现在一个脚本又建 Worker 又发证书");
  await app.type("id:composer-input", "问我用哪个数据库");
  await app.press("id:send");
  await app.waitFor("text:Which database?", { timeout: 10_000 });
  expect(app.texts()).toContain("A QUESTION FOR YOU");
  await app.press("id:question-option-SQLite");
  await app.press("id:question-send");
  await app.waitUntil(() => called(app, "runtime-request.respond", (p) => (p.answers as Record<string, string>)?.["Which database?"] === "SQLite"));
  await app.waitFor("text:Going with SQLite.");
});

test("pulls the upstream from the Git sheet", async () => {
  const app = await launch();
  await app.press("id:thread-t_cost");
  await app.waitFor("id:open-git >> nth=0");
  await app.press("id:open-git >> nth=0");
  await app.waitFor("id:git-pull");
  expect(app.texts()).toContain("origin/pansy/sessions-cost has 1 new commits");
  await app.press("id:git-pull");
  await app.waitUntil(() => called(app, "git.run", (p) => p.action === "pull"));
  await app.waitFor("text:Fast-forward");
});

test("the model sheet lists groups and providers and changes the thread's model", async () => {
  const app = await launch();
  await app.press("id:thread-t_relay");
  await app.waitFor("id:model-pill");
  await app.press("id:model-pill");
  await app.waitFor("id:model-group/cheap");
  expect(app.texts()).toContain("Routing groups");
  expect(app.texts()).toContain("DeepSeek");
  await app.press("id:model-group/cheap");
  await app.waitUntil(() => called(app, "thread.update", (p) => p.model === "group/cheap"));
  await app.waitFor("text:cheap");
});

test("commits and pushes from the Git sheet, step by step", async () => {
  const app = await launch();
  await app.press("id:thread-t_cost");
  await app.waitFor("id:open-git >> nth=0");
  await app.press("id:open-git >> nth=0");
  await app.waitFor("id:git-commit-push");
  expect(app.texts()).toContain("pansy/sessions-cost");
  await app.press("id:git-commit-push");
  await app.waitFor("id:commit-generate");
  await app.press("id:commit-generate");
  await app.waitUntil(() => (app.value("id:commit-message") ?? "").startsWith("Sessions: each folder's spend"));
  await app.press("id:commit-run");
  await app.waitFor("id:step-push-done", { timeout: 10_000 });
  expect(called(app, "git.run", (p) => p.action === "commit_push" && (p.files as string[]).length === 3)).toBe(true);
});

test("the diff shows a scope, and held lines go into the composer as a comment", async () => {
  const app = await launch();
  await app.press("id:thread-t_cost");
  await app.waitFor("id:open-diff >> nth=0");
  await app.press("id:open-diff >> nth=0");
  await app.waitFor("text:desktop/ui/Sessions.tsx");
  expect(app.texts()).not.toContain("desktop/ui/i18n.ts");
  await app.press("id:diff-scope-uncommitted");
  await app.waitFor("text:desktop/ui/i18n.ts");
  expect(called(app, "git.diff", (p) => p.scope === "turn")).toBe(true);
  expect(called(app, "git.diff", (p) => p.scope === "uncommitted")).toBe(true);
  await app.longPress("id:diff-line-desktop/ui/Sessions.tsx-3");
  await app.press("id:diff-line-desktop/ui/Sessions.tsx-4");
  await app.waitFor("label:Comment on 2 lines");
  await app.press("id:diff-comment");
  await app.waitUntil(() => (app.value("id:composer-input") ?? "").includes("<Spend value={cost(props.folder)} />"));
});

test("starts a new task in a project and opens it", async () => {
  const app = await launch();
  await app.waitFor("text:给 Sessions 页加按项目的花费");
  await app.press("id:new-task");
  await app.waitFor("text:Do something in pansy?");
  await app.press("id:new-task-workspace");
  await app.press("id:ws-worktree");
  await app.press("id:sheet-close");
  await app.type("id:composer-input", "给 README 加一张深色截图");
  await app.press("id:send");
  await app.waitUntil(() => called(app, "thread.create", (p) => p.folder === "/home/me/pansy" && (p.workspace as { kind: string }).kind === "worktree" && p.text === "给 README 加一张深色截图"));
  await app.waitFor("id:thread-title");
  await app.waitUntil(() => called(app, "thread.subscribe", (p) => String(p.threadId).startsWith("t_") && p.threadId !== "t_cost"));
});

test("settings show a machine's routes, scopes and devices, and revoke one", async () => {
  const app = await launch();
  await app.press("id:tab-settings");
  await app.waitFor(`id:machine-${MACHINE.id}`);
  await app.press(`id:machine-${MACHINE.id}`);
  await app.waitFor("id:devices");
  expect(app.texts()).toContain("home.test:7268");
  expect(app.texts()).toContain("In use");
  await app.press("id:revoke-d_ipad");
  await app.waitUntil(() => called(app, "devices.revoke", (p) => p.deviceId === "d_ipad"));
  await app.press("id:device-approval");
  await app.waitUntil(() => called(app, "devices.setRequireApproval", (p) => p.requireApproval === true));
});

test("with nothing paired, it asks to connect a computer and pairs by address and code", async () => {
  const env = { id: "m_new", name: "Workstation", version: "0.2.0", protocol: 1, capabilities: ["codex", "claude"], routes: [{ kind: "lan", url: BASE }] };
  const app = await open("./App.tsx", {
    network: {
      [`GET ${BASE}/.well-known/pansy/environment`]: { body: env },
      [`POST ${BASE}/api/auth/pair`]: (req) => {
        const body = JSON.parse(req.body ?? "{}");
        return body.secret === "s3cret-code" ? { body: { deviceId: "d_1", accessToken: "tok_1", expiresIn: 3600, scopes: SCOPES, approval: "approved" } } : { status: 403, body: { error: { code: "forbidden", message: "expired" } } };
      },
      [`POST ${BASE}/api/auth/ws-ticket`]: { body: { ticket: "tk_1" } },
    },
  });
  await app.waitFor("id:empty-machines");
  expect(app.texts()).toContain("Connect a computer");
  await app.press("id:empty-machines-button");
  await app.waitFor("id:pair-address");
  await app.type("id:pair-address", "home.test");
  await app.type("id:pair-code", "wrong");
  await app.press("id:pair-submit");
  await app.waitFor("id:pair-error");
  await app.clear("id:pair-code");
  await app.type("id:pair-code", "s3cret-code");
  await app.press("id:pair-submit");
  await app.waitFor("id:pair-done");
  expect(app.texts()).toContain("Workstation");
  const pairing = app.requests().filter((r) => r.method === "POST" && r.url === `${BASE}/api/auth/pair`);
  expect(pairing.length).toBe(2);
  const sent = JSON.parse(pairing[1].body ?? "{}");
  expect(sent.secret).toBe("s3cret-code");
  // WebCrypto here: a DPoP proof, not the cookie
  expect(sent.cookie).toBe(false);
  expect(/^ey[\w-]+\.ey[\w-]+\.[\w-]+$/.test(pairing[1].headers.dpop ?? "")).toBe(true);
  await app.waitUntil(() => (app.storage()["pansy.machines"] ?? "").includes("m_new"));
  const kept = JSON.parse(app.storage()["pansy.machines"]);
  expect(kept[0].auth.mode).toBe("dpop");
  expect(kept[0].auth.token).toBe("tok_1");
});

test("speaks Chinese when the system does", async () => {
  const app = await launch({ locale: "zh-CN" });
  await app.waitFor("text:给 Sessions 页加按项目的花费");
  expect(app.texts()).toContain("进行中");
  expect(app.texts()).toContain("等批准");
  expect(app.texts()).toContain("已归档 (3)");
});
