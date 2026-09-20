#!/usr/bin/env bash
# 冒烟：用 stub 引擎起服务，走一遍「送一段音频 → 出句子 → 导出四个文件」。
set -euo pipefail
cd "$(dirname "$0")/.."

PORT="${SMOKE_PORT:-8801}"
DATA_DIR="$(mktemp -d /tmp/mind_flow_smoke.XXXXXX)"
BIN="${1:-target/debug/mind_flow}"

if [[ ! -x "$BIN" ]]; then
  echo "找不到二进制 $BIN，先 cargo build" >&2
  exit 1
fi

cleanup() {
  if [[ -n "${SERVER_PID:-}" ]]; then kill "$SERVER_PID" 2>/dev/null || true; fi
  rm -rf "$DATA_DIR"
}
trap cleanup EXIT

"$BIN" --engine stub --data-dir "$DATA_DIR" --port "$PORT" --no-open >"$DATA_DIR/server.log" 2>&1 &
SERVER_PID=$!

for _ in $(seq 1 50); do
  if curl -sf "http://127.0.0.1:$PORT/api/state" >/dev/null 2>&1; then break; fi
  sleep 0.2
done

echo "-- 首页可访问"
curl -sf "http://127.0.0.1:$PORT/" | grep -q btn-finish

echo "-- 送两段音频（第二段用 48kHz，验证重采样）"
python3 - "$DATA_DIR" <<'PY'
import struct, sys
rate = 16000
for name, ms, rate in (("seg1.pcm", 1200, 16000), ("seg2.pcm", 700, 48000)):
    samples = b"".join(struct.pack("<h", 9000 if (i // 40) % 2 == 0 else -9000)
                       for i in range(rate * ms // 1000))
    open(f"{sys.argv[1]}/{name}", "wb").write(samples)
PY
curl -sf -X POST --data-binary "@$DATA_DIR/seg1.pcm" "http://127.0.0.1:$PORT/api/test/segment?rate=16000" >/dev/null
curl -sf -X POST --data-binary "@$DATA_DIR/seg2.pcm" "http://127.0.0.1:$PORT/api/test/segment?rate=48000" >/dev/null

for _ in $(seq 1 60); do
  STATE="$(curl -sf "http://127.0.0.1:$PORT/api/state")"
  if [[ "$(echo "$STATE" | python3 -c 'import json,sys;print(json.load(sys.stdin)["pending_jobs"])')" == "0" ]] \
     && [[ "$(echo "$STATE" | python3 -c 'import json,sys;print(json.load(sys.stdin)["sentences"])')" -ge 2 ]]; then
    break
  fi
  sleep 0.2
done

echo "-- 状态：$STATE"
echo "$STATE" | python3 -c '
import json, sys
state = json.load(sys.stdin)
assert state["segments"] == 2, state
assert state["sentences"] >= 2, state
assert state["audio_version"] == 2, state
print("   段落/句子/音频版本：", state["segments"], state["sentences"], state["audio_version"])
'

echo "-- 保存"
curl -sf -X POST -H 'content-type: application/json' \
  -d '{"title":"冒烟测试"}' "http://127.0.0.1:$PORT/api/session/finalize" \
  | python3 -c '
import json, os, sys
files = json.load(sys.stdin)["files"]
assert len(files) == 4, files
for path in files:
    assert os.path.exists(path), path
print("   产物：")
for path in files:
    print("   ", os.path.basename(path), os.path.getsize(path), "字节")
'

echo "== 冒烟通过 =="
