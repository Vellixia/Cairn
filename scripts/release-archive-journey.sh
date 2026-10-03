#!/usr/bin/env bash
set -euo pipefail

bin=$(cd "${1:?usage: scripts/release-archive-journey.sh ARCHIVE_BIN_DIR}" && pwd)
database_url=${CAIRN_TEST_DATABASE_URL:?CAIRN_TEST_DATABASE_URL is required}
work=$(mktemp -d)
isolated_home="$work/home"
repo="$work/repo"
server_log="$work/server.log"
socket="$isolated_home/cairnd.sock"
journey_remote="https://github.com/example/archive-journey-${RANDOM}-${RANDOM}.git"
server_pid=
daemon_pid=
port=$(python3 -c 'import socket; s=socket.socket(); s.bind(("127.0.0.1", 0)); print(s.getsockname()[1]); s.close()')
server_url="http://127.0.0.1:$port"

cleanup() {
  test -z "$daemon_pid" || kill "$daemon_pid" 2>/dev/null || true
  test -z "$server_pid" || kill "$server_pid" 2>/dev/null || true
  rm -rf "$work"
}
trap cleanup EXIT

CAIRN_ADMIN_EMAIL=archive-admin@example.test \
CAIRN_ADMIN_PASSWORD=hunter2hunter2 \
DATABASE_URL="$database_url" \
"$bin/cairn-server" --addr "127.0.0.1:$port" --web-origin http://127.0.0.1:13100 \
  >"$server_log" 2>&1 &
server_pid=$!
for _ in $(seq 1 100); do
  curl -fsS "$server_url/api/health" >/dev/null 2>&1 && break
  sleep 0.5
done
curl -fsS "$server_url/api/health" >/dev/null || { cat "$server_log"; exit 1; }
curl -fsS -D "$work/allowed-origin.headers" -o /dev/null \
  -H 'origin: http://127.0.0.1:13100' "$server_url/api/health"
rg -qi '^access-control-allow-origin: http://127\.0\.0\.1:13100' "$work/allowed-origin.headers"
curl -fsS -D "$work/denied-origin.headers" -o /dev/null \
  -H 'origin: https://unrelated.example.test' "$server_url/api/health"
! rg -qi '^access-control-allow-origin:' "$work/denied-origin.headers"

headers="$work/login.headers"
curl -fsS -D "$headers" -o /dev/null -X POST "$server_url/api/auth/login" \
  -H 'content-type: application/json' \
  --data '{"email":"archive-admin@example.test","password":"hunter2hunter2"}'
cookie=$(sed -n 's/^set-cookie: \([^;]*\).*/\1/ip' "$headers" | head -n1)
test -n "$cookie"
token_json="$work/token.json"
curl -fsS -o "$token_json" -X POST "$server_url/api/tokens" \
  -H "cookie: $cookie" -H 'content-type: application/json' --data '{"name":"archive-journey"}'
token=$(python3 -c 'import json,sys; print(json.load(open(sys.argv[1]))["token"])' "$token_json")
curl -fsS "$server_url/api/auth/me" -H "authorization: Bearer $token" >"$work/me.json"
test -s "$work/me.json"
curl -fsS -X POST "$server_url/api/projects" \
  -H "authorization: Bearer $token" -H 'content-type: application/json' \
  --data "{\"name\":\"Archive journey\",\"repository_remote\":\"$journey_remote\"}" >"$work/project.json"

mkdir -p "$isolated_home/.codex" "$isolated_home/.claude" "$repo"
git -C "$repo" init -q
git -C "$repo" remote add origin "$journey_remote"
env HOME="$isolated_home" XDG_CONFIG_HOME="$isolated_home/.config" CAIRN_HOME="$isolated_home" CAIRN_SOCKET="$socket" \
  "$bin/cairnd" --socket "$socket" >"$work/daemon.log" 2>&1 &
daemon_pid=$!
(cd "$repo" && env HOME="$isolated_home" XDG_CONFIG_HOME="$isolated_home/.config" CAIRN_HOME="$isolated_home" CAIRN_SOCKET="$socket" \
  CAIRN_SERVER_URL="$server_url" CAIRN_SERVER_TOKEN="$token" \
  CAIRN_WEB_URL=http://127.0.0.1:13100 "$bin/cairn" --json setup) >"$work/setup.json"
test -s "$work/setup.json"
test -f "$isolated_home/.codex/config.toml"
test -f "$isolated_home/.claude.json"
test -f "$repo/.claude/settings.local.json"
python3 -c 'import json,sys; assert json.load(open(sys.argv[1]))["data"]["project"]["linked"] is True' "$work/setup.json"

# Setup must be repeatable without rewriting an already-owned integration.
(cd "$repo" && env HOME="$isolated_home" XDG_CONFIG_HOME="$isolated_home/.config" CAIRN_HOME="$isolated_home" CAIRN_SOCKET="$socket" \
  CAIRN_SERVER_URL="$server_url" CAIRN_SERVER_TOKEN="$token" "$bin/cairn" --json setup) >"$work/setup-rerun.json"
python3 -c 'import json,sys; x=json.load(open(sys.argv[1]))["data"]["integrations"]; assert x["applied"] == [] and x["warnings"] == [], x' "$work/setup-rerun.json"

# A user edit to Cairn's MCP entry remains untouched and is reported.
python3 - "$isolated_home/.claude.json" <<'PY'
import json, sys
path = sys.argv[1]
config = json.load(open(path))
config["mcpServers"]["cairn"]["command"] = "user-edit"
with open(path, "w") as file:
    json.dump(config, file)
PY
(cd "$repo" && env HOME="$isolated_home" XDG_CONFIG_HOME="$isolated_home/.config" CAIRN_HOME="$isolated_home" CAIRN_SOCKET="$socket" \
  CAIRN_SERVER_URL="$server_url" CAIRN_SERVER_TOKEN="$token" "$bin/cairn" --json setup) >"$work/setup-conflict.json"
python3 - "$work/setup-conflict.json" "$isolated_home/.claude.json" <<'PY'
import json, sys
result = json.load(open(sys.argv[1]))["data"]["integrations"]
config = json.load(open(sys.argv[2]))
assert result["warnings"] and config["mcpServers"]["cairn"]["command"] == "user-edit", result
PY

kill "$daemon_pid"
wait "$daemon_pid" || true
daemon_pid=
env HOME="$isolated_home" XDG_CONFIG_HOME="$isolated_home/.config" CAIRN_HOME="$isolated_home" CAIRN_SOCKET="$socket" \
  "$bin/cairnd" --socket "$socket" >"$work/daemon-restart.log" 2>&1 &
daemon_pid=$!

for actor in archive-actor-a archive-actor-b; do
  printf '{"session_id":"%s","source":"startup"}\n' "$actor" | \
    (cd "$repo" && env HOME="$isolated_home" XDG_CONFIG_HOME="$isolated_home/.config" CAIRN_HOME="$isolated_home" CAIRN_SOCKET="$socket" CAIRND_BIN="$bin/cairnd" \
      "$bin/cairn" hook SessionStart --agent codex)
done

for event in SessionStart PreCompact PostCompact SessionEnd; do
  printf '{"session_id":"archive-claude","source":"startup","trigger":"manual"}\n' | \
    (cd "$repo" && env HOME="$isolated_home" XDG_CONFIG_HOME="$isolated_home/.config" CAIRN_HOME="$isolated_home" CAIRN_SOCKET="$socket" CAIRND_BIN="$bin/cairnd" \
      "$bin/cairn" hook "$event") >"$work/claude-$event.json"
done
python3 - "$work/claude-SessionStart.json" <<'PY'
import json, sys
reply = json.load(open(sys.argv[1]))
assert reply["hookSpecificOutput"]["hookEventName"] == "SessionStart", reply
PY
python3 - "$isolated_home/edge.sqlite3" <<'PY'
import sqlite3, sys
db = sqlite3.connect(sys.argv[1])
assert db.execute("SELECT count(*) FROM sessions WHERE agent_session_key='archive-claude' AND agent='claude-code' AND status='completed'").fetchone()[0] == 1
assert db.execute("SELECT count(*) FROM command_spool WHERE kind='handoff_generate'").fetchone()[0] >= 1
PY

mcp_tool() {
  local name=$1
  local arguments=$2
  local request result
  request=$(ARCHIVE_REPO="$repo" python3 - "$name" "$arguments" <<'PY'
import json, os, sys
print(json.dumps({"jsonrpc":"2.0", "id":1, "method":"tools/call", "params":{
    "name":sys.argv[1], "arguments":dict(json.loads(sys.argv[2]), cwd=os.environ["ARCHIVE_REPO"])
}}))
PY
)
  result=$(printf '%s\n' "$request" | env HOME="$isolated_home" XDG_CONFIG_HOME="$isolated_home/.config" CAIRN_HOME="$isolated_home" CAIRN_SOCKET="$socket" CAIRND_BIN="$bin/cairnd" "$bin/cairn" mcp)
  printf '%s\n' "$result" | python3 -c 'import json,sys; r=json.load(sys.stdin)["result"]; assert not r["isError"], r; print(r["content"][0]["text"])'
}

remember='{ "action":"create", "agent_session_key":"archive-actor-a", "type":"fact", "topic_key":"archive.journey", "value_key":"remembered", "content":"archive journey keeps its remembered fact" }'
remembered=$(mcp_tool cairn_remember "$remember")
printf '%s' "$remembered" | rg -q '"accepted_for_delivery": true'

# A second caller proves the installed MCP process reads durable state after restart.
for _ in $(seq 1 100); do
  recalled=$(mcp_tool cairn_search '{ "action":"search", "agent_session_key":"archive-actor-b", "query":"archive journey remembered fact" }')
  printf '%s' "$recalled" | rg -q 'archive journey keeps its remembered fact' && exit 0
  sleep 0.1
done
exit 1
