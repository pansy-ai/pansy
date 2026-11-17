# P0 结果：从 Rust 驱动 magpie 的插件宿主

[计划](plugin-host.md) 的第一步。结论：**通过**，可以进 P1；但计划要按下面
“对计划的修改”调整三处。

## 做了什么

`spikes/plugin-host/`（独立的 cargo 工作区，不进 pansy 的构建）：

- `run.sh` 在 scratch 目录里准备一切：下载 Bun 1.4.2 并用官方
  `SHASUMS256.txt` 校验；从 magpie 最后一个提交（`5d23ae860`）取出
  `host.js`、`pi.js` 和测试用的 fake 插件。scratch 目录以外什么都不写，
  `HOME` 也指向 scratch。
- `src/main.rs`（约 500 行）：起一个本地假厂商（axum），用 Bun 跑
  `host.js`，按 stdio 协议发 `init → providers → apiKey → load → fetch`，
  把 `fetch` 的 head/chunk 包成 `reqwest::Response`（网关读上游回复用的就是
  这个类型），并检查流控、取消和宿主崩溃。

运行：`spikes/plugin-host/run.sh [scratch目录]`。

## 结果（Linux x86_64，glibc 2.44）

| 检查 | 结果 |
| --- | --- |
| Bun 1.4.2 下载并校验 | 通过；`bun-linux-x64.zip` 36.6 MB |
| `init` 载入 fake 插件 | 通过，冷启动 42–88 ms |
| `providers` | 通过；按模型的 `npm` 映射出 chat / anthropic / gemini |
| `apiKey` 登录 | 通过；`plugin-auth.json` 是 0600 |
| `load` | 通过；拿到 loader 的 `baseURL`（请求地址要用它拼） |
| 非流式 `fetch` | 通过；厂商收到插件签的 `Bearer sk-spike`，JSON 原样回来 |
| 小请求开销 | 约 0.9–1.2 ms/次（连续 50 次，经 Bun 往返） |
| 流式 + 慢读者 | 4 MiB，26–34 MiB/s；宿主最多领先 128 KiB，正好等于给的窗口 |
| 读者放手即取消 | 厂商的流在 5–6 ms 内被关掉 |
| 8 个流并发 | 4 MiB，一个字节不差 |
| 宿主中途被杀 | 读者 3 ms 内拿到错误；之后的调用立刻失败而不是等超时 |
| 主动事件 | 登录时收到 `auth` 事件 |

协议和计划里写的一致：`fetch` 的回复是 head、若干 chunk（base64）、
`{result:null}`；每个 chunk 占窗口“字节数 + 2048”，读走后用 `credit`
归还；`abort` 之后宿主不再回答这个 id。

### 真实插件

在 scratch 里用 `bun add --ignore-scripts` 装了 4 个真实插件和 pi，只做
`init` 和 `providers`（不登录）：

| 插件 | 结果 |
| --- | --- |
| `@magpie-community/opencode-workbuddy-auth` 0.2.35 | 载入；`workbuddy`、`workbuddy-ai` 两个 provider，15/16 个模型，3 种登录方式 |
| `@magpie-community/opencode-cursor-auth` 0.2.48 | 载入；`cursor`，3 种登录方式 |
| `pi-provider-litellm` 4.8.0（pi package） | 载入；`litellm`，SSO 和 API key 两种登录；未登录时 0 个模型 |
| `opencode-gitlab-auth` 3.0.0 | **失败**："exports no OpenCode plugin function"。它用的是 OpenCode **V2** 插件接口（`@opencode/plugin` 的 `Plugin.define`），magpie 的宿主只支持 V1 |
| `opencode-gitlab-auth` 2.1.0 | **失败**：宿主把模块导出的每个函数都当插件调用，它额外导出的工具函数 `normalizeOptionalInstanceUrl` 被调用后抛错 |

安装：6 个包（含 pi-coding-agent 和 pi-ai）共 296 个依赖，29 秒，
`node_modules` 301 MB，大头是 pi。Bun 默认拦下了 2 个 postinstall 脚本。

## `host.js` / `pi.js` 里和 magpie 绑定的地方

都**不需要改**就能在 pansy 下跑；为了兼容社区插件，大部分也**不应该改**：

- 插件能看到的：`input.magpie`（`option`、`options`、`load`、`usage`、
  `account`）、`project.id: "magpie"`、package.json 的 `magpie.icon` 等字段、
  `Symbol.for("magpie.fellBack")`、错误信息里的 “magpie”。
- 协议上的：`x-magpie-*` 请求头（宿主会在转给厂商前去掉）、
  回复头 `x-magpie-sign-in`、子进程的代理变量 `MAGPIE_*_PROXY`。
- pi 的内部地址 `http://pi.magpie/v1`（只在宿主内部用）。

建议：照原样复用，只加一个 `input.pansy` 指向同一对象；pansy 这边沿用
`x-magpie-*` 和 `MAGPIE_*` 这些名字。

**只能用 Bun**：`host.js` 把 `Bun.$` 交给插件（OpenCode 的接口就是这样），
还用了 `Bun.gc`、`Bun.inspect`；`pi.js` 用 `Bun.resolveSync` 找 pi。

## 对计划的修改

1. **去掉“以后支持 Node”。** 宿主和插件接口都依赖 Bun（`Bun.$`），换 Node
   等于重写宿主。
2. **OpenCode V2 插件不在第一版范围内，但要有明确提示。** 判断方法：
   依赖 `@opencode/plugin`、或默认导出是 `Plugin.define(...)` 的结果。装的时候
   就告诉用户“这是 OpenCode V2 插件，暂不支持”，而不是载入失败才报错。
   以后要支持得自己扩展宿主（magpie 没做）。
3. **pi 很大**，按需装：只有装了 pi package 才装 pi-coding-agent，并在界面上
   说明体积。

另外两点补进 P1/P3：

- 请求地址由 pansy 拼：`load` 返回的 `baseURL`（没有就用模型的 `url` 或
  provider 的 `api`）加上协议路径。
- 每个插件的载入错误要原样展示（像上面 gitlab 的两种），这是用户最常遇到的
  问题。

## 没验证的

- **macOS 和 Windows。** 这台机器只有 Linux；`run.sh` 已经能识别 macOS 和
  Windows（Git Bash）并下载对应的 Bun，需要在那两台机器上跑一次。
- 真实插件的登录和请求（需要真实账号）。
- OAuth 浏览器登录流程（fake 插件支持，P4 再测）。
