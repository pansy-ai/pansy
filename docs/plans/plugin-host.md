# 计划：在 Bun 上复用 magpie 的社区插件

目标：让 pansy 像 magpie 一样运行社区插件，插件作者不需要为 pansy 改任何东西：

- **provider 插件**：npm 上的 OpenCode 插件（`auth` 钩子）和 pi package
  （`pi.registerProvider`）。插件负责登录、续期、列模型和发请求，pansy 负责
  路由、遮蔽、用量和把它们交给 agent。magpie-community 的 50 个，加上 npm 上
  的同类包。
- **middleware**：magpie 的 JS 网关中间件（`magpie.middleware`）和异步请求
  钩子（`magpie.hooks`），改写经过网关的请求和回复。magpie-community 24 个。
- **agent 插件**（`magpie.agent`）：让 pansy 能接入它没有内置的 agent。
  magpie-community 25 个。

不在本计划：tools 和 search 钩子（要先有 MCP）、页面和面板插件、OpenCode V2
插件，见文末“以后”。

**状态：P0–P8 已实现**（`crates/pansy-plugins`，用法见
[docs/plugins.md](../plugins.md)）。与下文不同或还没做的：

- agent 的 User-Agent 按 magpie 的映射传给插件 provider：未做，插件看到的是
  agent 原样的请求头。
- Windows 上用 Job 结束宿主的子进程树：未做（Unix 用进程组）。
- macOS、Windows 上的 P0 验证（`spikes/plugin-host/run.sh`）还没跑。
- 插件 provider 与内置 provider 同名时改名为 `<id>-plugin`；
  `pansy plugin use` 指定同一个 provider 由哪个插件提供。

## 最终方案

**所有插件都跑在 Bun 上，分两个宿主进程；不引入第二个 JS 引擎。**

| 插件 | 宿主 | 代码 |
| --- | --- | --- |
| provider（OpenCode） | A | 复用 magpie 的 `host.js` |
| pi package | A | 复用 magpie 的 `pi.js` |
| 异步请求钩子（`magpie.hooks`） | A | `host.js` 已有（`gatewayRequest`） |
| middleware（`magpie.middleware`） | B | 新写 `mw-host.js`，钩子和规则照 magpie 的 Go 实现 |
| agent（`magpie.agent`） | B | 同上 |

理由：

1. **只有一套基础设施。** Bun 下载、安装、宿主进程管理、`plugins.json` 只做
   一次，所有插件共用。
2. **性能够用。** 实测（Linux，Bun 对 Bun 的 stdio）：一个 SSE 事件往返
   0.033–0.041 ms，连续发出不等回复时 0.005 ms；80 KB 请求体往返
   0.36–0.39 ms。magpie 在进程内（moejs）的数字是 0.001–0.0016 ms 和
   0.2–0.7 ms。2000 个事件的回复总共多约 70 ms，分摊在整段流里。
3. **有些插件本来就离不开 Bun。** 记忆类的 `magpie.hooks` 插件要联网，magpie
   也是用 Bun 跑它们的。
4. **兼容风险最小。** 不用担心 QuickJS 缺 JS 特性或 Node API，也不用让网关跟着
   NSH 钉死的 `quickjs-jit =0.14.2` 一起升级。

分两个宿主，是因为 middleware 在每个请求的必经路径上：Bun 的 JS 只有一个
线程，一个跑飞的钩子会卡住整个宿主。分开后它只连累 middleware，provider
请求照常走宿主 A。

接受的代价：**middleware 没有沙箱**。它在遮蔽之前运行，能看到遮蔽前的内容，
权限和用户一样。这和 magpie 用 Bun 跑 `magpie.hooks`、以及 provider 插件的
信任模式相同。应对见“风险”。网关的钩子接口不绑死在 Bun 上，以后需要沙箱
时可以加一个 QuickJS 后端，社区插件不用改。

不选的：QuickJS / quickjs-jit（唯一的明显好处是给 middleware 沙箱，先不为此
多维护一个引擎；它的 JIT 对 JSON、字符串、正则没有加速）；NSH 自己的插件
机制（只能装 TSX 界面模块）；Node（宿主把 `Bun.$` 交给插件）；DeepSeek
Harness 插件（扩展的是 dsh 自己的 agent）。

## 已经确认的事实

- 插件是 JS/TS，大量用 Node API（`http` 起 OAuth 回调、`child_process`、
  `crypto`、`fs`）。抽查的 4 个包都没用 Bun 专有 API，也没有原生扩展。
  OpenCode 插件发编译好的 JS；pi package 直接发 `.ts`，并以
  `@earendil-works/pi-ai` / `pi-coding-agent` 为 peer。
- magpie 的做法：Go 进程 + Bun 子进程跑 `host.js`（3809 行）和 `pi.js`
  （1110 行），stdio 上一行一个 JSON 消息。协议见 `host.js` 开头注释：
  `{id, method, params}` → `{id, result|error}`；`fetch` 先回
  `{id, event:"head", status, headers}`，再回若干 `{id, event:"chunk", data}`
  （base64），最后 `{id, result:null}`；`abort` 取消，`credit` 归还流控窗口
  （默认 512 KiB）；宿主主动发 `log`、`toast`、`auth`、`signIn`、`renewing`、
  `load`、`usage`、`heavy` 事件。
- RPC 方法：`init`、`providers`、`prompt`、`validate`、`authorize`、
  `callback`、`apiKey`、`fetch`、`load`、`keepSignedIn`、`signOut`、`reload`、
  `check`、`import`、`take`、`setProviderOptions`、`gatewayRequest`，以及本计划
  暂不用的 `usage`、`checkin`、`calls`、`tool`、`search`。
- 社区插件会调 `input.magpie.option / load / usage / account`，读
  package.json 的 `magpie.icon / maxConcurrency / maxAccounts`，子进程读
  `MAGPIE_*_PROXY`。要兼容就得**保留这些名字**。
- 插件在 package.json 里声明自己是什么：`magpie.middleware`（别名
  `magpie.gateway`）、`magpie.hooks`、`magpie.agent`；单个文件则按文件名：
  `*.middleware.js`、`*.gateway.js`、`*.hooks.{js,mjs,ts,mts}`、`*.agent.js`。
  例：`@magpie-community/middleware-word-guard` 的
  `{"middleware": "./word-guard.middleware.js", "options": {…}}`，
  `@magpie-community/agent-aider` 的 `{"agent": "./aider.agent.js"}`，
  `@vshulcz/magpie-deja` 的 `{"hooks": "./deja.hooks.mjs"}`。
- magpie 钉的 Bun 是 1.4.2；装插件用 `bun add --ignore-scripts`。
- pansy 这边：`pansy-core/src/subscription/` 已有“一个订阅是一个 provider，
  账号像 key 一样按序尝试”的模型；网关在 `server.rs` 的 `signed` /
  `exchange` / `send` 发上游请求，拿到 `reqwest::Response` 后统一处理状态、
  失败转移、SSE 和遮蔽还原。`reqwest::Response::from(http::Response<_>)`
  已在 `send` 里用过，插件的回复可以包成同一个类型，后面的代码不用动。

## 关键决定

1. **只用 Bun，自动下载。** 钉一个版本，第一次需要时下到
   `PANSY_HOME/bun/<版本>/`，用官方 `SHASUMS256.txt` 校验；`PANSY_BUN` 可指定
   本机的 Bun。P0 确认宿主把 `Bun.$` 交给插件（OpenCode 的接口），`pi.js` 用
   `Bun.resolveSync`，换 Node 等于重写宿主。
2. **两个宿主进程**：A 跑 provider、pi 和 `magpie.hooks`；B 跑 middleware 和
   agent 插件。各自懒启动，各自崩溃重启。
3. **宿主 A 复用 magpie 的 `host.js` 和 `pi.js`，不重写。** 放进新 crate 的
   `host/` 目录，`include_str!` 编进二进制，启动时写到
   `PANSY_HOME/plugins/host-<hash>.js`。记下所取的 magpie 提交，写一份同步说明。
   MIT 允许复用，`NOTICE.md` 里“the code is new”要改。
4. **宿主 B 是 pansy 自己写的 `mw-host.js`**。magpie 的 middleware 运行逻辑
   是 Go 写的（`internal/middleware`，约 1,700 行），没有 JS 版本可复用；钩子、
   `ctx`、返回值和失败规则照它的文档和测试实现。
5. **兼容名字照旧。** `input.magpie`（另加 `input.pansy` 指向同一对象）、
   package.json 的 `magpie.*`、`MAGPIE_*_PROXY`、`x-magpie-*` 头都保持不变，
   否则社区插件会坏。
6. **文件格式与 magpie 一致**，便于从 magpie 导入：`plugins.json`
   （`{plugins:[{spec, off, options}], config, prefer}`）、`plugin-auth.json`
   （OpenCode 的 auth.json 形状，第二个账号起为 `id#slot`）。
7. **插件 provider 不塞进 `subscription::Kind`。** `Kind` 是封闭枚举，插件的
   provider id 是开放字符串。`Provider` 加 `plugin: Option<PluginRef>`，
   `Account` 加 `plugin_account: Option<PluginAccount>`，和 `subscription`
   并列。
8. **新 crate `pansy-plugins`**（依赖 `pansy-core`，被 `pansy-gateway`、
   `pansy-agents`、`pansy-cli`、`desktop/native` 依赖）：Bun、安装、两个宿主
   和 RPC 都在这里。`pansy-core` 只管数据和文件，符合现有分层。
9. **网关的 middleware 接口与引擎无关**：`pansy-gateway` 只依赖一个
   `Middleware` trait（`begin` / `request` / `account` / `provider` / `event` /
   `response`），宿主 B 是它的第一个实现。
10. **只支持 OpenCode V1 插件。** OpenCode V2 插件（依赖 `@opencode/plugin`，
    默认导出 `Plugin.define(...)`）magpie 的宿主不支持；安装时就识别并明确
    提示“暂不支持”，不要等载入失败。（P0：`opencode-gitlab-auth` 3.0.0 就是。）
11. **从 magpie 导入**：做，放在最后一个阶段。
12. **Plugins 放在 Settings 里**，和 magpie 的位置一样；侧栏留给日常页面。

## 文件（全部经 `pansy_core::paths`）

| 路径 | 内容 | 权限 |
| --- | --- | --- |
| `plugins.json` | 装了哪些、开关、选项、同 id 选谁 | 0600 |
| `plugin-auth.json` | 插件账号的令牌（宿主 A 读写，带锁） | 0600 |
| `plugin-providers.json` | 上次列出的 provider 和模型，启动时不等 Bun | 0600 |
| `agent-plugins.json` | 上次载入的 agent 描述，列 agent 时不用启动 Bun | 0600 |
| `account-ref.key` | 给 `onAccount` 算账号 ref 的密钥，每次安装生成一次 | 0600 |
| `plugins/` | `package.json`、`bun.lock`、`node_modules`、宿主脚本 | 0700 |
| `pi/` | pi 的数据目录（`PI_CODING_AGENT_DIR`） | 0700 |
| `bun/<版本>/` | 下载的 Bun | 0700 |
| `models-dev.json` | models.dev 缓存（宿主 A 补全模型信息用） | 普通 |

## 分阶段

每个阶段单独可合并，结束时 `cargo fmt/clippy/test` 和 `desktop` 的检查都过。
P1–P4 打通 provider 插件；P5–P6 加上 middleware 和 agent；P7–P8 补齐。

### P0 验证（已完成：Linux 通过，见 [plugin-host-p0.md](plugin-host-p0.md)；macOS、Windows 待跑 `spikes/plugin-host/run.sh`）

- 在 scratch 目录里用 Bun 跑 vendored `host.js`，载入 magpie 的
  `internal/plugin/testdata/fake`，从一个 Rust 小程序走通
  `init → providers → fetch(head/chunk/result) → abort`。
- 产出：确认协议、确认 Bun 1.4.2 在 Linux/macOS/Windows 都能跑，确认
  `host.js` 里需要改的 magpie 专属路径（`MAGPIE_*` 以外的硬编码）。
- 不通过就停下来重新评估。

### P1 `pansy-plugins`：运行时、安装、宿主

- `bun.rs`：找 Bun（`PANSY_BUN` → 已下载 → 下载并校验），平台/指令集
  不支持时给出明确错误。
- `store.rs`：`plugins.json` 读写（原子写、0600、损坏时保留不覆盖）；
  spec 解析（npm 名、`name@ver`、`github:`/git URL、本地路径）；
  `add / remove / set_off / set_options / update`，用
  `bun add|remove --ignore-scripts`，失败回滚 `package.json` 和锁文件。
- `kind.rs`：判断一个包是什么，可以同时是几种：OpenCode provider、pi
  package（`pi` 字段、`pi-package` 关键词、依赖 pi-coding-agent）、
  `magpie.middleware`、`magpie.hooks`、`magpie.agent`；识别 OpenCode V2 并拒绝。
  pi 的话把 pi 装在旁边。
- `host.rs`：通用的宿主进程（A、B 共用）：懒启动；JSON 行 RPC 客户端
  （id 表、超时、取消即发 `abort`）；事件广播通道；崩溃后退避重启；
  `plugins.json` 变了就重启；进程退出时连子进程树一起结束（Unix 进程组，
  Windows Job）。环境变量：代理（`MAGPIE_*_PROXY`）、`PI_CODING_AGENT_DIR`、
  `HOME` 不改。
- 测试：用本地文件夹插件（从 magpie 的 testdata 拿 `fake`、`renew`、
  `signinend`、`stuck`、`pi/fakepi.ts`），不联网；需要 Bun 的测试在没有
  Bun 时跳过并打印原因，CI 预装 Bun。

### P2 provider 与账号（`pansy-core` + `pansy-plugins`）

- `providers` RPC 的结果合并进 provider 列表，做法同
  `subscription::merge`：插件开着才出现；账号来自 `plugin-auth.json`；
  模型带上下文、输出上限、推理、图片输入。
- 协议映射（按模型或 provider 的 `npm`）：
  `@ai-sdk/openai-compatible` → Chat，`@ai-sdk/openai` → Responses，
  `@ai-sdk/anthropic`（以及所有 pi provider）→ Anthropic，
  `@ai-sdk/google` → Gemini。其他（Bedrock、Vertex 等）先不支持，
  模型隐藏并说明原因。
- 和已有 provider id 撞名（`google`、`openai`、`anthropic`）时改为
  `<id>-plugin`；两个插件给同一 id 时按 `prefer`，否则后装的生效。
- 列表缓存到 `plugin-providers.json`，网关启动不等 Bun；后台刷新。

### P3 网关走插件发请求（`pansy-gateway`）

- `exchange` 里，attempt 属于插件 provider 时不用 reqwest，改发宿主 A 的
  `fetch`：`{provider, account, model(apiId), npm, url, method, headers,
  body(base64), window, proxy, session}`。
- 宿主的 head/chunk 包成 `http::Response<reqwest::Body::wrap_stream(..)>`
  再转 `reqwest::Response`，`server.rs` 后面的状态判断、失败转移、SSE、
  遮蔽还原、用量和 trace 都不改。读走多少就回多少 `credit`；客户端断开、
  流被丢弃时发 `abort`。
- 请求在路由前已遮蔽，插件拿到的是遮蔽后的内容，遮蔽仍是“做不到就不发”。
- 请求地址由 pansy 拼：`load` 返回的 `baseURL`（没有就用模型的 `url` 或
  provider 的 `api`）加上协议路径。
- 宿主在 head 之前报错 = 没发出去，转下一个账号或 provider；401 由宿主
  自己判断令牌是否过期（`staleAccess`），pansy 只让这个账号休息。
- 按 `maxConcurrency` 给每个插件 provider 一个信号量。
- 每个插件的载入错误原样展示（P0 里 gitlab 插件的两种失败就是例子）。
- 测试：fake 插件把请求转到本地 axum 假上游，覆盖流式、非流式、取消、
  慢读者（流控）、宿主崩溃后转移。

### P4 登录与续期

- 登录流程：`providers` 给出方法（oauth / api，及其问题）→
  `prompt`/`validate` 逐个问 → `authorize` 得到网址和说明（`auto` 或
  `code`）→ `callback`（`code` 方式带用户粘贴的码）；API key 走 `apiKey`。
  可取消（`abort`）。
- `keepSignedIn` 后台续期；`signIn` 事件更新账号状态；`signOut`、`reload`。
- CLI：`pansy plugin login <provider>` 打印网址并等待，`code` 方式从
  stdin 读码；`pansy plugin logout`。
- 窗口：复用订阅登录对话框的形状（`desktop/native` 的 `Subscriptions`
  旁边加 `Plugins` 模块），文字全进 `i18n.ts`。

### P5 middleware 与请求钩子

**宿主 B（`mw-host.js`）**：

- 载入 `magpie.middleware` 模块，按 `plugins.json` 的顺序组成一条链；
  `ctx.options` 来自插件的 `options`。不限制 import（Bun 能解析 npm 包），
  magpie 的“只能 import 相对路径”的模块照样能跑。
- 每个请求、每个 middleware 一份 `ctx.state`，同一请求的几个钩子共用。
- 钩子和返回值照 magpie：
  - `onRequest(body, ctx)`：返回对象替换请求，`undefined` 不改；
    `ctx.reject(status, message)` 拒绝，agent 收到它自己协议格式的错误。
  - `onAccount(account, ctx)`：每次在订阅或插件账号上尝试之前；
    `{skip: true}` 跳过这个账号，不让它休息，也不计失败。
  - `onProvider(body, ctx)`：每次尝试发出前，`ctx.provider` 说明是哪家；
    每次都从 `onRequest` 之后的请求体重新开始。
  - `onEvent(event, ctx)`：流式回复里，名字在 `export const events` 里的
    事件；返回对象替换，`null` 丢弃，`undefined` 不改。
  - `onResponse(body, ctx)`：非流式回复的整个 JSON，`ctx.status` 是状态码。
  - `strategies`：每个成为一种路由方式 `<插件短名>/<名字>`，返回账号的
    尝试顺序；`onUsage(row, options)`：每行用量记录的白名单字段，经一个
    1024 条的队列送达，满了就丢弃并计数。

**Rust 端（`pansy-gateway` 的 `Middleware` trait，宿主 B 是第一个实现）**：

- 位置：`onRequest` 在**遮蔽之前**（和 magpie 一样）；`onAccount`、
  `onProvider` 在每次尝试处；`onEvent`、`onResponse` 包在写给 agent 的回复
  上，看到的是遮蔽还原之后、agent 将收到的内容。pansy 自己发的请求（标题、
  搜索替身等）不经过 middleware。
- 没有任何 middleware 开着时，不碰宿主 B，不付任何开销；没有插件声明
  `onEvent` 时，流不切成事件。
- 一个流的事件连续发给宿主 B，不逐个等回复，回来时保持顺序。
- 时限由 Rust 强制：`onRequest`、`onAccount`、`onProvider`、`onResponse`
  各 250 ms，每个事件 50 ms，载入 1 s，`strategies` 20 ms，`onUsage` 250 ms。
  超时、报错或返回不合规的值都**放行**（请求按原样走），计一次失败，
  日志每分钟最多 60 行。Bun 打断不了跑飞的同步代码，所以一个钩子超时 5 s
  还没回来，就杀掉宿主 B 重启。
- `onAccount` 的 `account.ref`：用 `account-ref.key` 对“middleware 名 +
  账号的 agent、用户、套餐”（小写）做 HMAC-SHA256，取前 16 字节的十六进制。
  同一账号对同一 middleware 永远一样，换 middleware 就不同，没有密钥推不出
  邮箱。
- `strategies` 接进 `route.rs`：路由方式 `<插件>/<名字>` 原样存进
  providers.json；插件关掉时按列出的顺序尝试，再打开时恢复。
- `onUsage` 接在写 `usage/<day>.jsonl` 之后。

**宿主 A 的 `magpie.hooks`**：

- 异步请求钩子用 `host.js` 已有的 `gatewayRequest`：路由前
  `await onRequest(body, ctx)` 补充上下文，回复结束后通知 `onResponse`。
  按 magpie 的规则：出错、超时、返回非法数据，或改了工具声明、`model`、
  `stream`，就回退到原请求。
- 测试：用 magpie testdata 里的 `word-guard.middleware.js`、
  `system-prompt.hooks.mjs`，外加 think-tags、model-map 这类社区包的本地副本；
  覆盖每个钩子、拒绝、跳过账号、事件丢弃、超时放行、宿主 B 被杀后重启。

### P6 agent 插件

- 宿主 B 载入 `magpie.agent` 模块：`agent` 描述（`id`、`name`、`bin`、
  `config`、`format`、`model`、`prefix`、`ua`、`notice`、`startup`）和
  `connect({gateway, …})`，后者返回要写入的其他键。
- 描述缓存进 `agent-plugins.json`：`pansy` 列 agent、窗口画 Agents 页都读
  缓存，不启动 Bun；只有连接、断开和模型列表变化时才调 `connect`。
- `pansy-agents` 把每个描述变成一个 `Agent`，排在内置的后面；和内置 agent
  同 id 的拒绝。写入复用现有的“只改一个键、记下原值、断开时还原”
  （`files.rs`、`jsonedit.rs`、`state.rs`）。
- **先补 YAML 的原地编辑**：Aider 这类用 `.yml`，`pansy-agents` 现在只支持
  JSON/JSONC、TOML 和 `.env`。和现有格式一样，只改一个键，其余字节不动。
- 测试：magpie testdata 里的 `aider.agent.js`、`deva.agent.js`；连接、断开
  后文件逐字节还原、只有模型没有其他键的“仅连接”型。

### P7 pi package

- 只有装了 pi package 才装 pi（P0：pi 连同依赖约 300 MB），界面上说明体积。
- pi 装在 `plugins/` 旁边，宿主 A 经 `pi.js` 用 pi 自己的加载器载入；pi 的
  provider 一律按 Anthropic 协议进出。
- pi 的登录（OAuth 问答、`ctx.ui` 的对话框和选择器）在 pansy 的登录流程里
  问；数据在 `PANSY_HOME/pi/`，不碰用户的 `~/.pi`。
- 测试：`fakepi.ts`。

### P8 管理界面、发现、导入、文档

- CLI：`pansy plugin [list] | add | rm | on | off | update | use | options`。
- 窗口：侧栏 模型 › 插件（已装：类型标记 provider / pi / middleware /
  hooks / agent、开关、更新、删除、载入错误、middleware 的调用次数和失败数；
  发现：读 magpie-community 的 `registry.json`，列 `kind` 为 provider、
  middleware、agent 的，内置一份离线兜底）；“添加 provider”里加“来自插件”；
  路由方式的选择器列出插件给的 `strategies`。
- 从 magpie 导入：读 magpie 配置目录的 `plugins.json` 和
  `plugin-auth.json`，重装同样的 spec，复制账号（经用户确认）。
- 更新 `docs/parity.md`、`README`、`NOTICE.md`；写 `docs/plugins.md`
  （用户怎么装、作者要知道什么、和 magpie 的差异）。

## 以后（不在本计划）

`usage` / `checkin` / `calls`（用量窗口、签到、外部调用），`load`（模型负载），
插件选项 UI，tools 和 search 钩子（要先有 MCP），页面和面板插件（用 NSH 的
`WebView`），给 middleware 用的 QuickJS 沙箱后端，OpenCode V2 插件。

## 风险

| 风险 | 应对 |
| --- | --- |
| `host.js` 是 magpie 的代码，后续修复要跟 | 记录来源提交；写同步步骤；本地改动尽量只在入口处 |
| 插件是任意代码，以用户身份运行，没有沙箱 | `--ignore-scripts`；装第三方插件前提示；界面显示每个插件来自哪里 |
| middleware 能看到遮蔽前的提示词和代码 | Plugins 页在每个 middleware 上写明；装第三方 middleware 前单独确认 |
| 一个跑飞的 middleware 卡住宿主 B | 和宿主 A 分开；超时放行；超过 5 s 杀掉重启 |
| 宿主 B 崩溃时，过滤类 middleware（word-guard、privacy-gate）悄悄失效 | 按 magpie 的“出错放行”；但在窗口和日志里明确提示“middleware 未生效” |
| 有些订阅插件可能违反厂商条款 | 沿用 magpie 的提示文字 |
| Bun 下载大、部分平台跑不了 | 按需下载、校验、可指定本机 Bun；跑不了时明确报错 |
| 测试依赖 Bun | 无 Bun 时跳过并说明；CI 装 Bun |
| 两个进程（窗口和 CLI）各起宿主同时写账号 | `host.js` 已对 `plugin-auth.json` 加锁并在变更后重读 |
| Windows 的管道比 Linux 慢 | P5 开始前在 Windows 上重测事件往返；慢就把事件批量发送 |

## 规模估计

- Rust 约 5,000–6,000 行：`pansy-plugins` 约 1,800，provider 合并约 300，
  网关插件请求约 300，登录流程约 400，middleware 接入约 900，agent 插件约
  500，YAML 原地编辑约 400，CLI 约 400，`desktop/native` 约 400。
- JS：新写的 `mw-host.js` 约 800 行；复用的 `host.js` + `pi.js` 约 4,900 行。
- TSX 约 1,000 行。
