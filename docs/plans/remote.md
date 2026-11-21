# 计划：在手机上继续一段 agent 对话

目标：人离开电脑后，用手机（或另一台电脑的浏览器）打开 pansy，看到本机
agent 的会话，接着说下一句，回答它的审批，必要时打断它，开工作树、提交、
开 PR。局域网和 Tailscale 直连，外面经 Cloudflare Tunnel。

- **驱动 agent，不镜像终端。** pansy 用 agent 自己的无头协议启动它、续接
  会话、收发结构化事件；手机上是聊天界面，不是 xterm。
- **客户端用 NSH 写一次。** 同一份 TSX 出 iOS、Android 和 Web 三个包。
- **照 T3 Code。** 鉴权照它的环境会话模型（一次性配对、带范围的会话、
  DPoP、WebSocket 票据、可吊销的设备）；账号、推送中继和托管隧道照 T3
  Connect，做成 pansy Connect。

**以 [phone.md](phone.md)（第二版）为准。** 它在本页之上照 T3 Code 的手机端
重排了方案，加了 git、账号、推送和更多 agent；本页留着远程接口、驱动和
鉴权的细节，两处不一致时按 phone.md。

不在本计划：接管一个正在终端里跑的 TUI、Claude Desktop 的 Chat 和 Cowork、
多人协作和访客链接，见文末“以后”。

**状态：未开始。** 下文标“待验证”的地方都还没有试过，P0 先把它们试出来。

## 和路线的关系

这是 [roadmap.md](roadmap.md) 的迭代 7。路线为它改了三处：远程不再只有
“经 SSH 转发控制接口”一种，手机和浏览器接这里的远程接口；“不做独立的
Web UI”改成“不做桌面窗口的 Web 版”，这里的 Web 版是手机那份 TSX 的
`--target web`；原则 5（数字留在本机）加了两个例外：用户配对过的设备，
和用户登录了的 pansy Connect。

与迭代 2（一个守护进程）的关系：被驱动的 agent 是长期运行的进程，不该属于
哪个窗口。所以迭代 2 的用户服务（`pansy service install`）和“窗口接到已在跑
的网关”先做，远程接口跑在那个 `pansy serve` 里。

## 最终方案

```
手机 app / 浏览器 (NSH TSX)
      │  HTTPS + WebSocket，每个请求都带会话
      ▼
[cloudflared 命名隧道]  或  [局域网直连]
      ▼
pansy-remote（独立端口，默认只听回环）
  ├─ auth      配对、会话、DPoP、票据、范围
  ├─ sessions  读 pansy-agents::sessions（只读）
  └─ runs      每个被驱动的回合一个子进程
        ├─ Claude Code 驱动（Agent SDK，在 Bun 里）
        ├─ Codex 驱动（app-server，JSON-RPC）
        └─ 以后：OpenCode（serve）、Pi（rpc）、ACP
              │ 启动参数和环境由 pansy-agents::run 给出
              ▼
        agent 进程 → 网关 → provider
```

新增：

| 位置 | 内容 |
| :-- | :-- |
| `crates/pansy-remote` | 远程接口：axum 路由、鉴权、线程、回合管理、驱动、git |
| `cloud/` | pansy Connect：账号、设备批准、推送中继、隧道分配（Cloudflare Workers） |
| `crates/pansy-agents/src/sessions/desktop.rs` | 读 Claude Desktop 的会话记录（只读） |
| `crates/pansy-agents/agents/*.toml` 的 `[drive]` | 每个 agent 怎么被驱动（协议、参数），与 `[run]` 并列 |
| `mobile/` | 独立工作区：手机和 Web 客户端的 TSX，不依赖 `pansy-core` |
| `desktop/ui/Sharing.tsx` | Settings › 共享与访问：开关、配对二维码、设备列表、隧道 |
| `pansy-cli` | `pansy service install\|uninstall\|status`、`pansy remote on\|off\|status`、`pansy pair`、`pansy devices [revoke <id>]`、`pansy login`、`pansy tunnel` |

理由：

1. **结构化协议在手机上才好用。** 消息、工具调用、diff、审批请求都是有类型
   的事件，能做成按钮和卡片；终端镜像在手机上打字和点审批都难。
2. **协议是现成的。** Claude Desktop 自己就是这样驱动 Claude Code 的（见
   “Claude Desktop”）；Codex 的桌面和 IDE 客户端用 app-server。
3. **`pansy run` 已经解决了“不写配置就指向网关”。** 驱动只是把 `Launch` 的
   参数和环境用在一个无头进程上，所以手机上续接时可以换模型。
4. **独立的端口。** 网关把回环来的请求当作本人、不要 key
   （`server.rs` 的 `authorized`）；`cloudflared` 转进来的请求恰好都来自回环。
   远程接口因此不能和网关共用监听，也不能有“回环免鉴权”这条规则。

## 驱动

一个驱动把某个 agent 的无头协议变成同一套事件。接口大致是：

```rust
trait Driver {
    /// 续接 `session`（或新开一段），在 `launch` 给出的程序、参数和环境上。
    async fn start(&self, launch: Launch, session: Option<&str>, opts: Opts)
        -> Result<Run>;
}
// Run：send(消息)、answer(审批 id, 允许|拒绝)、interrupt()、events() 流
```

事件（发给客户端的）：`turn.started`、`message.delta`、`message`、
`tool.started`、`tool.finished`、`approval.requested`、`approval.resolved`、
`turn.finished`（带用量）、`run.exited`。每个事件有递增的序号。

| agent | 怎么驱动 | 续接 | 审批 | 打断 |
| :-- | :-- | :-- | :-- | :-- |
| Claude Code | `@anthropic-ai/claude-agent-sdk` 的 `query()`，在 pansy 的 Bun 宿主里，指向用户的 `claude`（照 T3） | `resume` | `canUseTool`（审批和提问） | `interrupt()` |
| Codex | `codex app-server`，stdio 上的 JSON-RPC | `thread/resume`（另有 `thread/fork`、`turn/steer`） | `item/{commandExecution,fileChange,permissions}/requestApproval` | `turn/interrupt` |
| OpenCode（以后） | `opencode serve`，HTTP + SSE | 它的 session id | 它的 permission 接口 | 它的 abort 接口 |
| Pi（以后） | `pi --mode rpc` | 它的会话 | 它的 rpc | 它的 rpc |
| 其余（以后） | ACP（Agent Client Protocol） | `session/load` | `session/request_permission` | `session/cancel` |

Codex 的方法名已在 0.171 的 `codex app-server generate-json-schema` 里核对；
Agent SDK 在 Bun 上能否跑、它的条款**待验证**（P0）。各版本记在清单的
`[drive]` 里（`verified = "…"`，与 `[run]` 的做法相同）。哪些 agent 走哪条，
见 phone.md 的“驱动”。

规则：

- **驱动不碰会话文件。** 文件由 agent 自己写；pansy 只启动它。历史仍由
  `sessions/` 只读地读出来。
- **一段会话同时只有一个写的人。** 会话文件在最近一分钟里被写过（沿用
  `trash.rs` 的判断），就不直接续接：告诉用户它可能正在别处运行，可以选
  “另开一支继续”（Claude Code 的 `--fork-session`，Codex 的 fork），原会话
  不动。
- **回合的事件只在内存里。** 每个回合留最近的事件（按条数和字节封顶），
  客户端断线重连时带上最后的序号补齐；更早的内容从 agent 的会话文件读。
  pansy 不另存一份对话，也就没有新的含正文的文件。
- **进程归 pansy 管。** Unix 上自成进程组，远程接口停止时结束它们；空闲
  （无客户端、无进行中的回合）超过一段时间的进程收掉，下次再续接。
- **权限模式照 T3。** 新线程最初是 Full access，之后沿用正在看的线程的模式；
  换到别的模式时，没有客户端在线的审批一直等着，不自动放行。

## Claude Desktop

本机上看到的（Linux，Claude Desktop 4.23118，第三方网关模式）：

- Desktop 的 Code 会话是它自带的 `claude`
  （`~/.config/Claude-3p/claude-code/<版本>/…/claude`），以
  `--input-format stream-json --output-format stream-json
  --permission-prompt-tool=stdio` 启动，stdio 接在 Desktop 的主进程上。
- 每个会话一条记录：`~/.config/Claude-3p/claude-code-sessions/**/local_*.json`，
  有 `cliSessionId`、`cwd`、`title`、`model`、`permissionMode`、`isArchived`。

所以：

- **能续接。** `sessions/desktop.rs` 只读这些记录，把 `cliSessionId` 对到
  Claude Code 的会话上，给它 Desktop 的标题，并标明来自 Desktop。续接走
  Claude Code 驱动，不需要另一个驱动。
- **不能接管。** 正在跑的那个进程的 stdio 属于 Desktop，外面插不进去。
  Desktop 里正开着的会话按“最近写过”处理：另开一支。
- **不做的：** Chat（不是本地 agent 进程）、Cowork（在它自己的虚拟机里）、
  操控 Electron 窗口（调试端口、辅助功能；每次升级都可能坏）。

待验证：从外部续接后 Desktop 窗口是否显示新的回合；用系统里的 `claude`
续接 Desktop 自带版本写的会话是否兼容（不兼容就用记录里钉的那个版本）；
macOS 和 Windows 上这些文件的位置（`claude-desktop.toml` 已注明各不相同）。

## 鉴权

读自 T3 Code 的 `docs/internals/environment-auth.md`、`remote.md`、
`t3-connect.md` 和 `docs/user/remote-access.md`。本节是“环境自己签发会话”
的一层；T3 Connect 那一层（账号、设备批准、中继、托管隧道）是 pansy
Connect，见 phone.md。

### 配对

1. 桌面窗口（或 `pansy pair`）生成一次性的配对密钥：随机 32 字节，10 分钟
   有效，只在创建时显示一次；磁盘上只存它的哈希、范围和到期时间。
2. 显示成二维码和链接：`https://<地址>/pair#<密钥>`，app 的
   `pansy://pair?...#<密钥>`。密钥在 `#` 之后，不进任何请求行和日志。
3. 客户端生成自己的密钥对，`POST /auth/pair`：配对密钥 + DPoP 证明 + 设备
   名。服务端核对后作废配对密钥，建立一个会话，回一个 1 小时的访问令牌。
4. **设备批准（可选，默认关，照 T3）。** 打开后，新设备的令牌要等有人在
   桌面上（或一台有 `access:write` 的设备上）核对由客户端公钥指纹得出的
   短码并批准才可用，这样链接被截走也换不到会话。打开时已连过的设备算已
   批准。

### 会话

| | 做法 |
| :-- | :-- |
| 令牌 | 1 小时；绑定客户端的公钥指纹（DPoP）。证明无效就拒绝，不退回成普通 bearer |
| 续期 | 带着令牌（过期的也行）和同一把密钥的证明来换新的；会话到期时间顺延 30 天。30 天没用过就重新配对 |
| 浏览器 | 同一个会话模型的 cookie 形式（`HttpOnly`、`SameSite=Strict`、HTTPS 上 `Secure`）。局域网的纯 HTTP 页面没有 WebCrypto，只能用 cookie；这种会话不能学到别的地址（见“路由”） |
| WebSocket | 先经已鉴权的 HTTP 换一张 30 秒的一次性票据，票据进 URL；DPoP 客户端的票据绑定公钥，升级时再带一份新的证明 |
| 重放 | DPoP 证明的 `jti` 在有效期内只收一次；证明里写着方法和 URL |
| 吊销 | 一台设备一行，吊销即断开它的连接、作废它的令牌和没用掉的配对 |

### 范围

照 T3 的名字：

| 范围 | 能做什么 |
| :-- | :-- |
| `orchestration:read` | 列线程和会话、读内容、看正在进行的回合 |
| `orchestration:operate` | 发消息、续接、新开、打断、回答审批、换模型和权限模式 |
| `source-control:write` | 建删工作树、提交、推送、开和合并 PR |
| `filesystem:read` | 浏览和预览文件 |
| `terminal:read`、`terminal:operate` | 看和操作终端 |
| `settings:write`、`providers:manage` | 改设置、管 provider |
| `access:read`、`access:write` | 看设备；生成配对、批准和吊销设备、开关隧道 |

- **配对默认给全部范围，照 T3。** T3 的文档直说：能操作终端或启动全权限
  agent 的客户端本来就能给自己签出任何会话，分得再细也拦不住。生成配对时
  可以收窄（比如只读），换令牌时只能收窄；要生成新的配对，自己得有
  `access:write` 和要给出去的每一个范围。
- 每个路由和每种 WebSocket 消息声明它要的范围，中间件在处理之前检查；
  握手成功不代表有权。
- 设备名、系统、最近连接时间只用于显示，不参与鉴权。

### 存在哪里

- `remote.json`（`PANSY_HOME`，0600）：设备的会话（令牌哈希、公钥指纹、
  范围、批准、名字、时间）、未用的配对（哈希）、设置。没有可还原的密钥。
- 服务端的签名密钥：跟 `keyStore` 走（系统的 key store，或 0600 的文件）。
- 客户端：私钥和令牌在 iOS 的 Keychain、Android 的 Keystore。NSH 0.6.1 没有
  钥匙串的接口，由 app 自己的 Rust crate 包（`nsh init --template mixed`）；
  能否生成不可导出的签名密钥**待验证**，不行就存一把软件密钥。

## 路由：局域网和隧道

- **默认关。** 不开“共享与访问”，远程接口不监听。
- **局域网。** 开了之后听所有 IPv4 接口，但只接受来源和目的都是回环、私网
  或 tailnet（`100.64.0.0/10`）的连接，这样有公网地址的机器不会把它露出去。
  这不是防火墙；鉴权照旧每个请求都做。
- **Cloudflare Tunnel。** pansy 托管 `cloudflared`：
  - 托管隧道（照 T3 Connect）：登录了 pansy Connect 的机器，由中继在
    pansy 的 Cloudflare 账号下分配一条，用户什么都不用设。推荐。
  - 命名隧道，用户自己的 Cloudflare 账号和域名（`cloudflared tunnel login`
    由用户做一次），地址固定，可以在前面再加 Cloudflare Access。不想登录
    pansy Connect 的用这个。
  - 快速隧道（`trycloudflare.com`）：不用账号，地址每次都变，只用来试。
  - 隧道只指向远程接口的回环端口，**永远不指向网关的端口**；启动前核对。
  - `PATH` 上或设置里指定的 `cloudflared` 是用户的，不替换；没有才问过之后
    下载钉住版本的那个，校验和核对。
  - 用 Tailscale 的人不需要隧道：tailnet 地址就是一条局域网路由。
- **一台机器几条路。** 客户端存一个有序的地址表（局域网、tailnet、隧道），
  连第一条通的；连之前先读公开的描述（`GET /descriptor`：机器的 id、版本、
  能力），id 不对就不把凭据发过去。连上后服务端报告它现在的局域网地址，
  客户端记下，于是在外面配对一次，回家自动走局域网。经 HTTPS 配对的令牌
  不会被拿去走纯 HTTP 的地址。
- 机器的 id 在 `remote.json` 里，重启和换地址都不变。

## 客户端

`mobile/`，自成一个工作区，像 `desktop/` 那样不进核心的构建：

- **只是远程接口和 pansy Connect 的客户端。** HTTP + WebSocket，不 import
  `pansy/native`。页面照 T3 的手机端，逐页的对照和分期见 phone.md。
- **三个目标。** `nsh package --target ios|android|web`。Web 包由远程接口
  自己在 `/` 上送出，所以局域网的纯 HTTP 地址也能用；不另设托管站点。
- **字符串**全在 `mobile/ui/i18n.ts`，英文和简体中文，`nsh check` 把关，
  与桌面相同。会话内容的渲染若与 `desktop/ui/Sessions.tsx` 重合，抽到两边
  都能 import 的一处。
- **发版。** Web 包随 pansy 一起；`.apk` 和 `.ipa` 各自签名（iOS 要在 macOS
  上打包）。密钥照 `docs/release.md` 留在 git 之外。

### 通知

照 T3：等审批、等回答、完成、失败时推送，点开进对应线程，app 在前台时不弹。

- **原生推送**经 pansy Connect 的中继发 APNs 和 FCM，负载用设备公钥加密。
  NSH 0.6.1 的 app 拿不到推送令牌（iOS 的 `AppDelegate` 不收、`nsh package`
  加不了 `aps-environment`；Android 打不进 Firebase），要等 NSH。
- **Web Push**：主屏幕上的 Web 版由家里的 pansy 直接发，不经中继。
- **webhook**：用户填一个自己的地址（ntfy、Bark、Telegram bot 之类），
  **内容只有线程标题和事件种类，没有正文**，默认关。

## 不变量

加进 `AGENTS.md` 的：

- **远程接口的每个请求都要会话。** 没有“回环免鉴权”；它有自己的端口，隧道
  只指向它。
- **配对密钥和令牌不落盘。** `remote.json` 只有哈希和公钥指纹，0600。
- **被驱动的会话仍是用户的文件。** pansy 启动 agent，不写它的会话；最近写过
  的会话不直接续接。
- **回合的内容只在内存里。** 通知里没有正文；经中继的推送是密文。
- **pansy Connect 不碰内容。** 它存账号、机器、设备和推送令牌；不存 key、
  对话、代码，应用流量不经过它。
- **测试不跑真的 agent，也不出网。** 驱动的测试用一个照着协议说话的假程序
  （`PANSY_HOME`、`HOME` 指向临时目录）；隧道的测试用一个假的
  `cloudflared`；key 用 `PANSY_KEYSTORE=dir:…`。

## 分期

分期以 phone.md 为准（P0 验证，之后一期、二期、三期）。本页各部分的
“做完的样子”：

| 部分 | 做完的样子 |
| :-- | :-- |
| 驱动 | 命令行 `pansy drive <agent> <session>` 上续接一段会话、回答审批、打断；测试用假 agent |
| 鉴权 | 没配对的请求一律拒绝；吊销立即生效；测试覆盖重放、收窄、过期 |
| 客户端 | 手机扫码后续接一段会话、批准一次工具调用、提交并开一个 PR |
| 隧道 | 在蜂窝网络上完成同样的事；隧道指向网关端口时拒绝启动 |

## 风险

- **这是一个远程执行代码的入口。** 照 T3，配对默认给全部范围，新线程默认
  Full access，账号下的新设备默认不用批准，所以**配对过的设备和登录的账号
  就是钥匙**。应对：默认关、独立端口、每个请求鉴权、令牌绑定设备、可吊销、
  可打开设备批准、提示开两步验证。文档和页面上直说“开了它，配对过的设备
  能在这台电脑上做任何事”。
- **agent 的无头协议会变。** Agent SDK、Codex 的 app-server 和 ACP 都没有
  稳定性承诺。应对：按版本记在清单里，驱动遇到不认识的事件原样记下
  不崩；`pansy doctor` 报“这个版本没试过”。
- **两处同时写一段会话。** 应对见“驱动”的规则；仍可能在一分钟的窗口之外
  撞上，fork 是默认的安全选项。
- **NSH 移动端的缺口。** 推送令牌、钥匙串、相机、系统登录窗口没有；后台
  WebSocket 待验证。应对：钥匙串和登录窗口用 app 自己的 Rust crate；推送等
  NSH，之前 Web Push 和 webhook 兜底；Web 版始终可用。
- **`cloudflared` 的供应。** 下载的二进制要钉版本、核对校验和；快速隧道的
  地址谁拿到都能连到登录口，只用来试。
- **pansy Connect 是一个要运营的云服务。** 账号数据、隐私政策、删号、推送
  证书、隧道的费用和滥用。应对：存得最少，负载加密，按机器限流。

## 以后

- 接管由 pansy 启动的交互式会话（`pansy run` 起在 pansy 的 PTY 里）。
- 访客链接（只读一段会话）、多台机器之间转交任务。
