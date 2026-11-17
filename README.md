<div align="center">

<img src="assets/brand/pansy-icon-256.png" width="112" alt="pansy">

# pansy

### Every agent's model. One place.

**Claude Code on Kimi. Codex on DeepSeek. Gemini CLI on GLM.**<br>
Pick any model for any agent; one local gateway serves them all,<br>
and when a key runs out of quota, the next one quietly takes over.

**English** · [简体中文](README.zh-CN.md)

</div>

pansy is a rewrite of [magpie](https://github.com/yetone/magpie) in Rust, with
its desktop window built on [Native Shell](https://nsh.dev) (NSH) 0.5.1. As the
three petals of its logo meet at one point, your agents and your providers meet
in one gateway.

<picture>
  <source media="(prefers-color-scheme: dark)" srcset="assets/agents-dark.png">
  <img src="assets/agents-light.png" width="900" alt="pansy's Agents page: Claude Code on DeepSeek, Codex on the Daily routing group, each picked from one list">
</picture>

## What it does

- **Any model, any agent.** Pick a model for Claude Code, Codex, Gemini CLI,
  OpenCode, Qwen Code or Crush from one list. pansy edits the one setting that
  matters in the agent's own config; comments, order and formatting stay as
  they were, and *Disconnect* puts back exactly what pansy changed.
- **One gateway, four protocols.** `127.0.0.1:7267` speaks Anthropic Messages,
  OpenAI Chat Completions, OpenAI Responses and Gemini, and translates between
  them — streaming, tool calls, reasoning and Codex's free-form tools included.
  When an agent and a provider speak the same protocol the request passes
  through untouched.
- **Keys that fail over.** A provider can hold several keys, tried in order. A
  key that is rate-limited, out of quota or refused rests for a while and the
  next one answers, before the agent sees an error.
- **Subscriptions.** A ChatGPT plan, as Codex uses it, GitHub Copilot's and
  Gemini Code Assist's, as Gemini CLI uses it, serve every agent: pansy reads
  the sign-in Codex, Copilot's editors and Gemini CLI already have, where they
  keep it, and signs in to more accounts itself (in the browser or by a
  code). Each is a provider (`codex/…`, `copilot/…`, `gemini-cli/…`) whose
  accounts fail over like keys; what each has left of its allowance is on
  the Usage page and in `pansy limits`, and one its vendor holds is passed
  over until its window renews.
- **Routing groups.** `group/daily` is several models picked as one, in order,
  taking turns or least used first; groups can contain groups.
- **Profiles.** Save every agent's model under one name and switch them all
  back in one move.
- **Sessions.** Claude Code's, Codex's and OpenCode's sessions on this
  computer, by the folder they ran in: what each spent, the command that
  resumes it, what was said in it, found by its words. Delete moves a
  session's files into pansy's trash, which puts them back (OpenCode's live in
  a database it keeps open, so they are only listed).
- **Usage.** Requests, tokens and cache reads by day, model, agent and
  provider, counted on this computer. Prompts and replies are never kept.
- **Masking.** API keys, tokens, passwords, personal data and your own words
  leave this computer as placeholders (`{{API_KEY_k3j9x2ab}}`) and come back as
  they were in the answer, streamed or not. Each kind can instead warn or hold
  the request back; Insights › Masking shows what each vendor saw, never
  keeping the values.
- **60 presets, as magpie lists them.** Anthropic, OpenAI, Gemini, DeepSeek,
  xAI, Kimi and Kimi Code, Zhipu GLM, Z.ai, MiniMax, StepFun, Xiaomi MiMo,
  LongCat, Baidu Qianfan, Tencent Cloud, Huawei Cloud, Volcengine Ark, Qwen and
  Alibaba's Token Plans, Mistral, Groq, Amazon Bedrock, Azure OpenAI, Ollama
  Cloud, Nous Portal, OpenRouter, Vercel AI Gateway, OpenCode Go and Zen, Kilo,
  SiliconFlow, NVIDIA NIM, ModelScope, AiHubMix, Ollama, LM Studio, oMLX,
  GPUStack… each with magpie's logo, its coding plans or regions picked in its
  editor, and any OpenAI-, Anthropic- or Gemini-compatible URL.

## Quick start

```sh
pansy provider add deepseek sk-…            # a preset needs only the key
pansy claude deepseek/deepseek-chat         # Claude Code on DeepSeek
pansy codex moonshot/kimi-k2-0905-preview   # Codex on Kimi
pansy group add "Daily" models=deepseek/deepseek-chat,moonshot/kimi-k2-0905-preview
pansy gemini group/daily                    # fails over between them
pansy save work && pansy use work           # profiles
pansy usage                                 # what went through the gateway
pansy settings set masking '{"secrets": true}'   # keys go to vendors as placeholders
pansy mask "my key is sk-proj-…"            # what a vendor would see
pansy serve                                 # the gateway without a window
pansy doctor                                # whether the gateway, providers and agents work
pansy test deepseek/deepseek-chat           # one short message through the gateway
pansy pause all && pansy resume all         # the agents on their own configs for now, and back
```

Or open the window (`pansy` without arguments): **Providers → Add provider**,
then pick a model for each agent on **Agents**. Closing the window keeps pansy
in the tray, so the gateway goes on serving your agents.

Anything that takes a base URL can use the gateway too:

```sh
export OPENAI_BASE_URL=http://127.0.0.1:7267/v1     OPENAI_API_KEY=pansy
export ANTHROPIC_BASE_URL=http://127.0.0.1:7267     ANTHROPIC_API_KEY=pansy
export GOOGLE_GEMINI_BASE_URL=http://127.0.0.1:7267 GEMINI_API_KEY=pansy
```

## Building

```sh
# The core, the gateway and the headless `pansy` command
cargo test --workspace && cargo build --release -p pansy-cli

# The desktop app (NSH 0.5.1)
cd desktop
nsh check && ./test.sh                     # Rust and headless UI tests in a scratch home
cargo build -p pansy-desktop && nsh build App.tsx --out target/debug/nsh
./target/debug/pansy                       # the window, with the gateway beside it
packaging/build.sh                         # dist/pansy-<version>-linux-<arch>.tar.gz
```

## Layout

| Path | What it is |
| :-- | :-- |
| `crates/pansy-core` | Providers, keys, models, presets, groups, settings, usage, masking's records, the subscriptions' sign-ins (`subscription/`); all files under `~/.config/pansy` (`PANSY_HOME`) |
| `crates/pansy-mask` | Masking: the rules that find secrets and personal data, placeholders, and putting the values back in streamed answers |
| `crates/pansy-gateway` | The gateway: the four protocols read into one shape and written back out (`ir.rs`, `proto/`), routing and failover (`route.rs`), what the Routing page plays (`trace.rs`), the HTTP side (`server.rs`) |
| `crates/pansy-agents` | The agents: finding them and their versions, editing their JSON/JSONC, TOML, `.env` and YAML configs in place, what was changed, profiles, their sessions (`sessions/`) |
| `crates/pansy-plugins` | magpie's community plugins on Bun: installing them, the hosts they run in, sign-ins, middleware, agent plugins ([docs/plugins.md](docs/plugins.md)) |
| `crates/pansy-cli` | The `pansy` command |
| `desktop/` | The NSH app: `App.tsx` and `ui/` (TSX), `native/` (`pansy/native`, the Rust the TSX calls), `runner/` (the program) |
| `assets/brand` | The pansy logo: the app icon (SVG, PNG, ICO) and the one-color glyph |

## How far the rewrite has come

This first pass ports magpie's core, its rule-based masking and the ChatGPT,
Copilot and Gemini Code Assist subscriptions, and magpie's community plugins
(OpenCode auth plugins, pi packages, middleware, agents; see
[docs/plugins.md](docs/plugins.md)). Still to come: Claude's,
Antigravity's and Grok's subscriptions and the allowances, masking with a local model, quotas and costs, intent routing and
the `smart`/`pace` modes, the library (instructions, MCP servers, skills),
search and image tools, sync, remote pansy and LAN gateway keys per client,
the TUI, and the rest of magpie's 80+ agents. See
[docs/parity.md](docs/parity.md).

## License

[MIT](LICENSE). pansy is a rewrite of magpie by yetone; see
[NOTICE.md](NOTICE.md).
