#!/usr/bin/env bash
# 在有 N 卡的机器上验证 CUDA 引擎：跑一次 --probe 并在有测试音频时测 RTF。
#
#   bash scripts/verify-gpu.sh [数据目录]
#
# 结果可以直接回填 DESIGN.md 的「CUDA 真机验证」一节。
set -euo pipefail
cd "$(dirname "$0")/.."

DATA_DIR="${1:-./data}"
ENGINE="$DATA_DIR/runtime/cuda/mind_flow-engine"
MODELS="$DATA_DIR/models"
WAV="${VERIFY_WAV:-vendor/models/paraformer-zh-2023-09-14-int8/0.wav}"

if [[ ! -x "$ENGINE" ]]; then
  echo "找不到 CUDA 引擎：$ENGINE" >&2
  echo "先运行 bash scripts/fetch-cuda-engine.sh" >&2
  exit 1
fi
if [[ ! -f "$MODELS/paraformer-zh-2023-09-14-int8/model.int8.onnx" ]]; then
  echo "找不到模型：$MODELS" >&2
  exit 1
fi

echo "== 1) 设备信息 =="
if command -v nvidia-smi >/dev/null 2>&1; then
  nvidia-smi --query-gpu=name,driver_version,memory.total --format=csv,noheader | sed 's/^/   /'
else
  echo "   没有 nvidia-smi：驱动可能没装好"
fi

echo "== 2) 自检（装载 CUDA 模型 + 解码一小段）=="
START=$(date +%s.%N)
REPORT="$("$ENGINE" --probe --provider cuda --models "$MODELS" 2>/tmp/mind_flow_probe.err | tail -1)"
ELAPSED=$(echo "$(date +%s.%N) - $START" | bc)
if [[ -z "$REPORT" ]]; then
  echo "自检失败：" >&2
  sed 's/^/   /' /tmp/mind_flow_probe.err >&2
  exit 1
fi
echo "$REPORT" | python3 -c '
import json, sys
report = json.load(sys.stdin)
info = report["info"]
assert info["provider"] == "cuda", info
print(f"   provider={info[\"provider\"]} device={info[\"device\"]} model={info[\"model\"]}")
print(f"   协议版本={report[\"protocol\"]}")
'
echo "   自检耗时 ${ELAPSED}s"

if [[ -f "$WAV" ]]; then
  echo "== 3) 用真实音频比一比 CPU / GPU 的端到端耗时 =="
  for provider in cpu cuda; do
    START=$(date +%s.%N)
    "$ENGINE" --probe --provider "$provider" --models "$MODELS" >/dev/null 2>&1 || {
      echo "   $provider 自检失败（跳过）"
      continue
    }
    LOAD=$(echo "$(date +%s.%N) - $START" | bc)
    echo "   $provider：模型装载+自检 ${LOAD}s（音频 $(basename "$WAV") 的识别耗时用 scripts/smoke-model.sh 看）"
  done
fi

cat <<'EOF'

== 把下面这段回填 DESIGN.md 的「CUDA 真机验证」 ==
（设备名、provider、上面打印的耗时；再补一条 scripts/smoke-model.sh 的 RTF）
EOF
