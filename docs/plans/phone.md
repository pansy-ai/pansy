# 计划：手机 app 控制家里的 agent（第二版，照 T3 Code）

**状态：P0 做完（[phone-p0.md](phone-p0.md)）；一期的功能在 `phone-remote` 分支上齐了，
本机端到端走通（Codex 和 Claude Code：配对、新任务进工作树、审批、提问、
本回合 diff、提交推送、拉取），等真机试（大陆网络、手机浏览器）。开 PR 还没
对真的 GitHub 仓库试过（只有 GitHub，别的 forge 在三期）。** 是
[roadmap.md](roadmap.md) 迭代 7 的方案；[remote.md](remote.md) 留着远程接口、
驱动和鉴权的细节，不一致时以本页为准；界面见 [phone-ui.md](phone-ui.md)。

依据：T3 Code `c7157d37`（server v0.1.11，MIT）的 `apps/mobile`、
`apps/server`、`packages/`、`docs/`；ACP 官方登记表；本机的 Claude Code
2.1.329、Codex 0.171、NSH 0.6.1。文中 `T3:` 后面是 T3 的出处。

## 一句话

手机上是 pansy 的 app（NSH TSX，iOS、Android、Web）；家里是 pansy 本身，
装成用户服务，多一个远程接口，替用户驱动各家 agent、管 git；云端一个
pansy Connect，管账号、设备批准、推送和隧道。pansy 不做自己的 agent。

## 原则：照 T3，例外写明

能照 T3 的都照 T3：功能、命令的形状、驱动方式、鉴权、云端的分工。以下是
不照的，各有原因：

| 处 | T3 | pansy | 原因 |
| :-- | :-- | :-- | :-- |
| 家里的实现 | Node / Effect，约 30 万行 | Rust，`pansy-remote` | pansy 是 Rust；只做手机用得到的部分 |
| 协议 | Effect RPC，约 235 个方法 | 自己的小协议，命令和字段照 T3 起名 | Effect RPC 在 Rust 和 QuickJS 上都没有现成实现 |
| 手机 app | Expo / React Native | NSH TSX | pansy 的窗口就是 NSH |
| 终端里开过的会话 | 导入（只 Claude、Codex、Pi，最多 200 条，无工具记录） | 直接列出，续接时才建线程 | `sessions/` 已经读得出全部，不必导入 |
| 提交说明、PR、分支名 | 起一个 agent 的无头模式来写 | 走 pansy 的网关，用户选模型 | 不必为一句话起一个 agent |
| 检查点 ref | 每个线程都写进用户仓库 | 只写在 pansy 建的工作树里 | 已定：不碰用户自己的分支 |
| 模型 | 各 agent 自己的 | 任何 agent 都能换到 pansy 的任何模型和路由组 | pansy 的本行 |

## 总体

```
手机 app（NSH：iOS / Android / Web）
   │ WebSocket + HTTP，DPoP 令牌
   ├──────── 局域网 / Tailscale 直连 ────────┐
   └── pansy Connect 的托管隧道 ──┐          │
                                  ▼          ▼
pansy Connect（云）           家里：pansy serve（用户服务）
 账号 · 设备批准 · 推送中继 ·    ├─ 网关 127.0.0.1:7267（不对外）
 隧道分配                        └─ pansy-remote（独立端口）
   ▲  推送状态                        ├─ 鉴权：配对、DPoP、范围
   └──────────────────────────────────├─ 线程、回合、驱动
                                      └─ git：工作树、检查点、提交、PR
                                            │ run::launch 给参数和环境
                                            ▼
                                      agent 进程 → 网关 → provider
```

## 家里：pansy-remote

### 用户服务

T3: `t3 service install`（`docs/user/background-service.md`）。

`pansy service install|uninstall|status`：Linux 的 systemd 用户单元（开
linger）、macOS 的 launchd、Windows 的计划任务，跑 `pansy serve`。窗口启动
时先连已在跑的服务，没有才自己起（roadmap 迭代 2 的这一部分先做）。没有
它，人一走，回合就没了。

有回合在跑时，电脑不因空闲睡着（`pansy-remote/src/awake.rs`，照 noop 的
`src/awake.ts`）：macOS 用 `caffeinate -i`，Linux 用 `systemd-inhibit
--what=sleep:idle`（被 polkit 拒了退到 `idle`），Windows 用
`SetThreadExecutionState`。最后一个回合结束就放开；pansy 怎么退出都会一起
放开。合盖照样睡（logind 默认 `LidSwitchIgnoreInhibited`）；Linux 上手动
挂起会先提示有程序阻止。

### 协议

T3: `packages/contracts/src/rpc.ts`、`orchestrationV2.ts`、
`docs/internals/connection-runtime.md`。

- 一条 WebSocket，JSON；请求、回复、订阅流。订阅两种：`subscribeShell`
  （线程列表和角标）、`subscribeThread`（一个线程的事件）。线程列表的首屏走
  HTTP 快照。
- 命令一个入口 `dispatchCommand`，名字照 T3：

| 命令 | 做什么 |
| :-- | :-- |
| `message.dispatch { text, attachments?, modelSelection?, dispatchMode }` | `dispatchMode`：`start_immediately`、`queue_after_active`、`steer_active`、`restart_active` |
| `runtime-request.respond { requestId, decision, feedback?, answers? }` | `decision`：`accept`、`acceptForSession`、`acceptAlways`、`decline`、`cancel`；提问的回答放 `answers` |
| `thread.user-input.dismiss` | |
| `run.interrupt { runId, holdQueue? }`、`thread.stop` | |
| `thread.model-selection.set`、`thread.runtime-mode.set`、`thread.interaction-mode.set` | 换模型、权限模式、plan 模式 |
| `queued-run.edit / reorder / cancel / start`、`queued-message.promote-to-steer` | 排队 |
| `thread.create`、`thread.rename`、`thread.archive`、`thread.pin`、`thread.settle`、`thread.snooze` | |
| `git.runStackedAction { action, files?, message? }` | 见"git" |

- 协议版本号加能力标志，客户端连上先读 `GET /.well-known/pansy/environment`
  （机器 id、版本、能力）。旧客户端连新服务要能用，反之降级。

### 线程

T3: 线程、运行、执行节点（`docs/orchestration-v2/README.md`）。

- `threads.json`（`PANSY_HOME`，0600）：线程 id、agent、agent 的会话 id、项目、
  工作树和分支、基线、PR、标题、置顶、settle、snooze。**没有正文**。
- 历史从 agent 自己的会话文件读（`sessions::transcript`）；进行中的回合事件
  只在内存里，按条数和字节封顶，断线重连按序号补。
- 线程的角标照 T3：Approval、Input、Working、Failed、Unsent。

### 驱动

T3: `apps/server/src/provider/builtInDrivers.ts`；接口
`packages/provider-core/src/server/ProviderAdapter.ts`（`ensureThread`、
`resumeThread`、`startTurn`、`steerTurn`、`interruptTurn`、
`respondToRuntimeRequest`、`applyModelSelection`、`forkThread`）。pansy 的
`Driver` 照这组方法。

| agent | 方式（照 T3） | 网关 | 期 |
| :-- | :-- | :-- | :-- |
| Codex | `codex app-server`，JSON-RPC；类型由 `codex app-server generate-json-schema` 生成，按版本钉 | 能 | 一期 |
| Claude Code（含 Claude Desktop 的 Code 会话） | `@anthropic-ai/claude-agent-sdk` 的 `query()`，跑在 pansy 的 Bun 宿主里，指向用户的 `claude`；`canUseTool` 管审批和提问，`setModel`、`setPermissionMode`、`interrupt()` | 能 | 一期 |
| OpenCode | `opencode serve`（HTTP + SSE），要 1.14.19 以上 | 能 | 二期 |
| Pi、mu | `pi --mode rpc` | 能 | 二期 |
| 登记表里的 ACP agent | 一个通用 ACP 驱动，v2 预览优先、v1 兜底；照 T3 读它对 Claude、Copilot、Devin、Droid、Kimi、OpenCode、OMP 的修补 | 能（pansy 有清单的） | 二期：Gemini CLI、Qwen Code、Copilot CLI；三期：其余 |
| Grok Build | ACP 加 xAI 扩展 | 能 | 三期 |
| fx | `fx acp` | 能 | 三期 |
| Antigravity | Google 的 ACP 程序 | 待验证 | 三期 |
| Kiro | `kiro-cli acp` | 先加清单 | 三期 |
| Muse Code | Muse 自己的协议（MSP，照 T3 的 `provider-muse`） | 能 | 三期 |
| Cursor | Cursor 的 Agent SDK | 不能（用 Cursor 的模型） | 三期，只控制 |
| 本地 ACP 命令 | 用户填程序和参数 | 能（`run::launch` 给环境） | 三期 |

pansy 清单里走 ACP 登记表的：Gemini CLI、Qwen Code、Copilot CLI、Goose、
Kimi Code、CodeBuddy、Droid、Qoder、Mistral Vibe、MiniMax Code。

T3 文档里写的坑照着防（`docs/internals/adding-a-provider.md` "Traps"、
`providers.md` "Protocol traps"）：回合还没开始就停止、续不上的会话、后台
任务和子 agent、额度用尽、符号链接的工作区、Codex 的异步提问要用新消息回答。

**不支持：** VS Code、VS Code Insiders、Copilot（JetBrains）这类编辑器扩展；
Grok App、LiveAgent、Pencil、OpenPencil、Tuanjie Cowork、WorkBuddy、
WorkBuddy AI、ZCode 这类桌面 app；T3 Code 本身；Claude Desktop 的 Chat 和
Cowork。其余命令行 agent（Crush、Hermes、OpenClaw 等 22 个）P0 查文档，有
ACP 的经"本地 ACP 命令"接入，没有的不支持；只有单次打印、没有审批的不做。

### 权限模式和审批

T3: `docs/user/permission-modes.md`。

- 四种模式照 T3：Supervised、Auto-accept edits、Auto、Full access；只给 agent
  自己执行得了的（Muse、Kiro 只有两种，fx、Grok 没有 Auto-accept edits）。
- 照 T3：最初的默认是 **Full access**，之后新线程沿用正在看的那个线程的模式；
  手机上可以改成任何一种。Full access 下 agent 不问就执行命令、改文件，审批
  卡片只在用户换到别的模式时出现。不给设备设"最宽能开到哪种模式"的上限：
  配对照 T3 给全部范围、含终端，有终端的设备本来就能做任何事，上限拦不住
  什么。
- 没有客户端在线时审批一直等着，不自动放行。

### git

T3: `ThreadLaunchService.ts`、`CheckpointService.ts`、`git/GitManager.ts`、
`packages/source-control-*`。

- **工作树。** 新任务三选一：当前分支、新分支、新工作树（选基线，可先 fetch
  远端）。位置 `PANSY_HOME/worktrees/<repo>/<分支前缀>-<hash>`，可在设置里改。
  分支先叫 `pansy/<8hex>`，第一条消息后由网关的模型起名改名。可选项目的
  setup 脚本。
- **清理**照 T3 的策略，默认关：不活动 N 天、settle 后、PR 合并后、没有改动、
  随线程删除。有未提交改动的不删；只碰 pansy 建的。
- **检查点。** 每个回合后，用临时 index `write-tree` / `commit-tree` 存到
  `refs/pansy/checkpoints/<线程>/<序号>`，不动分支和工作区；只在 pansy 建的
  工作树里。diff：本回合、最近回合、未提交、相对基线。回滚
  （`checkpoint.rollback`）只在 agent 能同时回滚对话时允许。
- **提交、推送、PR。** `git.runStackedAction`，`action` 为 `commit`、`push`、
  `create_pr`、`commit_push`、`commit_push_pr`，分阶段回报进度（含 hook
  输出）。说明和 PR 标题正文没给就由网关的模型写，可按仓库习惯或
  Conventional Commits。推送用机器自己的凭据。在默认分支上要确认。
- **forge。** GitHub 走 REST / GraphQL，令牌依次取 `GH_TOKEN`、
  `GITHUB_TOKEN`、`gh auth token`，pansy 不存。其余照 T3 走各自的 CLI：
  GitLab `glab`、Forgejo / Gitea `fj` / `tea`、Azure `az devops`。
- **PR 页**照 T3 的 `pullRequests.*`：检查、动态、评审、合并（merge、squash、
  rebase、自动合并）、改标题标签评审人、监视 PR 并在检查失败时叫醒 agent。

## 鉴权和路由

T3: `docs/internals/environment-auth.md`、`remote.md`。

- **配对**：`pansy pair` 或窗口生成一次性密钥，二维码和链接
  `https://<地址>/pair#<密钥>`（密钥在 `#` 后）；客户端用 RFC 8693 换令牌。
- **令牌**：DPoP（ES256 / P-256）绑定，1 小时；会话用一次顺延 30 天；
  WebSocket 先换 30 秒的票据；DPoP 证明无效不退回 bearer。局域网纯 HTTP 的
  Web 页没有 WebCrypto，用 cookie 形式的会话，且不能学到别的地址。
- **范围**照 T3：`access:read|write`、`orchestration:read|operate`、
  `source-control:write`、`filesystem:read`、`terminal:read|operate`、
  `settings:write`、`providers:manage`；每个命令声明它要的范围。
- **配对默认给全部范围**，照 T3。生成配对时可以收窄（比如只读、不给终端），
  换令牌时也只能收窄。窗口和文档上直说：配对过的设备能在这台电脑上做任何
  事，等于一个远程终端。
- **路由**：默认只听回环；开了只收私网和 tailnet 的连接；`tailscale serve`
  出 HTTPS；客户端存有序的地址表，连第一个通的，学服务端报来的局域网和
  tailnet 地址。远程接口永远不和网关共用端口。
- **设备**：一台一行，可吊销，吊销立即断开。

## 云端：pansy Connect

T3: `docs/internals/t3-connect.md`、`docs/user/remote-access.md`、
`mobile-notifications.md`。照 T3 Connect 的分工；它是 pansy 第一处云端，
roadmap 的原则 5 已为它写明例外；要有隐私政策和删号。

- **账号。** 照 T3 用托管的身份服务（Clerk）。五种登录 Clerk 都有：邮箱
  验证码、magic link（邮件链接）、GitHub、Google、Apple。
  - 手机上走系统浏览器（Google 不许内嵌 WebView 登录）：iOS 要
    `ASWebAuthenticationSession`，Android 用 `openUrl` 开 Custom Tabs，回到
    `pansy://`。
  - iOS 上架有第三方登录就必须有 Apple 登录（审核指南 4.8）；原生按钮要
    entitlement，`nsh package` 加不了，先用网页版。
  - GitHub 登录只要 `read:user`、`user:email`，和开 PR 的令牌无关。
  - 大陆用户：GitHub、Google 时好时坏，邮箱验证码和 Apple 是主路；Clerk 在
    大陆能否访问 P0 要试，不行就改自建（Cloudflare Workers 加开源的身份库）。
- **机器登记。** 家里的 pansy 也登录（窗口设置或 `pansy login`），把自己挂
  到账号下；凭据进 `keyStore`。
- **设备批准。** 照 T3，**默认关**：登录了账号的设备直接能连账号下的电脑。
  开了之后（每台电脑的设置里，T3 的 `requireConnectDeviceApproval`），新设备
  第一次连，要在那台电脑上或一台已有 `access:write` 的设备上核对由设备公钥
  指纹得出的短码并批准；按密钥批准，不过期，可吊销。打开时已连过的设备算
  已批准。
  - 默认关意味着**账号就是钥匙**：邮箱或 GitHub、Google、Apple 账号被盗，
    家里的电脑就被控。设置页和登录页上说明，并在账号设置里提示用户开两步
    验证；"设备批准"放在设置里显眼处。
- **托管隧道。** 照 T3：中继在 pansy 的 Cloudflare 账号下为每台机器分配一条
  隧道，家里跑 `cloudflared`，隧道只指向远程接口的回环端口，永远不指向网关。
  应用流量走隧道，不经中继。也可以用用户自己的命名隧道（remote.md）。
- **推送中继。** 家里把线程的状态（运行、完成、失败、等审批、等回答）用机器
  签名发给中继，中继经 APNs 和 FCM 发到手机，含 iOS 的 Live Activities 和
  Android 的 Live Updates。负载用设备公钥加密，中继看不到线程标题。app 在
  前台时不弹。点开进对应线程。
- 部署：Cloudflare Workers（发版已在 Cloudflare 上），`cloud/` 自成一处。

## 手机 app

`mobile/`，自成工作区，NSH TSX。NSH 0.6.1 对照 T3 用到的原生能力：

| T3 用的 | NSH 0.6.1 | 办法 |
| :-- | :-- | :-- |
| WebSocket、深链、分享、剪贴板、触感、标签栏、本地通知、语音输入 | 有 | |
| DPoP 签名 | `crypto.subtle` 有 ECDSA | |
| SecureStore（Keychain / Keystore） | 没有 | app 自己的 Rust crate（`nsh init --template mixed`，iOS、Android 都支持） |
| 相机扫码 | 没有 | 一期用系统相机扫码打开链接；以后自己的 crate |
| 系统登录窗口 | 没有 | iOS 自己的 crate 包 `ASWebAuthenticationSession` |
| 推送令牌（APNs / FCM） | **没有，自己的 crate 也补不了**：iOS 的 `AppDelegate` 不收设备令牌、`nsh package` 加不了 `aps-environment`；Android 打不进 Firebase | **要 NSH 改**，在此之前原生推送做不了 |
| 原生终端、diff、高亮、富文本编辑器 | 没有 | diff 和高亮用 TSX 画；终端用 TSX 画一个简单的 |
| 小组件、Live Activities | 没有 | 等 NSH |

Web 版由家里的 pansy 在 `/` 上送出；Web Push（主屏幕上的 Web 版，iOS 16.4
起）由家里直接发，不经中继，是原生推送到来之前的推送。

### 页面（照 T3 的 `Stack.tsx`）

| T3 | 期 |
| :-- | :-- |
| 首页标签：Chats、PRs、Usage（T3 另有 Issues，不做） | Chats、Usage（网关的用量，`usage.summary`）一期；PRs 不做标签，开 PR 在 Git 里 |
| Chats：所有机器的线程，Working / Settled / Snoozed 三栏，按机器和项目筛，角标 | 一期；搜索、置顶、贪睡、settle、未读、撤销 二期 |
| 机器：列表、配对（链接、扫码、地址加码）、路由、"你的权限"、移除 | 一期 |
| 新任务：项目 → 机器 → 分支（当前 / 新分支 / 新工作树）→ 线程设置 | 一期；添加项目（克隆、本地文件夹）二期 |
| 线程：Markdown、工具和命令输出的折叠日志、子 agent、流式 | 一期；Mermaid、数学、图片、PDF 二期 |
| 审批卡片（允许一次 / 本会话 / 总是 / 拒绝、拒绝附理由） | 一期 |
| 提问卡片 | 一期 |
| 计划卡片、plan 模式 | 二期 |
| 输入框：发送 / 排队 / 插话 / 重开、停止、强制停止；模型选择（含 pansy 的路由组）、权限模式 | 一期 |
| 排队面板（立即发、编辑、删除） | 二期 |
| 附件、语音、斜杠命令和技能、@ 文件 | 二期；斜杠命令三期 |
| 从某条回答分叉、side chat | 三期 |
| Git：状态、提交（选文件、自动说明）、推送、开 PR、pull | 一期；分支切换、合回源分支 二期 |
| Review：本回合 / 最近回合 / 未提交 / 全部；行评论进输入框 | 一期 |
| PR 页：概览、文件、检查、评审、合并、"修复失败的检查" | 二期 |
| 终端 | 三期 |
| 文件浏览和预览 | 三期 |
| 设置：机器、通知、外观、语言、工作树清理、账号 | 随功能 |
| 离线缓存、未发出的消息重试（Unsent） | 二期 |
| 平板分栏 | 二期 |
| 推送、Live Activities、小组件 | Web Push 二期；原生三期（等 NSH） |
| 不做：Issues、定时任务、评审导览、MCP app、模拟器画面、访客链接、ChatGPT 转交 | |

## 分期和工作量（粗估，一人）

| 期 | 内容 | 周 |
| :-- | :-- | :-- |
| P0 | 试：Codex app-server；Agent SDK 在 Bun 上、读它的条款；NSH Web 包的 WebSocket、service worker、Web Push；Clerk 在大陆；三档 agent 查文档 | 2 |
| 一期 | 用户服务；`pansy-remote` 协议、线程、回合、Codex 和 Claude Code 两个驱动；配对、DPoP、范围、局域网和 tailnet；工作树、检查点、diff、提交推送 PR（GitHub）；Web 版一期页面；webhook 通知 | 20–24 |
| 二期 | Android 包；pansy Connect 的账号（五种登录）、设备批准；ACP 驱动和 Gemini、Qwen、Copilot；OpenCode、Pi；PR 页、Usage；离线缓存和重试；Web Push | 16–20 |
| 三期 | iOS 包；托管隧道；推送中继和原生推送（等 NSH）；其余 agent、本地 ACP 命令、Cursor；终端、文件；其余 forge | 16–20 |

一期约 5–6 个月，到"在手机浏览器上续接 Codex 或 Claude Code、批准、提交、
开 PR"。

## 待定

1. 身份服务：照 T3 用 Clerk，还是自建（取决于 P0 的大陆可达性）。
2. NSH 的远程推送：向 NSH 提需求，还是等。
3. 一期只出 Web 版（原生 app 在二期）。
