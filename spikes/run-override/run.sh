#!/usr/bin/env bash
# Whether an agent can be pointed at a gateway and a model for one run alone,
# without writing its config: by the environment or by its own flags. Two
# vendors that answer nothing listen on two ports; each agent runs once in a
# home with no config and once in a home whose config points at the other
# port (as `pansy <agent> <model>` leaves it), and the log says which port
# was asked for which model. Nothing outside the scratch directory is
# written: HOME and the XDG directories point into it.
#
#   spikes/run-override/run.sh [scratch] [agent…]
set -u
here=$(cd "$(dirname "$0")" && pwd)
scratch=${1:-$(mktemp -d)}
shift || true
agents=${*:-claude codex gemini opencode qwen crush}
mkdir -p "$scratch"
log=$scratch/asked.log
: >"$log"
RUN=47301 GLOBAL=47302
python3 "$here/fake.py" $RUN "$log" &
p1=$!
python3 "$here/fake.py" $GLOBAL "$log" &
p2=$!
trap 'kill $p1 $p2 2>/dev/null' EXIT
sleep 0.5

# A program's real file, past mise's shims: a shim of a tool that is not
# installed would install it.
real() {
  local found
  for found in "$HOME/.local/share/mise/installs/$1/latest/node_modules/.bin/$1" \
    "$HOME/.local/share/mise/installs/$1/latest/bin/$1" \
    "$HOME/.local/share/mise/installs/$1/latest/$1"; do
    [ -x "$found" ] && { echo "$found"; return; }
  done
  found=$(command -v "$1" 2>/dev/null) || return 1
  case $found in */mise/shims/*) return 1 ;; esac
  echo "$found"
}

# home <name>: a scratch home, and the environment that keeps an agent in it.
home() {
  h=$scratch/$1
  mkdir -p "$h/work" "$h/.config" "$h/.local/share" "$h/.cache"
  env=(env -i "PATH=$PATH" "HOME=$h" "XDG_CONFIG_HOME=$h/.config" "XDG_DATA_HOME=$h/.local/share"
    "XDG_CACHE_HOME=$h/.cache" "TERM=dumb" "NO_COLOR=1")
}

# try <agent> <case> <command…>: run it, and say what the vendors heard.
try() {
  local agent=$1 name=$2
  shift 2
  local before
  before=$(wc -l <"$log")
  (cd "$h/work" && timeout 60 "$@" </dev/null >"$scratch/$agent-$name.out" 2>&1)
  local heard
  heard=$(tail -n +$((before + 1)) "$log" | grep -v "model= " | sort -u | tr '\n' ';')
  [ -n "$heard" ] || heard=$(tail -n +$((before + 1)) "$log" | sort -u | head -3 | tr '\n' ';')
  printf '%-9s %-22s %s\n' "$agent" "$name" "${heard:-nothing asked (see $scratch/$agent-$name.out)}"
}

url=http://127.0.0.1:$RUN
other=http://127.0.0.1:$GLOBAL

for agent in $agents; do
  case $agent in
  claude)
    bin=$(real claude) || { echo "claude    not installed"; continue; }
    vars=("ANTHROPIC_BASE_URL=$url" ANTHROPIC_AUTH_TOKEN=run-key ANTHROPIC_MODEL=run/model
      ANTHROPIC_DEFAULT_HAIKU_MODEL=run/small ANTHROPIC_SMALL_FAST_MODEL=run/small)
    settings="{\"env\":{\"ANTHROPIC_BASE_URL\":\"$url\",\"ANTHROPIC_AUTH_TOKEN\":\"run-key\",\"ANTHROPIC_MODEL\":\"run/model\",\"ANTHROPIC_DEFAULT_HAIKU_MODEL\":\"run/small\",\"ANTHROPIC_SMALL_FAST_MODEL\":\"run/small\"}}"
    home claude-clean
    try claude clean-env "${env[@]}" "${vars[@]}" "$bin" -p hi
    home claude-wired
    mkdir -p "$h/.claude"
    echo "{\"env\":{\"ANTHROPIC_BASE_URL\":\"$other\",\"ANTHROPIC_AUTH_TOKEN\":\"global-key\",\"ANTHROPIC_MODEL\":\"global/model\",\"ANTHROPIC_DEFAULT_HAIKU_MODEL\":\"global/small\",\"ANTHROPIC_SMALL_FAST_MODEL\":\"global/small\"}}" >"$h/.claude/settings.json"
    try claude wired-env "${env[@]}" "${vars[@]}" "$bin" -p hi
    try claude wired-settings-flag "${env[@]}" "$bin" --settings "$settings" -p hi
    ;;
  codex)
    bin=$(real codex) || { echo "codex     not installed"; continue; }
    flags=(-c model=run/model -c model_provider=pansyrun -c model_providers.pansyrun.name=Pansy
      -c "model_providers.pansyrun.base_url=$url/v1" -c model_providers.pansyrun.wire_api=responses
      -c model_providers.pansyrun.experimental_bearer_token=run-key)
    home codex-clean
    try codex clean-flags "${env[@]}" "$bin" exec --skip-git-repo-check "${flags[@]}" hi
    home codex-wired
    mkdir -p "$h/.codex"
    printf 'model = "global/model"\nmodel_provider = "pansy"\n[model_providers.pansy]\nname = "Pansy"\nbase_url = "%s/v1"\nwire_api = "responses"\nexperimental_bearer_token = "global-key"\n' "$other" >"$h/.codex/config.toml"
    try codex wired-flags "${env[@]}" "$bin" exec --skip-git-repo-check "${flags[@]}" hi
    ;;
  gemini)
    bin=$(real gemini) || { echo "gemini    not installed"; continue; }
    # Trusting the folder is the spike's, to run without a terminal; `pansy run` leaves it to the user.
    vars=("GOOGLE_GEMINI_BASE_URL=$url" GEMINI_API_KEY=run-key GEMINI_MODEL=run/model GEMINI_CLI_TRUST_WORKSPACE=true)
    home gemini-clean
    try gemini clean-env "${env[@]}" "${vars[@]}" "$bin" -p hi
    try gemini clean-env-m "${env[@]}" "${vars[@]}" "$bin" -m run/model -p hi
    home gemini-wired
    mkdir -p "$h/.gemini"
    echo '{"model":{"name":"global/model"},"security":{"auth":{"selectedType":"gemini-api-key"}}}' >"$h/.gemini/settings.json"
    printf 'GOOGLE_GEMINI_BASE_URL=%s\nGEMINI_API_KEY=global-key\n' "$other" >"$h/.gemini/.env"
    try gemini wired-env "${env[@]}" "${vars[@]}" "$bin" -p hi
    try gemini wired-env-m "${env[@]}" "${vars[@]}" "$bin" -m run/model -p hi
    ;;
  qwen)
    bin=$(real qwen) || { echo "qwen      not installed"; continue; }
    vars=("OPENAI_BASE_URL=$url/v1" OPENAI_API_KEY=run-key OPENAI_MODEL=run/model)
    home qwen-clean
    try qwen clean-env "${env[@]}" "${vars[@]}" "$bin" -p hi
    home qwen-wired
    mkdir -p "$h/.qwen"
    echo '{"model":{"name":"global/model"},"security":{"auth":{"selectedType":"openai"}}}' >"$h/.qwen/settings.json"
    printf 'OPENAI_BASE_URL=%s/v1\nOPENAI_API_KEY=global-key\nOPENAI_MODEL=global/model\n' "$other" >"$h/.qwen/.env"
    try qwen wired-env "${env[@]}" "${vars[@]}" "$bin" -p hi
    try qwen wired-env-m "${env[@]}" "${vars[@]}" "$bin" -m run/model -p hi
    ;;
  opencode)
    bin=$(real opencode) || { echo "opencode  not installed"; continue; }
    content="{\"provider\":{\"pansy\":{\"npm\":\"@ai-sdk/openai-compatible\",\"name\":\"Pansy\",\"options\":{\"baseURL\":\"$url/v1\",\"apiKey\":\"run-key\"},\"models\":{\"run/model\":{\"name\":\"run/model\"},\"run/small\":{\"name\":\"run/small\"}}}},\"model\":\"pansy/run/model\",\"small_model\":\"pansy/run/small\"}"
    home opencode-clean
    try opencode clean-content "${env[@]}" "OPENCODE_CONFIG_CONTENT=$content" "$bin" run hi
    home opencode-wired
    mkdir -p "$h/.config/opencode"
    echo "{\"provider\":{\"pansy\":{\"npm\":\"@ai-sdk/openai-compatible\",\"name\":\"Pansy\",\"options\":{\"baseURL\":\"$other/v1\",\"apiKey\":\"global-key\"},\"models\":{\"global/model\":{\"name\":\"global/model\"}}}},\"model\":\"pansy/global/model\",\"small_model\":\"pansy/global/model\"}" >"$h/.config/opencode/opencode.json"
    try opencode wired-content "${env[@]}" "OPENCODE_CONFIG_CONTENT=$content" "$bin" run hi
    ;;
  crush)
    bin=$(real crush) || { echo "crush     not installed"; continue; }
    conf="{\"providers\":{\"pansy\":{\"name\":\"Pansy\",\"type\":\"openai\",\"base_url\":\"$url/v1\",\"api_key\":\"run-key\",\"models\":[{\"id\":\"run/model\",\"name\":\"run/model\",\"context_window\":128000,\"default_max_tokens\":16000}]}},\"models\":{\"large\":{\"model\":\"run/model\",\"provider\":\"pansy\"},\"small\":{\"model\":\"run/model\",\"provider\":\"pansy\"}}}"
    home crush-wired
    mkdir -p "$h/.config/crush" "$h/run"
    echo "${conf//$url/$other}" | sed 's/run-key/global-key/; s#run/model#global/model#g' >"$h/.config/crush/crush.json"
    echo "$conf" >"$h/run/crush.json"
    try crush wired-global-config "${env[@]}" "CRUSH_GLOBAL_CONFIG=$h/run" "$bin" run hi
    ;;
  esac
done
echo "run port $RUN, global port $GLOBAL; outputs in $scratch"
