# 协议：手机和家里的 pansy（v1）

**状态：一期的约定。** 服务端是 `crates/pansy-remote`（`server.rs`），客户端是
`mobile/`。命令的名字照 T3 Code（phone.md "协议"）。两边按这一页写，改动先
改这一页。

## 连接

- 远程接口单独一个端口，默认 `7268`，默认只听回环；`pansy remote on` 后听
  所有接口，只收回环、私网（10/8、172.16/12、192.168/16、fc00::/7）和 tailnet
  （100.64/10）来的连接。**永远不和网关（7267）共用端口。**
- 同一个端口上：`/` 是手机的 Web 版（`mobile/` 的 `nsh package --target web`），
  `/api/…` 是 HTTP，`/ws` 是 WebSocket，`/.well-known/pansy/environment` 公开。

### 公开的描述

`GET /.well-known/pansy/environment`（不鉴权）

```json
{"id": "m_5f2a…", "name": "家里的 Mac", "version": "0.2.0", "os": "macos", "protocol": 1,
 "capabilities": ["codex", "claude", "git", "github"],
 "routes": [{"kind": "lan", "url": "http://192.168.1.20:7268"},
            {"kind": "tailnet", "url": "http://100.88.1.2:7268"}]}
```

客户端连之前先读它；`id` 和存着的不一样就不把凭据发过去。

## 鉴权（`auth.rs`）

两种会话，同一张设备表：

| | DPoP（app 和 HTTPS / localhost 上的 Web） | cookie（局域网纯 HTTP 的 Web，没有 WebCrypto） |
| :-- | :-- | :-- |
| 凭据 | `Authorization: DPoP <token>` 加 `DPoP: <proof>` | `pansy_session` cookie，`HttpOnly`、`SameSite=Strict` |
| 绑定 | 设备的 P-256 公钥指纹（RFC 7638 `jkt`） | 无 |

DPoP 证明：JWT，header `{"typ":"dpop+jwt","alg":"ES256","jwk":{"kty":"EC","crv":"P-256","x","y"}}`，
payload `{"jti","htm","htu","iat","ath"?}`。`htu` 不带 query；`iat` 前后 60 秒内；
`jti` 在 5 分钟内只收一次；有令牌时 `ath` 是令牌 SHA-256 的 base64url。

### 配对

1. 电脑上生成配对：`pansy pair`、窗口，或已有 `access:write` 的设备
   `pair.create`。得到 32 字节随机的密钥，10 分钟有效，只显示一次；
   `remote.json` 只存它的 SHA-256。链接 `http(s)://<地址>/#pair=<base64url 密钥>`。
2. `POST /api/auth/pair`

   ```json
   {"secret": "…", "deviceName": "iPhone", "platform": "ios|android|web", "cookie": false}
   ```

   DPoP 方式带 `DPoP` 头（无 `ath`）；cookie 方式 `"cookie": true`，不带。
   回答：

   ```json
   {"deviceId": "d_…", "accessToken": "…", "expiresIn": 3600,
    "scopes": ["orchestration:read", "…"], "approval": "approved|pending", "code": "K7Q-42M"}
   ```

   cookie 方式不回 `accessToken`，回 `Set-Cookie`。配对密钥用一次就作废。
   设备批准开着（默认关）时 `approval` 是 `pending`，`code` 是公钥指纹（cookie
   方式是设备 id）得出的短码，令牌在电脑上批准之前除了 `GET /api/auth/me` 什么都
   不能做。
3. `POST /api/auth/refresh`（DPoP，令牌过期了也行）→ `{"accessToken","expiresIn"}`；
   会话最后一次用后 30 天作废。cookie 会话每次用顺延。
4. `GET /api/auth/me` → `{"deviceId","name","scopes","approval"}`。
5. `POST /api/auth/ws-ticket` → `{"ticket": "…"}`，30 秒、一次。
   WebSocket：`GET /ws?ticket=<ticket>`，DPoP 的再加 `&dpop=<proof>`（`htm`
   为 `GET`，`htu` 为 `/ws` 的地址）。

### 范围

`access:read` `access:write` `orchestration:read` `orchestration:operate`
`source-control:write` `filesystem:read` `terminal:read` `terminal:operate`
`settings:write` `providers:manage`。配对默认给全部（照 T3）；生成配对时可以
收窄。每个方法声明它要的范围（下表），没有就回 `forbidden`。

## WebSocket

一条连接，JSON 文本帧。

```
→ {"id": 7, "method": "thread.subscribe", "params": {...}}
← {"id": 7, "result": {...}}            或  {"id": 7, "error": {"code": "forbidden", "message": "…"}}
← {"event": "thread.event", "data": {...}}
```

错误码：`bad_request` `forbidden` `not_found` `conflict` `agent` `internal`。
服务端每 20 秒发 `{"event":"ping"}`；客户端 60 秒收不到任何东西就重连。

### 方法

| 方法 | 范围 | params | result |
| :-- | :-- | :-- | :-- |
| `shell.subscribe` | orchestration:read | `{}` | `{machine, threads: [ThreadSummary]}`，之后推 `shell.thread`、`shell.removed` |
| `thread.subscribe` | orchestration:read | `{threadId, after?}` | `{thread: ThreadSummary, history: [Turn], live: [Seq<Event>], seq, changes: [TurnChange]}`，之后推 `thread.event` |
| `thread.unsubscribe` | orchestration:read | `{threadId}` | `{}` |
| `thread.create` | orchestration:operate | `{folder, agent, model?, effort?, mode?, workspace: {kind: "current"\|"branch"\|"worktree", base?}, text}` | `{threadId}` |
| `message.dispatch` | orchestration:operate | `{threadId, text, dispatchMode: "start"\|"queue"\|"steer"\|"restart", model?, effort?}` | `{}` |
| `runtime-request.respond` | orchestration:operate | `{threadId, requestId, decision: "accept"\|"acceptForSession"\|"decline"\|"cancel", feedback?}`；问题用 `{threadId, requestId, answers: {问题: 选项}}`（多选用 ", " 连起来） | `{}` |
| `run.interrupt` | orchestration:operate | `{threadId, force?}` | `{}`（`force` 结束 agent 进程，下一条消息接着会话重开） |
| `thread.update` | orchestration:operate | `{threadId, title?, model?, effort?, mode?, pinned?, archived?, snoozedUntil?}` | `{thread: ThreadSummary}` |
| `thread.delete` | orchestration:operate | `{threadId, force?}` | `{worktreeKept}`（只删 pansy 的线程记录，不碰会话文件；pansy 建的工作树没有未提交的改动才一起删，`force` 照删；分支有未合并的提交就留着） |
| `models.list` | orchestration:read | `{agent?}` | `{groups: [Group], providers: [{id, name, models: [Model]}]}` |
| `projects.list` | orchestration:read | `{}` | `{projects: [{folder, name, git: bool, branch?}]}` |
| `usage.summary` | orchestration:read | `{days?}`（1 到 90，默认 7） | `{days, total: Totals, byDay: {日期: Totals}, byModel: {"provider/model": Totals}, byAgent: {agent: Totals}}`；`Totals` 是 `pansy_core::usage::Totals`（`requests, errors, input, output, cacheRead, cacheWrite, reasoning`），每个请求的明细不给 |
| `git.status` | orchestration:read | `{threadId}` | `GitStatus` |
| `git.diff` | orchestration:read | `{threadId, scope: "turn"\|"lastTurn"\|"uncommitted"\|"base", turn?}` | `Diff`。`turn`、`lastTurn` 只有 pansy 建的工作树有（检查点只在那里），别处是 `bad_request`；`turn` 带 `turn: n` 是第 n 回合，不带时回合进行中是从上个检查点到现在，否则是上一回合 |
| `git.generate` | source-control:write | `{threadId, kind: "commit"\|"pr"}` | `{title, body}` |
| `git.run` | source-control:write | `{threadId, action: "commit"\|"push"\|"commit_push"\|"create_pr"\|"commit_push_pr"\|"pull", files?, message?, pr?: {title?, body?, base?}}` | `{commit?, pr?: {number, url}, pull?}`；进度推 `git.progress`；`pull` 只快进上游 |
| `devices.list` | access:read | `{}` | `{devices: [Device], requireApproval}` |
| `devices.setRequireApproval` | access:write | `{requireApproval}` | `{}` |
| `devices.revoke` / `devices.approve` | access:write | `{deviceId}` | `{}` |
| `pair.create` | access:write | `{scopes?}` | `{secret, url, expiresAt}` |

### 推送的事件

| event | data |
| :-- | :-- |
| `shell.thread` | `ThreadSummary`（新的或变了的） |
| `shell.removed` | `{threadId}` |
| `thread.event` | `{threadId, seq, event: Event}`；`seq` 每个线程单调增 |
| `git.progress` | `{threadId, step: "commit"\|"push"\|"pr", status: "running"\|"done"\|"failed", detail?}` |
| `ping` | `{}` |

### 类型

```ts
type ThreadSummary = {
  id: string;                 // "t_…"；终端里开的、还没接管的会话："s:<agent>:<sessionId>"
  agent: "codex" | "claude-code" | string;
  title: string;
  folder: string;             // 项目文件夹（工作树线程是原仓库）
  workdir: string;            // agent 实际跑的目录（工作树或项目）
  branch?: string; worktree?: boolean; base?: string;
  model: string; effort?: string; mode: "ask" | "edits" | "auto" | "full";
  status: "working" | "approval" | "input" | "failed" | "done" | "idle" | "unsent";
  startedAt?: string;         // 进行中的回合开始时（计时用）
  updatedAt: string;          // ISO 时间
  source: "pansy" | "terminal";
  session?: string;           // agent 自己的会话 id（续接用）
  pr?: { number: number; url: string; state: "open" | "closed" | "merged" };
  diff?: { add: number; del: number };
  pinned?: boolean; archived?: boolean; snoozedUntil?: string; unread?: boolean;
  error?: string;
  queue?: number;
};

type Turn = {                 // 历史：pansy_agents::sessions::timed_transcript
  role: "user" | "assistant" | "reasoning" | "tool" | "compact" | "end";
  name?: string; text: string; input?: string; output?: string; status?: string; at?: string;
  ms?: number;                // end：回合用时，agent 写了才有（Codex 的 task_complete / turn_aborted）
};                            // end 的 status 是 "stopped" 时是用户停的（Claude Code 只写这一种）

type Event =                  // pansy_remote::Event，tag 是 type
  | { type: "thread"; thread: string; model: string }
  | { type: "turnStarted"; turn: string }
  | { type: "delta"; item: string; text: string }
  | { type: "message"; item: string; text: string }
  | { type: "user"; text: string }                       // 发出去的一句（多端同步）
  | { type: "toolStarted"; item: string; kind: string; title: string }
  | { type: "toolFinished"; item: string; kind: string; title: string; status: string; output?: string; exit?: number }  // exit：命令的退出码（Codex 给）
      // kind 是 agent 自己的词，只有一个统一："subagent"（Claude Code 的 Task / Agent、Codex 的 spawnAgent），
      // title 是"名字 · 类型或 agent · 模型"，output 是它交回来的报告；Codex 的 plan 的 output 是计划全文
  | { type: "approval"; request: string; kind: string; title: string; detail: string; reason?: string }
  | { type: "question"; request: string; questions: { question: string; header: string; multiSelect: boolean;
      options: { label: string; description: string }[] }[] }
  | { type: "resolved"; request: string }
  | { type: "turnFinished"; turn: string; status: string; error?: string }
  | { type: "exited"; code?: number }
  | ({ type: "turnChanges" } & TurnChange);              // 工作树线程的回合改了东西时，紧跟它的 turnFinished，hub 发

type TurnChange = {           // 一回合改了什么：它的检查点对上一个；路径和行数，最多 100 个文件
  turn: number; at: string;   // at：结束的时间，在 agent 的历史里找它的那句话用
  add: number; del: number;
  files: { path: string; status: "added" | "modified" | "deleted" | "renamed"; add: number; del: number }[];
};

type Seq<T> = { seq: number; event: T };

type Group = { id: string; name: string; members: number; routing: string };
type Model = { id: string; name: string; context?: number; reasoning?: boolean; vision?: boolean };

type GitStatus = {
  branch: string; upstream?: string; ahead: number; behind: number;
  defaultBranch: string; onDefault: boolean; worktree: boolean;
  files: { path: string; status: "added" | "modified" | "deleted" | "renamed" | "untracked"; add: number; del: number }[];
  pr?: { number: number; url: string; state: string };
};

type Diff = { files: {
  path: string; oldPath?: string; status: string; add: number; del: number; binary?: boolean;
  hunks: { header: string; lines: { kind: "+" | "-" | " "; old?: number; new?: number; text: string }[] }[];
}[]; truncated?: boolean };

type Device = { id: string; name: string; platform: string; scopes: string[];
  approval: "approved" | "pending"; createdAt: string; lastSeen?: string; current?: boolean };
```

## 回合和线程（服务端的约定）

- 一个线程同时一个 agent 进程；`message.dispatch`：
  - `start`：没有进行中的回合时开一个；有则当 `queue`。
  - `queue`：排在进行中的回合后面，回合结束自动发；`queue` 计数在摘要里。
  - `steer`：Codex 用 `turn/steer`；Claude 打断再开（T3 也这样）。
  - `restart`：打断，然后马上发。
- 摘要的 `status`：有审批在等 → `approval`；回合进行中 → `working`；上一个回合
  失败 → `failed`；完成且没看过 → `done`；否则 `idle`。
- 进行中的回合的事件只在内存里，每个线程留最近 2000 条；重连带 `after`（最后
  的 `seq`）补齐。更早的从 agent 的会话文件读（`history`）。
- 进程空闲 30 分钟（没有回合、没有订阅）就收掉，下次发消息时 `resume`。
- 终端里开的会话（`sessions::list`）出现在列表里，`source: "terminal"`；发第一句
  时建线程记录并续接；会话文件最近一分钟被写过（`active`）时拒绝（`conflict`），
  说它可能正在别处运行。
