#!/usr/bin/env bash
# Show the backdrop, live. One command, a window opens, edits appear on their own.
#
# The server exists only because a browser refuses an ES module import over
# file://; you should never have to think about it, so this starts it and opens
# the page itself.
set -euo pipefail
cd "$(dirname "$0")/.."

port="${1:-8765}"
while lsof -i ":$port" >/dev/null 2>&1; do port=$((port + 1)); done

python3 -m http.server "$port" --bind 127.0.0.1 >/dev/null 2>&1 &
server=$!
trap 'kill $server 2>/dev/null || true' EXIT

# Wait for it to answer before opening, or the tab loads an error page.
url="http://localhost:$port/tools/backdrop-preview.html"
for _ in $(seq 1 50); do
  curl -fsS -o /dev/null "$url" && break
  sleep 0.1
done

open "$url"
echo "Backdrop preview open. Edits to ui/js/backdrop.js appear on their own. Ctrl-C to stop."
wait $server
