#!/usr/bin/env bash
# 验证某个离线包「解压即用」：解压 → 启动（不联网）→ 用包内示例语音跑真实识别。
#
#   bash scripts/smoke-bundle.sh dist/mind_flow-...-offline.tar.gz
#   bash scripts/smoke-bundle.sh dist/mind_flow-...-offline.zip           # Windows 包只做内容检查
#   bash scripts/smoke-bundle.sh dist/...-offline.tar.gz --read-only      # 解压到只读目录，验证回退后仍能找到随包模型
set -euo pipefail
cd "$(dirname "$0")/.."

ARCHIVE="${1:?用法: bash scripts/smoke-bundle.sh <离线包> [--read-only]}"
MODE="${2:-}"
PORT="${SMOKE_PORT:-8890}"
WORKDIR="$(mktemp -d /tmp/mind_flow_bundle.XXXXXX)"
FAKE_HOME="$(mktemp -d /tmp/mind_flow_home.XXXXXX)"
SERVER_PID=""

cleanup() {
  [[ -n "$SERVER_PID" ]] && kill "$SERVER_PID" 2>/dev/null || true
  chmod -R u+w "$WORKDIR" 2>/dev/null || true
  rm -rf "$WORKDIR" "$FAKE_HOME"
}
trap cleanup EXIT

echo "== 解包 $(basename "$ARCHIVE") =="
case "$ARCHIVE" in
  *.zip)
    unzip -q "$ARCHIVE" -d "$WORKDIR"
    ROOT="$(find "$WORKDIR" -maxdepth 1 -mindepth 1 -type d | head -1)"
    echo "-- Windows 包：只检查内容是否齐全（本机跑不了 exe）"
    for f in mind_flow.exe sherpa-onnx-c-api.dll onnxruntime.dll; do
      [[ -f "$ROOT/$f" ]] || { echo "缺少 $f" >&2; exit 1; }
    done
    for rel in \
      data/models/paraformer-zh-2023-09-14-int8/model.int8.onnx \
      data/models/paraformer-zh-2023-09-14-int8/tokens.txt \
      data/models/punct-ct-transformer-zh-en-vocab272727-2024-04-12/model.int8.onnx \
      data/models/silero_vad.onnx; do
      [[ -f "$ROOT/$rel" ]] || { echo "缺少模型 $rel" >&2; exit 1; }
    done
    echo "-- 内容检查通过（模型与运行库齐全）"
    exit 0
    ;;
  *)
    tar xzf "$ARCHIVE" -C "$WORKDIR"
    ROOT="$(find "$WORKDIR" -maxdepth 1 -mindepth 1 -type d | head -1)"
    ;;
esac

BIN="$ROOT/mind_flow"
[[ -x "$BIN" ]] || { echo "包内没有可执行文件：$BIN" >&2; exit 1; }
[[ -f "$ROOT/data/models/paraformer-zh-2023-09-14-int8/model.int8.onnx" ]] \
  || { echo "包里没有内置模型" >&2; exit 1; }

if [[ "$MODE" == "--read-only" ]]; then
  echo "-- 把解压目录设为只读，验证「数据目录回退后仍能找到随包模型」"
  chmod -R a-w "$ROOT"
fi

echo "== 启动（HOME=$FAKE_HOME，避免污染真实用户目录）=="
HOME="$FAKE_HOME" "$BIN" --port "$PORT" --no-open --test-api >"$WORKDIR/run.log" 2>&1 &
SERVER_PID=$!

for _ in $(seq 1 120); do
  STATE="$(curl -sf "http://127.0.0.1:$PORT/api/state" 2>/dev/null || true)"
  if [[ -n "$STATE" ]] && echo "$STATE" | grep -q '"model_ready":true'; then break; fi
  sleep 0.5
done

echo "== 诊断 =="
curl -sf "http://127.0.0.1:$PORT/api/diagnostics" | python3 -c '
import json, sys
diagnostics = json.load(sys.stdin)
models = diagnostics["models"]
print("  程序路径:", diagnostics["exe_path"])
print("  数据目录:", diagnostics["data_dir"], "(程序同级)" if diagnostics["data_dir_is_portable"] else "(回退到用户目录)")
print("  模型目录:", diagnostics["model_dir"])
print("  模型就绪:", models["ready"], "| 标点/VAD:", models["extras_ready"])
print("  结论:", models["hint"])
assert models["ready"], "模型没就绪"
assert models["extras_ready"], "标点或 VAD 没加载"
'

echo "== 用包内示例语音跑真实识别 =="
SAMPLE="$ROOT/示例语音_0.wav"
[[ -f "$SAMPLE" ]] || SAMPLE="$(find "$ROOT" -name '*.wav' | head -1)"
curl -sf -X POST --data-binary "@$SAMPLE" "http://127.0.0.1:$PORT/api/test/segment?rate=16000" >/dev/null

for _ in $(seq 1 60); do
  SESSION="$(curl -sf "http://127.0.0.1:$PORT/api/session" 2>/dev/null || true)"
  if [[ -n "$SESSION" ]] && echo "$SESSION" | grep -q '"sentences":\[{'; then break; fi
  sleep 0.5
done

curl -sf "http://127.0.0.1:$PORT/api/session" | python3 -c '
import json, sys
session = json.load(sys.stdin)
sentences = session["sentences"]
assert sentences, "没有识别出任何句子"
text = "".join(s["text"] for s in sentences)
assert any("\u4e00" <= ch <= "\u9fff" for ch in text), f"识别结果里没有中文：{text!r}"
for sentence in sentences:
    assert sentence["end_ms"] > sentence["start_ms"], sentence
    start, end, text = sentence["start_ms"], sentence["end_ms"], sentence["text"]
    print(f"  [{start:>5}ms → {end:>5}ms] {text}")
print(f"  共 {len(sentences)} 句，识别文本 {len(text)} 字")
'

echo "== 日志里不应有下载动作 =="
if grep -qE '模型不完整|下载' "$WORKDIR/run.log"; then
  echo "!! 日志里出现了下载/缺模型的痕迹：" >&2
  grep -E '模型不完整|下载' "$WORKDIR/run.log" >&2
  exit 1
fi
echo "  没有下载动作"
echo "== 离线包验收通过 =="
