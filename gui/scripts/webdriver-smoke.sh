#!/usr/bin/env bash
# Drive the real Nanna GUI over WebDriver on Linux, fully isolated, and prove
# its live path: the page mounts, the Tauri bridge exists, and a command
# reaches the app's own daemon sidecar. Writes a screenshot.
#
#   gui/scripts/webdriver-smoke.sh <nanna-gui binary> <out dir>
#
# Needs: a GUI built with `--features e2e-webdriver` (never a release build),
# its sidecar at gui/src-tauri/binaries/, `tauri-webdriver` on PATH
# (`cargo install tauri-webdriver --locked`), python3, curl, and a display.
#
# Isolation is the point. A GUI started with defaults attaches to whatever
# daemon holds :5149 — on a developer's machine, their own, with their real
# data. Everything here lives under <out dir>: HOME, config, the daemon's data
# dir, and a separate daemon port. GDK_BACKEND=x11 because on Wayland with
# the NVIDIA driver WebKitGTK dies at once with "Gdk Error 71".
set -euo pipefail

app="${1:?usage: webdriver-smoke.sh <nanna-gui binary> <out dir>}"
out="${2:?usage: webdriver-smoke.sh <nanna-gui binary> <out dir>}"
wd_port="${WD_PORT:-4444}"
daemon_port="${NANNA_DAEMON_PORT:-51990}"

test -x "$app" || { echo "not an executable: $app" >&2; exit 2; }
command -v tauri-webdriver >/dev/null || { echo "tauri-webdriver not on PATH" >&2; exit 2; }
for port in "$wd_port" "$daemon_port"; do
  if ss -ltn | grep -q ":$port "; then echo "port $port is already in use" >&2; exit 2; fi
done

mkdir -p "$out/home" "$out/data"
cat > "$out/config.toml" <<'EOF'
[scheduler]
heartbeat_enabled = false
EOF

# Stop what this script started, by PID — never by name (a developer's own
# GUI and daemon share those names).
cleanup() {
  if [[ -n "${session:-}" ]]; then
    curl -s -m 10 -X DELETE "http://127.0.0.1:$wd_port/session/$session" >/dev/null || true
  fi
  if [[ -n "${wd_pid:-}" ]]; then kill "$wd_pid" 2>/dev/null || true; fi
  sleep 2
  local sidecar
  sidecar="$(ss -ltnp | grep ":$daemon_port " | grep -o 'pid=[0-9]*' | head -1 | cut -d= -f2 || true)"
  if [[ -n "$sidecar" ]]; then
    echo "sidecar still up after the GUI ended (pid $sidecar); stopping it" >&2
    kill "$sidecar" 2>/dev/null || true
  fi
}
trap cleanup EXIT

env -u XDG_CONFIG_HOME -u XDG_DATA_HOME -u XDG_CACHE_HOME \
  HOME="$out/home" \
  NANNA_CONFIG_PATH="$out/config.toml" \
  NANNA_DEV_DATA_DIR="$out/data" \
  NANNA_DAEMON_PORT="$daemon_port" \
  DBUS_SESSION_BUS_ADDRESS=unix:path=/nonexistent \
  GDK_BACKEND=x11 \
  tauri-webdriver --port "$wd_port" >"$out/tauri-webdriver.log" 2>&1 &
wd_pid=$!
for _ in $(seq 1 50); do ss -ltn | grep -q ":$wd_port " && break; sleep 0.1; done

wd="http://127.0.0.1:$wd_port"
caps=$(python3 -c 'import json,sys; print(json.dumps({"capabilities":{"alwaysMatch":{"tauri:options":{"application":sys.argv[1]}}}}))' "$app")
session=$(curl -s -m 120 -X POST "$wd/session" -H 'content-type: application/json' -d "$caps" \
  | python3 -c 'import json,sys; print(json.load(sys.stdin)["value"]["sessionId"])')
echo "session $session"

# Run one async script; it must call its callback with a string.
run() {
  local body
  body=$(python3 -c 'import json,sys; print(json.dumps({"script":sys.argv[1],"args":[]}))' "$1")
  curl -s -m 90 -X POST "$wd/session/$session/execute/async" -H 'content-type: application/json' -d "$body" \
    | python3 -c 'import json,sys; print(json.load(sys.stdin)["value"])'
}

page=$(run 'const done = arguments[arguments.length - 1];
  const until = Date.now() + 30000;
  const check = () => {
    if (document.querySelector("#__nuxt") && window.__TAURI_INTERNALS__) {
      done(JSON.stringify({ title: document.title, url: location.href }));
    } else if (Date.now() > until) {
      done("TIMEOUT");
    } else {
      setTimeout(check, 250);
    }
  };
  check();')
echo "page: $page"
[[ "$page" != "TIMEOUT" ]] || { echo "the app never mounted" >&2; exit 1; }

# A command that must travel GUI -> Tauri -> the isolated sidecar and back.
status=$(run 'const done = arguments[arguments.length - 1];
  const tryOnce = (left) => window.__TAURI_INTERNALS__.invoke("get_mcp_servers")
    .then((r) => done(JSON.stringify(r)))
    .catch((e) => left > 0 ? setTimeout(() => tryOnce(left - 1), 1000) : done("ERR " + e));
  tryOnce(30);')
echo "get_mcp_servers: $status"
[[ "$status" != ERR* ]] || { echo "the sidecar never answered" >&2; exit 1; }

curl -s -m 30 "$wd/session/$session/screenshot" \
  | python3 -c 'import base64,json,sys; open(sys.argv[1],"wb").write(base64.b64decode(json.load(sys.stdin)["value"]))' \
    "$out/screenshot.png"
echo "screenshot: $out/screenshot.png"
echo "PASS"
