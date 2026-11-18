# Agents as files

An agent pansy wires is a manifest: a TOML file that says how the agent is
found, where it keeps its config, the keys that point it at the gateway and
how one run of it is pointed elsewhere. pansy's own six are
[`crates/pansy-agents/agents/`](../crates/pansy-agents/agents); yours go in
`agents/` of pansy's folder (`~/.config/pansy/agents/*.toml`) and need no new
pansy. `pansy agents` says what is wrong with one it can't read.

```toml
id = "codex"                          # small letters, digits, dashes
name = "Codex"
protocol = "responses"                # anthropic, chat, responses or gemini
programs = ["codex"]                  # on PATH: it is installed
dirs = [".codex"]                     # under the home folder: it was
small_model = false                   # it has a second model for quick tasks

[files]                               # the main one first
config = "{env:CODEX_HOME|~/.codex}/config.toml"

[[set]]                               # the keys connecting sets, in order
file = "config"
key = ["model"]
value = "{model}"

[base_url]                            # where its base URL is read back
file = "config"
key = ["model_providers", "pansy", "base_url"]

[model]                               # and its model
file = "config"
key = ["model"]

[run]                                 # `pansy run`: one run, nothing written
verified = "0.169.0"                  # the version this was tried on
args = ["-c", "model={model:q}"]
```

- **Values** are text with `{base_url}`, `{v1}` (the base URL with `/v1`),
  `{key}`, `{model}` and `{small}` filled in; `{model:q}` is in quotes, as
  JSON and TOML write a string. A table or list of such works too. A table
  whose one key is `"$each_model"` becomes one entry for each model the
  gateway offers, by its id (`{id}`); alone in a list, one item each.
- **A file's format** is its ending's: `.toml`, `.yaml`, `.env`, else JSON
  with comments. In a path, `~/` is the home folder and `{env:NAME|else}` a
  variable's value or what follows the bar; several paths are the first that
  is there, else the last.
- **`[run]`** takes `args` (put before the user's), `[run.env]`,
  `[run.json_env.NAME]` (a variable whose value is JSON, laid over what it
  holds already), `[[run.json_arg]]` (`flag` and the JSON `value` after it),
  `note`, and `unsupported = "why"` for an agent that reads its files alone.
- **`[[install]]`** is `via`, `command` and the systems it is for (`os`:
  `linux`, `macos`, `windows`; all, without it).

Connecting, disconnecting, profiles, Details and the preview work for a
manifest's agent as for pansy's own. The gateway names a request's agent by
its user agent, a list of its own: a new agent's requests are counted under
`other` until it is added there.

## One run on another model: `pansy run`

How each agent takes a model for one run was tried on the real programs in a
scratch home (`spikes/run-override/run.sh`), each once with no config and once
with a config pointing elsewhere, as `pansy <agent> <model>` leaves it:

| Agent | Tried on | How | What was learned |
| :-- | :-- | :-- | :-- |
| Claude Code | 2.1.325 | `--settings '{"env":{…}}'` | Its settings' `env` wins over the environment it starts in, so variables alone do nothing once it is connected; `--settings` wins over both |
| Codex | 0.169.0 | `-c key=value` | Wins over `config.toml` |
| Gemini CLI | 0.65.0 | `GOOGLE_GEMINI_BASE_URL`, `GEMINI_API_KEY`, `GEMINI_MODEL` | The environment wins over `~/.gemini/.env` and the settings' model. Without a terminal (`-p`) it takes the key only when its settings say `gemini-api-key`, as connecting leaves them |
| OpenCode | 2.3.0 | `OPENCODE_CONFIG_CONTENT` | A config of its own, laid over the user's |
| Qwen Code | not tried | `OPENAI_*` and `-m` | Not installed where this was tried; as its fork of Gemini CLI should take them |
| Crush | — | — | Reads its providers from its config files alone: `pansy run crush` says to connect it instead |
