#!/usr/bin/env bash
# 把下载好的模型归档整理成程序期望的目录布局（vendor/models/...），供本地测试用。
#
# 程序运行时用的布局（也是 data/models 的结构）：
#   models/paraformer-zh-2023-09-14-int8/{model.int8.onnx,tokens.txt}
#   models/punct-ct-transformer-zh-en-vocab272727-2024-04-12/{model.int8.onnx|model.onnx}
#   models/silero_vad.onnx
set -euo pipefail
cd "$(dirname "$0")/.."

MODELS="${MODELS_DIR:-vendor/models}"
ASR_DIR="$MODELS/paraformer-zh-2023-09-14-int8"
PUNCT_DIR="$MODELS/punct-ct-transformer-zh-en-vocab272727-2024-04-12"
mkdir -p "$ASR_DIR" "$PUNCT_DIR"

if [[ -f "$MODELS/paraformer-zh-2023-09-14-int8.tar.bz2" ]]; then
  tar xjf "$MODELS/paraformer-zh-2023-09-14.tar.bz2" -C "$MODELS" 2>/dev/null || true
fi

# 归档解出来的目录名与程序期望的不一致，这里统一成期望布局
for candidate in \
  "$MODELS"/sherpa-onnx-paraformer-zh-2023-09-14/*.onnx \
  "$MODELS"/sherpa-onnx-paraformer-zh-2023-09-14/tokens.txt; do
  [[ -f "$candidate" ]] || continue
  cp -f "$candidate" "$ASR_DIR/"
done

for candidate in \
  "$MODELS"/sherpa-onnx-punct-ct-transformer-zh-en-vocab272727-2024-04-12-int8/model.int8.onnx \
  "$MODELS"/punct-fp32-model.onnx; do
  [[ -f "$candidate" ]] || continue
  target="$PUNCT_DIR/$(basename "$candidate")"
  [[ "$(basename "$candidate")" == "punct-fp32-model.onnx" ]] && target="$PUNCT_DIR/model.onnx"
  cp -f "$candidate" "$target"
done

echo "布局完成："
find "$MODELS" -maxdepth 2 -name '*.onnx' -o -maxdepth 2 -name 'tokens.txt' | sort | sed 's/^/  /'
