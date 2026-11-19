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
  gateway offers the agent, by its id (`{id}`) and the name it is listed by
  (`{name}`); alone in a list, one item each. The user picks which models an
  agent lists and whether a name leads with its provider (`pansy <agent>
  list`, `pansy <agent> names`).
- **A key** is the steps down to it. A step in brackets picks an item of a
  list: `[id=pansy]` the item whose `id` is `pansy`, `[=pansy/{model}]` the
  item that is that text. Setting it replaces that item or adds it after the
  others, which stay as they are written; disconnecting takes out that item
  alone. JSON and TOML (`[[providers]]` tables, or an array) have them, and
  a JSON file may be a list itself (`key = ["[id=pansy]"]`). A step takes
  `{model}` and the other variables too (`["models", "pansy/{model}"]`).
- **A `[[set]]`** may also say:
  `each_model = true` — one key for each model the agent lists, `{id}` and
  `{name}` in the key and the value being that model's
  (`key = ["models", "pansy/{id}"]`), beside whatever else the table holds;
  a model that leaves the list has its key put back;
  `remove = true` (and no value) — the key is taken out while the agent is
  connected and put back after;
  `if_there = true` — set only where what holds the key (the list, for an
  item of one) is in the file already.
- **In YAML** a table is written as its keys, one by one; a list is not
  written (pansy edits YAML a line at a time).
- **`[base_url]` and `[model]`** say where the gateway's address and the
  model are read back. An agent that keeps neither where it can be read
  leaves them out: pansy then goes by what it wrote.
- **A file's format** is its ending's: `.toml`, `.yaml`, `.env`, else JSON
  with comments. In a path, `~/` is the home folder and `{env:NAME|else}` a
  variable's value or what follows the bar; several paths are the first that
  is there, else the last.
- **`[[field]]`** is a setting of the agent's own beside its model: a `key`
  (small letters, digits, `_`), a `kind` (`model` or `effort`) and, for an
  effort, its `levels`. A value names it as `{key}`; a `[[set]]` that names a
  field left unset is left out, and what the key held is put back. A model
  with `follows = true` is the agent's model when unset; an effort with
  `joins = ":"` is written as `:high` after a model's name, or as nothing.
  `pansy <agent> <key> <value>` sets one, `default` unsets it.
  The kinds `tokens` (a size as `200k`, `1m` or digits, within `min` and
  `max`), `percent` (1 to 100) and `switch` (`on`) are the rows of the
  agent's settings sheet. `{key:n}` alone as a value writes a number, not
  text; a `[[set]]` with `when = "<key>"` is left out until that field has a
  value, whatever else it names.
- **`[sign_in]`** is a `file` and a `key` in it that is there once the agent
  is signed in to its own account. Only that the key is there is read, never
  what it holds.
- **`npm`** is the agent's package in npm's registry, where its newest
  version is looked up; **`self_update`** the arguments of its own updater
  (`["update"]`), run when it was not installed by a package manager.
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
