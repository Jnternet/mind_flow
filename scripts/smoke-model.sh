#!/usr/bin/env bash
# 真实模型冒烟：用真引擎识别一段中文音频，检查文本、时间戳与耗时。
#
# 需要：
#   1) 带 sherpa 特性的二进制（bash scripts/build.sh --release）
#   2) 模型目录（默认 vendor/models，可用 MODEL_DIR 指定；目录布局见 src/models.rs）
#
# 用法：bash scripts/smoke-model.sh [二进制路径] [wav 路径]
set -euo pipefail
cd "$(dirname "$0")/.."

BIN="${1:-target/release/mind_flow}"
MODEL_DIR="${MODEL_DIR:-vendor/models}"
WAV="${2:-$MODEL_DIR/paraformer-zh-2023-09-14-int8/0.wav}"
PORT="${SMOKE_PORT:-8802}"
DATA_DIR="$(mktemp -d /tmp/mind_flow_model.XXXXXX)"

if [[ ! -x "$BIN" ]]; then
  echo "找不到二进制 $BIN，先运行 scripts/build.sh --release" >&2
  exit 1
fi
if [[ ! -f "$MODEL_DIR/paraformer-zh-2023-09-14-int8/model.int8.onnx" ]]; then
  echo "模型目录布局不对：$MODEL_DIR" >&2
  exit 1
fi
if [[ ! -f "$WAV" ]]; then
  echo "找不到测试音频：$WAV" >&2
  exit 1
fi

cleanup() {
  if [[ -n "${SERVER_PID:-}" ]]; then kill "$SERVER_PID" 2>/dev/null || true; fi
  rm -rf "$DATA_DIR"
}
trap cleanup EXIT

"$BIN" --data-dir "$DATA_DIR" --model-dir "$MODEL_DIR" --port "$PORT" --no-open --test-api \
  >"$DATA_DIR/server.log" 2>&1 &
SERVER_PID=$!

for _ in $(seq 1 100); do
  if curl -sf "http://127.0.0.1:$PORT/api/state" >/dev/null 2>&1; then break; fi
  sleep 0.2
done

echo "-- 送入真实语音：$(basename "$WAV")"
START=$(date +%s.%N)
curl -sf -X POST --data-binary "@$WAV" "http://127.0.0.1:$PORT/api/test/segment?rate=16000" >/dev/null

for _ in $(seq 1 200); do
  STATE="$(curl -sf "http://127.0.0.1:$PORT/api/state")"
  PENDING="$(echo "$STATE" | python3 -c 'import json,sys;print(json.load(sys.stdin)["pending_jobs"])')"
  SENTENCES="$(echo "$STATE" | python3 -c 'import json,sys;print(json.load(sys.stdin)["sentences"])')"
  if [[ "$PENDING" == "0" && "$SENTENCES" -ge 1 ]]; then break; fi
  sleep 0.2
done
ELAPSED=$(echo "$(date +%s.%N) - $START" | bc)

python3 - "$DATA_DIR" "$MODEL_DIR" "$WAV" "$ELAPSED" <<'PY'
import json, sys, wave, os, glob
data_dir, model_dir, wav_path, elapsed = sys.argv[1], sys.argv[2], sys.argv[3], float(sys.argv[4])
doc = json.load(open(glob.glob(os.path.join(data_dir, "sessions", "*", "session.json"))[0]))
sentences = doc["sentences"]
with wave.open(wav_path) as handle:
    audio_seconds = handle.getnframes() / handle.getframerate()
assert sentences, "真实引擎没有识别出任何句子"
text = "".join(s["text"] for s in sentences)
assert len(text.strip()) >= 4, f"识别文本太短：{text!r}"
previous = -1
for sentence in sentences:
    assert sentence["start_ms"] >= 0 and sentence["end_ms"] > sentence["start_ms"], sentence
    assert sentence["start_ms"] >= previous, f"时间戳应单调：{sentences}"
    previous = sentence["start_ms"]
assert sentences[-1]["end_ms"] <= doc["duration_ms"] + 500, "末句不应超出音频时长"
print(f"   识别结果：{text}")
for sentence in sentences:
    print(f"   [{sentence['start_ms']:>6}ms → {sentence['end_ms']:>6}ms] {sentence['text']}")
print(f"   引擎：{doc['engine']['name']} {doc['engine']['model']} · {doc['engine']['device']}"
      f" · 标点={doc['engine']['punctuation']} · VAD={doc['engine']['vad']}")
print(f"   音频 {audio_seconds:.2f}s，端到端 {elapsed:.2f}s，RTF≈{elapsed / audio_seconds:.3f}")
print(f"   句子数：{len(sentences)}，首句时间戳非零：{sentences[0]['start_ms'] > 0}")
PY

echo "== 真实模型冒烟通过 =="
