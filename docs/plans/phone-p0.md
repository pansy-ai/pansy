# P0 结论：手机控制 agent 的前提验证

**状态：P0 做完，2026-11-21。** 方案见 [phone.md](phone.md)，界面见
[phone-ui.md](phone-ui.md)。脚本在 `spikes/remote/`，都在临时目录里跑，
不读不写用户的 agent 文件；agent 都经本机正在跑的 pansy 网关
（`127.0.0.1:7267`），和 `pansy run` 指过去的方式一样。

| 项 | 结论 |
| :-- | :-- |
| Codex app-server | **通过**：起线程、批准命令、插话、打断、新进程里续接 |
| Claude Code 经 Agent SDK | **通过**，Node 和 pansy 的 Bun 都行：批准、会话中换模型（换到另一家）、打断、续接 |
| Agent SDK 的条款 | **未定**：许可是 Anthropic 的专有条款；claude.ai 登录的限制没读到原文 |
| NSH 的 Web 包 | **通过**：WebSocket、ECDSA（DPoP）、存储、主屏幕清单；Service Worker 要经顶层页面注册；推送权限要真机试 |
| Clerk、Google、GitHub 在大陆 | **没法在这台电脑上判断**：所有流量都走 Clash |
| 待查的 agent | MothX 有 ACP；5 个是 OpenCode 或 Pi 的分支；其余没有或没查到 |

## 1. Codex（`spikes/remote/codex.mjs`）

Codex 0.171.0，模型 `default/gpt-6.1-terra`，经网关的 Responses。

```
thread/start {cwd, approvalPolicy: "untrusted", sandbox: "workspace-write"}
turn/start  → item/commandExecution/requestApproval
              {command: "/usr/bin/bash -lc 'touch approved.txt'", cwd, commandActions, …}
            ← {decision: "accept"}            → 文件建了，turn/completed: completed
turn/start  → turn/steer {expectedTurnId}     → ok
            → turn/interrupt {threadId, turnId} → turn/completed: interrupted
新进程 thread/resume {threadId}               → 2 个回合都在；问"刚才建了什么文件"答 approved.txt
```

- 见到的通知：`thread/started`、`thread/status/changed`、`turn/started`、
  `item/started`、`item/completed`、`item/agentMessage/delta`、
  `item/commandExecution/requestApproval`、`serverRequest/resolved`、
  `thread/tokenUsage/updated`、`account/rateLimits/updated`、
  `turn/moderationMetadata`、`thread/readState/changed`、`thread/goal/cleared`、
  `turn/completed`。
- 审批的回答还有 `acceptForSession`、`decline`、`cancel` 和两种带规则修改的；
  T3 的五种回答都对得上。
- **换模型不用重起进程**：`turn/start` 自带 `model`、`effort`、
  `approvalPolicy`、`sandboxPolicy`（生成的类型里有，没实测）。
- 坑：`CODEX_HOME` 在 `/tmp` 下时它不建辅助程序（只是警告）。实际要用用户
  自己的 `CODEX_HOME`，这样会话写进用户的会话目录，pansy 的 `sessions/` 照样
  列得出来。
- 类型：`codex app-server generate-ts --out <dir>` 生成 700 多个文件，可以按
  版本生成 Rust 类型。

## 2. Claude Code（`spikes/remote/claude.mjs`）

Agent SDK 0.3.330，用户的 `claude` 2.1.330（`pathToClaudeCodeExecutable`），
起始模型 `nofx/claude-fable-5`。在 Node 26.13 和 pansy 已装的 Bun 1.4.2
（`~/.config/pansy/bun/1.4.2/`）上结果一样：

| 步骤 | 结果 |
| :-- | :-- |
| 流式输入（一个 AsyncIterable 一条条喂） | 一个进程里多个回合 |
| `canUseTool` | Bash 前被调用，参数里有命令和说明；返回 allow 后执行 |
| `setModel("default/gpt-6.1-terra")` | 会话中途换到**另一家**的模型，经网关翻译，在 Bun 上流式输出了 47 段 |
| `interrupt()` | 回合以 `error_during_execution` 结束 |
| 新 query `resume: sessionId` | 同一个会话 id，记得前面建的文件 |

坑，驱动要照着处理：

- **打断后 SDK 会抛错**（"Claude Code returned an error result"），整个
  query 结束。打断后的下一句要用 `resume` 新起一个 query。
- 安装：`npm install --omit=optional @anthropic-ai/claude-agent-sdk`。不加会多
  下一个 142 MB 的内置 `claude`；我们用用户自己的，用不到。SDK 本身 5 MB，加上
  依赖约 31 MB。用 npmmirror 4 秒装完；从 npmjs 连内置 `claude` 一起装，两分钟
  没装完。
- 环境里要删掉 `ANTHROPIC_API_KEY`，换成网关的 `ANTHROPIC_BASE_URL` 和
  `ANTHROPIC_AUTH_TOKEN`。
- 网关上 `nofx/claude-fable-5-1` 和 `nofx/claude-opus-5-5` 当时都回 502（上游的
  问题），`setModel` 会把它当错误抛出来。手机上换模型失败时要能看到这一句。

## 3. Agent SDK 的条款

- `LICENSE.md`：© Anthropic PBC，保留所有权利，使用受
  `code.claude.com/docs/en/legal-and-compliance` 的协议约束。README 指向
  Commercial Terms 和隐私政策，并说 SDK 会收集反馈和用量数据。
- **不能随 pansy 一起发。** 照插件的做法，运行时用 pansy 的 Bun 从 npm 装到
  `PANSY_HOME`，钉版本。
- **claude.ai 登录的限制没读到原文。** 这台机器连不上
  `code.claude.com`。印象里文档说第三方产品未经批准不能提供 claude.ai 登录，
  要用 API key，但没有核对原文，不能当结论。pansy 驱动的是用户自己装、自己
  登录的 `claude`，多数时候经网关用 API key；用户直接用订阅时算不算，**要读
  原文再定**（T3 也这样驱动，可参考它的说法）。

## 4. NSH 的 Web 包（`/tmp/p0/nshweb`，没进仓库）

`nsh package --target web` 出 5.6 MB 的静态站（wasm 加 JS），自带
`manifest.webmanifest`（`display: standalone`，可加到主屏幕）。

| 探的 | 结果 |
| :-- | :-- |
| 应用代码跑在哪 | 一个同源的 `about:blank` iframe 里，`top.location` 读得到 |
| WebSocket | 通，要在 `nsh.json` 写 `capabilities.network.hosts` |
| `crypto.subtle` ECDSA P-256 不可导出密钥签名 | 通，64 字节签名：Web 版也能做 DPoP，不必退回 cookie |
| `Storage` | 通 |
| Service Worker | 在应用的 iframe 里注册报 `InvalidStateError`；**经 `top.navigator.serviceWorker` 在顶层页面注册成功**（同源） |
| Web Push 订阅 | `pushManager` 在；订阅要通知权限，测试用的内嵌浏览器默认拒绝。**要在真手机上试**（iOS 必须先加到主屏幕） |

- 所以 Web Push 做法是：pansy 送出 Web 包时，在同一目录放 `sw.js`；应用经
  `top` 注册、订阅，把订阅交给家里的 pansy，由 pansy 用自己的 VAPID 密钥直接发。
- 要加 CSP 时需要 `script-src 'self' 'wasm-unsafe-eval'`（`nsh package` 的
  提示）。

## 5. 大陆的连通性

这台电脑的流量全部经 Clash Verge（mihomo，假 IP DNS，`2001:2::/48`）。
Clerk 的 API、Google、GitHub、Apple、Resend、npmjs 都能通，但**说明不了没有
代理的大陆网络**。要在一部不开代理的手机上（蜂窝网络）试：

- Clerk 的前端接口（`clerk.<我们的域名>` CNAME 到 `frontend-api.clerk.services`）
- `accounts.google.com`、`github.com/login/oauth`、`appleid.apple.com`

只有 npmmirror 明显更快（0.1 秒），说明这台电脑在大陆。

## 6. 待查的 agent（看 README 和包信息，没装没跑）

| agent | 结论 | 依据 |
| :-- | :-- | :-- |
| MothX | **有 ACP** → 本地 ACP 命令 | npm README："ACP protocol for VS Code, JetBrains" |
| DevEco Code、MiMo Code | 很可能有 `acp` 和 `serve` → 照 OpenCode | 都是 OpenCode 的分支（清单里写明） |
| OmO、Prime Agent、mu | 很可能有 `--mode rpc` → 照 Pi | 都是 Pi 的分支或改名 |
| miniQ | 有自己的守护进程，WebSocket 上的 JSON-RPC 2.0 | README 的架构图；要专门写驱动，不排期 |
| QwenPaw | **不是这个 ACP**：它的是 Agent Communication Protocol，另有 HTTP API | README |
| Snow CLI、PenguinHarness、Paimon | 只有 headless（单次） | README；没有审批，不做 |
| Crush、Hermes Agent、OpenClaw、nanobot | README 里没有 ACP 或 RPC | 不支持，除非以后加 |
| AtomCode、Command Code、Empryo、jcode、Reasonix、StepCode、Ante、Mister Morph | 没查到 | 没找到公开的 README，留到有人要时再查 |

"很可能"的五个，二期接 ACP 和 Pi 时实测。

## 对方案的改动

1. **Claude Code 走 Agent SDK、跑在 pansy 的 Bun 上**：照旧，已验证。SDK 运行时
   装、不随包发。
2. Codex 换模型、换权限用 `turn/start` 的字段，不重起进程。
3. 驱动要把"打断后 SDK 抛错"当成回合结束，下一句用 `resume` 新起。
4. Web 版也用 DPoP（WebCrypto 可用），cookie 只给纯 HTTP 的局域网页面。
5. Web Push 经顶层页面注册 Service Worker；真机上试订阅，iOS 要先加到主屏幕。
6. 还要在真手机上试的：大陆网络下的 Clerk 和各家登录、Web Push 的权限和送达。
7. 还要读原文的：Agent SDK 关于 claude.ai 登录的条款。
