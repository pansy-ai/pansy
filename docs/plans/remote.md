# 计划：在手机上继续一段 agent 对话

目标：人离开电脑后，用手机（或另一台电脑的浏览器）打开 pansy，看到本机
agent 的会话，接着说下一句，回答它的审批，必要时打断它。局域网内直连，
外面经 Cloudflare Tunnel。

- **驱动 agent，不镜像终端。** pansy 用 agent 自己的无头协议启动它、续接
  会话、收发结构化事件；手机上是聊天界面，不是 xterm。
- **客户端用 NSH 写一次。** 同一份 TSX 出 iOS、Android 和 Web 三个包。
- **鉴权照 T3 Code 的环境会话模型**（一次性配对、带范围的会话、DPoP、
  WebSocket 票据、可吊销的设备），不要它的云端账号和中继。

不在本计划：终端镜像、接管一个正在终端里跑的 TUI、Claude Desktop 的 Chat 和
Cowork、pansy 自营的中继或账号、多人协作和访客链接，见文末“以后”。

**状态：未开始。** 下文标“待验证”的地方都还没有试过，P0 先把它们试出来。

## 和路线的关系

这是 [roadmap.md](roadmap.md) 的迭代 7。路线为它改了两处：远程不再只有
“经 SSH 转发控制接口”一种，手机和浏览器接这里的远程接口；“不做独立的
Web UI”改成“不做桌面窗口的 Web 版”，这里的 Web 版是手机那份 TSX 的
`--target web`。原则 5（数字留在本机）加了一个例外：用户自己配对过的设备。

与迭代 2（一个守护进程）的关系：远程接口应当长在守护进程里，因为被驱动的
agent 是长期运行的进程，不该属于哪个窗口。守护进程没做完之前，它先跑在
`pansy serve` 和窗口各自的进程里，代价是关掉窗口时由它启动的回合会结束；
窗口要在关闭前说明这一点。

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
        ├─ Claude Code 驱动（stream-json）
        ├─ Codex 驱动（app-server，JSON-RPC）
        └─ 以后：OpenCode（serve）、ACP
              │ 启动参数和环境由 pansy-agents::run 给出
              ▼
        agent 进程 → 网关 → provider
```

新增：

| 位置 | 内容 |
| :-- | :-- |
| `crates/pansy-remote` | 远程接口：axum 路由、鉴权、回合管理、驱动 |
| `crates/pansy-agents/src/sessions/desktop.rs` | 读 Claude Desktop 的会话记录（只读） |
| `crates/pansy-agents/agents/*.toml` 的 `[drive]` | 每个 agent 怎么被驱动（协议、参数），与 `[run]` 并列 |
| `mobile/` | 独立工作区：手机和 Web 客户端的 TSX，不依赖 `pansy-core` |
| `desktop/ui/Sharing.tsx` | Settings › 共享与访问：开关、配对二维码、设备列表、隧道 |
| `pansy-cli` | `pansy remote on\|off\|status`、`pansy pair`、`pansy devices [revoke <id>]`、`pansy tunnel` |

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
| Claude Code | `claude -p --input-format stream-json --output-format stream-json --verbose` | `--resume <id>`；并发时 `--fork-session` | `--permission-prompt-tool=stdio`，审批作为控制请求到 stdin/stdout | 控制请求 `interrupt` |
| Codex | `codex app-server`，stdio 上的 JSON-RPC | `thread/resume` | 服务端发来的审批请求，回 allow/deny | `turn/interrupt` |
| OpenCode（以后） | `opencode serve`，HTTP + SSE | 它的 session id | 它的 permission 接口 | 它的 abort 接口 |
| 其余（以后） | ACP（Agent Client Protocol） | `session/load` | `session/request_permission` | `session/cancel` |

上表里 Claude Code 的参数是从本机正在运行的 Claude Desktop 的进程上看到的；
控制请求的具体字段、Codex app-server 的方法名和字段**待验证**（P0），并且
按版本记在清单的 `[drive]` 里（`verified = "…"`，与 `[run]` 的做法相同）。

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
- **审批默认逐次问。** 远程启动的回合不会比配对时给这台设备的上限更宽
  （见“鉴权”的模式上限）；没有客户端在线时审批一直等着，不自动放行。

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
`t3-connect.md` 和 `docs/user/remote-access.md`。取它“环境自己签发会话”的
一层；T3 Connect（Clerk 账号、中继、托管隧道）不取。

### 配对

1. 桌面窗口（或 `pansy pair`）生成一次性的配对密钥：随机 32 字节，10 分钟
   有效，只在创建时显示一次；磁盘上只存它的哈希、范围和到期时间。
2. 显示成二维码和链接：`https://<地址>/pair#<密钥>`，app 的
   `pansy://pair?...#<密钥>`。密钥在 `#` 之后，不进任何请求行和日志。
3. 客户端生成自己的密钥对，`POST /auth/pair`：配对密钥 + DPoP 证明 + 设备
   名。服务端核对后作废配对密钥，建立一个会话，回一个 1 小时的访问令牌。
4. **设备核对码。** 经隧道来的配对默认还要在桌面上（或一台已连接的设备上）
   核对一个由客户端公钥指纹得出的短码并批准，之后令牌才可用。这样链接被
   截走也换不到会话。局域网内可在设置里关掉这一步。

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

| 范围 | 能做什么 |
| :-- | :-- |
| `sessions:read` | 列会话、读内容、看正在进行的回合 |
| `sessions:operate` | 发消息、续接、新开、打断 |
| `approvals:answer` | 回答审批 |
| `access:write` | 生成配对、批准和吊销设备、开关隧道 |

- 配对时定下范围，换令牌时只能收窄；要生成新的配对，自己得有
  `access:write` 和要给出去的每一个范围。
- 每个路由和每种 WebSocket 消息声明它要的范围，中间件在处理之前检查；
  握手成功不代表有权。
- **模式上限。** T3 Code 的文档直说：能操作终端或启动全权限 agent 的客户端，
  本来就能给自己签出任何会话，所以它默认配对给全部范围。pansy 没有远程
  终端，于是在会话里再签一个上限：这台设备能启动的最宽权限模式（默认
  “每次都问”；“自动接受编辑”和“跳过审批”要在桌面上明确给）。有了上限，
  `sessions:operate` 才不等于 `access:write`。**哪天加了终端镜像，这条界线
  就没有了**，到时默认范围要跟着改。
- 设备名、系统、最近连接时间只用于显示，不参与鉴权。

### 存在哪里

- `remote.json`（`PANSY_HOME`，0600）：设备的会话（令牌哈希、公钥指纹、
  范围、上限、名字、时间）、未用的配对（哈希）、设置。没有可还原的密钥。
- 服务端的签名密钥：跟 `keyStore` 走（系统的 key store，或 0600 的文件）。
- 客户端：私钥和令牌在 iOS 的 Keychain、Android 的 Keystore（NSH 0.6.0 里
  有 keychain 的接口；能否在其中生成不可导出的签名密钥**待验证**，不行就
  存一把软件密钥）。

## 路由：局域网和隧道

- **默认关。** 不开“共享与访问”，远程接口不监听。
- **局域网。** 开了之后听所有 IPv4 接口，但只接受来源和目的都是回环、私网
  或 tailnet（`100.64.0.0/10`）的连接，这样有公网地址的机器不会把它露出去。
  这不是防火墙；鉴权照旧每个请求都做。
- **Cloudflare Tunnel。** pansy 托管 `cloudflared`：
  - 命名隧道，用户自己的 Cloudflare 账号和域名（`cloudflared tunnel login`
    由用户做一次），地址固定，可以在前面再加 Cloudflare Access。推荐。
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

- **只是远程接口的客户端。** HTTP + WebSocket，不 import `pansy/native`。
  页面：机器列表和配对（扫码）、会话列表（按文件夹，搜索）、一段会话（消息、
  工具调用折叠、diff、审批卡片、输入框、打断、换模型）、设备和路由。
- **三个目标。** `nsh package --target ios|android|web`。Web 包由远程接口
  自己在 `/` 上送出，所以局域网的纯 HTTP 地址也能用；不另设托管站点。
- **字符串**全在 `mobile/ui/i18n.ts`，英文和简体中文，`nsh check` 把关，
  与桌面相同。会话内容的渲染若与 `desktop/ui/Sessions.tsx` 重合，抽到两边
  都能 import 的一处。
- **发版。** Web 包随 pansy 一起；`.apk` 和 `.ipa` 各自签名（iOS 要在 macOS
  上打包）。密钥照 `docs/release.md` 留在 git 之外。

### 通知

NSH 0.6.0 里没找到推送通知。app 在前台时靠 WebSocket；在后台时：

- 可选的 webhook：用户填一个自己的地址（ntfy、Bark、Telegram bot 之类），
  “回合结束”和“等待审批”时 pansy 向它发一条。**内容只有会话标题和事件
  种类，没有正文**，默认关。
- NSH 以后有了推送再换。

## 不变量

加进 `AGENTS.md` 的：

- **远程接口的每个请求都要会话。** 没有“回环免鉴权”；它有自己的端口，隧道
  只指向它。
- **配对密钥和令牌不落盘。** `remote.json` 只有哈希和公钥指纹，0600。
- **被驱动的会话仍是用户的文件。** pansy 启动 agent，不写它的会话；最近写过
  的会话不直接续接。
- **回合的内容只在内存里。** 通知里没有正文。
- **测试不跑真的 agent，也不出网。** 驱动的测试用一个照着协议说话的假程序
  （`PANSY_HOME`、`HOME` 指向临时目录）；隧道的测试用一个假的
  `cloudflared`；key 用 `PANSY_KEYSTORE=dir:…`。

## 分期

| 期 | 内容 | 做完的样子 |
| :-- | :-- | :-- |
| P0 验证 | `spikes/remote/`：用脚本驱动 Claude Code 和 Codex（续接、发一句、审批、打断），记下协议字段和版本；`nsh run --target android` 和 `web` 上跑通 WebSocket、扫码、存密钥；试 Desktop 的两个待验证项 | 一页结论，待验证的地方都有答案；不通的改方案 |
| P1 驱动 | `pansy-remote` 的回合管理和两个驱动，`[drive]` 清单，`sessions/desktop.rs`；先只在回环上，用 `pansy drive <agent> <session>` 在终端里试 | 命令行上续接一段会话、回答审批、打断；测试用假 agent |
| P2 鉴权 | 配对、会话、DPoP、票据、范围和模式上限、`remote.json`、`pansy pair`、`pansy devices` | 没配对的请求一律拒绝；吊销立即生效；测试覆盖重放、收窄、过期 |
| P3 客户端 | `mobile/`：Web 和 Android，局域网；桌面的“共享与访问”页 | 手机扫码后续接一段 Claude Code 会话并批准一次工具调用 |
| P4 隧道 | 托管 `cloudflared`、设备核对码、多条路由、`pansy tunnel` | 在蜂窝网络上完成同样的事；隧道指向网关端口时拒绝启动 |
| P5 之后 | iOS 包、webhook 通知、OpenCode 和 ACP 驱动、Sessions 页的“在手机上继续” | |

P1 和 P2 互不依赖，可以并行。守护进程（roadmap 迭代 2）做完后，把远程接口
搬进去，P1 里“关窗口即结束回合”的限制随之消失。

## 风险

- **这是一个远程执行代码的入口。** 应对：默认关、独立端口、每个请求鉴权、
  令牌绑定设备、经隧道配对要核对码、模式上限、审批默认逐次问。文档和页面
  上直说“开了它，配对过的设备能在这台电脑上让 agent 干活”。
- **agent 的无头协议会变。** Claude Code 的控制请求和 Codex 的 app-server
  都没有稳定性承诺。应对：按版本记在清单里，驱动遇到不认识的事件原样记下
  不崩；`pansy doctor` 报“这个版本没试过”。
- **两处同时写一段会话。** 应对见“驱动”的规则；仍可能在一分钟的窗口之外
  撞上，fork 是默认的安全选项。
- **NSH 移动端的缺口。** 推送没有；不可导出的密钥、后台 WebSocket 待验证。
  应对：P0 先试，webhook 兜底，Web 版始终可用。
- **`cloudflared` 的供应。** 下载的二进制要钉版本、核对校验和；快速隧道的
  地址谁拿到都能连到登录口，所以配对必须有核对码。
- **窗口进程里跑的回合。** 守护进程之前，关窗口会结束它们。应对：关闭前
  提示；`pansy serve` 下不受影响。

## 以后

- 终端镜像（PTY），给没有无头协议的 agent 兜底；默认范围要重新定。
- 接管由 pansy 启动的交互式会话（`pansy run` 起在 pansy 的 PTY 里）。
- 访客链接（只读一段会话）、多台机器之间转交任务。
- pansy 自营的中继：不做，除非先改 roadmap 的原则（不保管用户的云端凭据）。
