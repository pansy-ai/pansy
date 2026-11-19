#!/usr/bin/env bash
# Runs the native module's Rust tests and the window's headless tests in a
# scratch home: never the user's agents, providers or pansy files.
set -euo pipefail
cd -- "$(dirname -- "${BASH_SOURCE[0]}")"
export PATH="$HOME/.cargo/bin:$PATH"
# The scratch HOME below must not hide the Rust toolchain nsh builds with.
export RUSTUP_HOME="${RUSTUP_HOME:-$HOME/.rustup}" CARGO_HOME="${CARGO_HOME:-$HOME/.cargo}"

scratch=$(mktemp -d)
trap 'rm -rf -- "$scratch"' EXIT
# An agent to connect: Claude Code, its settings folder and a program
# that says its version (the user's own is never run).
mkdir -p "$scratch/home/.claude" "$scratch/bin"
printf '#!/bin/sh\necho "2.1.0 (Claude Code)"\n' > "$scratch/bin/claude"
chmod +x "$scratch/bin/claude"
# And one whose install tells how it is updated: Codex as npm installs it
# under a prefix, an npm that "installs" the version asked for by writing
# the package's file, and a registry (a folder, served) that says which
# version is out. Neither the user's npm nor npm's registry is asked.
pkg="$scratch/prefix/lib/node_modules/@openai/codex"
mkdir -p "$pkg/bin" "$scratch/prefix/bin" "$scratch/home/.codex" "$scratch/registry/@openai/codex"
printf '{"name":"@openai/codex","version":"0.168.0"}\n' > "$pkg/package.json"
printf '#!/bin/sh\necho "codex-cli 0.168.0"\n' > "$pkg/bin/codex.js"
printf '#!/bin/sh\nprintf %%s '"'"'{"name":"@openai/codex","version":"0.170.0"}'"'"' > "%s/package.json"\n' "$pkg" > "$scratch/prefix/bin/npm"
chmod +x "$pkg/bin/codex.js" "$scratch/prefix/bin/npm"
ln -s "$pkg/bin/codex.js" "$scratch/bin/codex"
printf '{"name":"@openai/codex","version":"0.170.0"}\n' > "$scratch/registry/@openai/codex/latest"
registry_port=$(python3 -c 'import socket; s = socket.socket(); s.bind(("127.0.0.1", 0)); print(s.getsockname()[1])')
python3 -m http.server "$registry_port" --bind 127.0.0.1 --directory "$scratch/registry" > /dev/null 2>&1 &
registry_pid=$!
trap 'kill "$registry_pid" 2> /dev/null; rm -rf -- "$scratch"' EXIT
# OpenCode's settings folder: an agent that lists pansy's models itself,
# for Pick and the names in its list (its program is never run).
mkdir -p "$scratch/home/.config/opencode"
# A session of it for the Sessions page, written days ago (one written in
# the last minute may still be running, and is not deleted).
session="$scratch/home/.claude/projects/-work-app/11111111-2222-4333-8444-555555555555.jsonl"
mkdir -p "$(dirname -- "$session")"
cat > "$session" <<'EOF'
{"type":"user","cwd":"/work/app","sessionId":"11111111-2222-4333-8444-555555555555","timestamp":"2026-11-10T10:00:00.000Z","message":{"role":"user","content":"Fix the login bug"}}
{"type":"assistant","cwd":"/work/app","sessionId":"11111111-2222-4333-8444-555555555555","timestamp":"2026-11-10T10:00:30.000Z","message":{"id":"msg_0","role":"assistant","model":"claude-sonnet-4-5","content":[{"type":"tool_use","id":"toolu_1","name":"Grep","input":{"pattern":"login"}}]}}
{"type":"user","cwd":"/work/app","sessionId":"11111111-2222-4333-8444-555555555555","timestamp":"2026-11-10T10:00:31.000Z","message":{"role":"user","content":[{"type":"tool_result","tool_use_id":"toolu_1","content":"src/login.ts:3: export function login"}]}}
{"type":"assistant","cwd":"/work/app","sessionId":"11111111-2222-4333-8444-555555555555","timestamp":"2026-11-10T10:01:00.000Z","message":{"id":"msg_1","role":"assistant","model":"claude-sonnet-4-5","usage":{"input_tokens":1200,"output_tokens":300},"content":[{"type":"text","text":"The form now trims the e-mail before it checks it."}]}}
{"type":"ai-title","aiTitle":"Login bug fix","sessionId":"11111111-2222-4333-8444-555555555555"}
EOF
touch -t 202611101005 "$session"
# And one of OpenCode's, as its files were before its database: listed and
# resumed, never deleted.
oc="$scratch/home/.local/share/opencode/storage"
oc_id=ses_1a2b3c4d5e6fAAAAAAAAAAAAAA
mkdir -p "$oc/session/p1" "$oc/message/$oc_id" "$oc/part/msg_1" "$oc/part/msg_2"
printf '{"id":"%s","directory":"/work/api","title":"Health check endpoint","time":{"created":1794297600000,"updated":1794297660000}}\n' "$oc_id" > "$oc/session/p1/$oc_id.json"
printf '{"id":"msg_1","role":"user","time":{"created":1794297601000}}\n' > "$oc/message/$oc_id/msg_1.json"
printf '{"type":"text","text":"Add a health check"}\n' > "$oc/part/msg_1/prt_1.json"
printf '{"id":"msg_2","role":"assistant","modelID":"gpt-5.1","tokens":{"input":900,"output":80,"reasoning":0,"cache":{"read":0,"write":0}},"time":{"created":1794297602000,"completed":1794297660000}}\n' > "$oc/message/$oc_id/msg_2.json"
printf '{"type":"text","text":"Added GET /health."}\n' > "$oc/part/msg_2/prt_2.json"
find "$oc" -type f -exec touch -t 202611100801 {} +
# CC Switch's providers, which the Providers page offers to bring over
mkdir -p "$scratch/home/.cc-switch"
printf '{"claude":{"providers":{"a":{"name":"Switch Relay","settingsConfig":{"env":{"ANTHROPIC_BASE_URL":"https://relay.example.com","ANTHROPIC_AUTH_TOKEN":"sk-switch-test-0123456789","ANTHROPIC_MODEL":"relay-1"}}}}}}\n' > "$scratch/home/.cc-switch/config.json"
env PATH="$scratch/bin:$PATH" HOME="$scratch/home" PANSY_HOME="$scratch/home/.config/pansy" \
  XDG_CONFIG_HOME="$scratch/home/.config" XDG_DATA_HOME="$scratch/home/.local/share" \
  XDG_STATE_HOME="$scratch/home/.local/state" XDG_CACHE_HOME="$scratch/home/.cache" \
  CODEX_HOME= OPENCODE_DB= ANTHROPIC_BASE_URL= ANTHROPIC_API_KEY= OPENAI_BASE_URL= OPENAI_API_KEY= \
  LANG=en_US.UTF-8 LC_ALL= LC_MESSAGES= LANGUAGE= PANSY_LANGUAGE= \
  PANSY_NPM_REGISTRY="http://127.0.0.1:$registry_port" \
  nsh test "$@"
