# 计划：照 T3 驱动 Antigravity（`agy acp`）

**状态：已查，未做。** 要等远程驱动里的通用 ACP 驱动（[remote.md](remote.md)）。
起因：Google 在官方论坛上说，运行官方原版的 antigravity-acp、用它自带的登录、
本人交互着用，就是合规的；T3 Code 就是这么做的。

依据：T3 Code 桌面版 0.1.11 的 `apps/server`（AppImage 里解出）；本机
Antigravity CLI 1.5.13，在临时 HOME 里实测，没有登录 Google 账号。

## T3 的做法

- **程序。** 没有单独的 antigravity-acp 包：ACP 是 Antigravity CLI 的
  `agy acp`，`antigravity-acp` 只是它的数据目录名。T3 不用用户的 agy，而是按
  Google 的清单（`antigravity-cli-auto-updater-974169037036.us-central1.run.app/manifests/<linux_amd64>.json`，
  给出 version、url、sha512）从 `storage.googleapis.com/antigravity-public/antigravity-cli/`
  下载到自己的目录，验 sha512，再关掉 agy 的自动更新；设置里也可以指定用户自己的
  agy。T3: `antigravityCliRelease.ts`、`AntigravityInstallation`。
- **启动。** `antigravity acp --gemini-dir <profile> --app-data-dir antigravity-acp`；
  `GEMINI_HOME` 也指向这个 profile，并设 `AGY_ACP_FORCE_FILE_STORAGE=1`（令牌存文件，
  不进钥匙串）和 `AGY_CLI_DISABLE_AUTO_UPDATE=1`；`GEMINI_API_KEY`、
  `GOOGLE_CLOUD_PROJECT` 这类环境变量都去掉。profile 是 T3 自己的目录，用户
  `~/.gemini` 里的 skills、`AGENTS.md`、`GEMINI.md` 链接进去。T3:
  `buildAntigravityAcpSpawnInput`、`antigravityEnvironment`、`linkAntigravityUserConfig`。
- **登录。** 登录完全交给 agy 自己的 `agy acp --login`：它打印 Google 的授权链接
  （跳回 `antigravity.google/oauth-callback`），用户把网页上显示的代码贴回来，T3
  再写进 agy 的 stdin。令牌由 agy 存进 profile 的 `antigravity-acp/acp_token.json`，
  T3 只转交链接和代码，从不读令牌。另外三种登录（Gemini Enterprise、Gemini API
  key、Agent Platform）也照 agy 的支持。T3: `codeLogin`、`makeAntigravityAuth`。

## 实测

- **没登录时。** `initialize` 返回 protocolVersion 1，支持 `loadSession`、
  `list`、`resume`、`close`，列出四种 authMethods；`session/new` 返回
  `-32000 Authentication required`。
- **profile 里写了网关时。** profile 的 `config/config.json` 写上 `gateway`
  （和 `antigravity.toml` 写进 `~/.gemini/config/config.json` 的是同一块）后，不登录
  也能 `session/new`，模型列表里多出网关的模型。Gemini 的几个模型仍然列着，选了
  它们会发生什么还没试。

## pansy 的做法

两种跑法，跑的都是 Google 的原版程序：

1. **用 Google 账号**，照 T3：登录交给 `agy acp --login`，pansy 不碰令牌。这就是
   Google 论坛上说的合规用法（官方程序、它自带的登录、本人交互着用）。
2. **用 pansy 的模型**：在 pansy 自己的 profile 里写网关，不登录 Google。效果和
   桌面上的"连接 Antigravity"一样，但写的是 pansy 自己的目录，不改用户的文件。

已定：agy 照 T3 下载到 pansy 自己的目录（约 270 MB），按平台放在
`tools/antigravity-acp/<平台>-<架构>/`。按 Google 的清单下载，验 sha512，关掉 agy
的自动更新，pansy 自己检查新版本；设置里可以改用用户自己的 agy。下载的写法照
`pansy-plugins/src/bun.rs`：第一次用到时才下载，测试里不下载。用户的 `~/.gemini`
只读不写：pansy 只在自己的 profile 里建链接指向它。

工作量是在通用 ACP 驱动之上，再加安装、profile、登录和选模型。
