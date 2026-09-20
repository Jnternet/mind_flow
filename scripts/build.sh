#!/usr/bin/env bash
# 构建带真实识别引擎的二进制。
#
# 需要：
#   vendor/sherpa-onnx/sherpa-onnx-<版本>-<平台>-<模式>/lib
# 没有的话先用 scripts/fetch-vendor.sh 拉下来（本机网络不通时走 GitHub API）。
set -euo pipefail
cd "$(dirname "$0")/.."

VERSION="1.13.8"
LIB_ROOT="${SHERPA_ONNX_LIB_DIR:-$PWD/vendor/sherpa-onnx/sherpa-onnx-v${VERSION}-linux-x64-static-lib/lib}"
if [[ ! -d "$LIB_ROOT" ]]; then
  echo "找不到 sherpa-onnx 静态库：$LIB_ROOT" >&2
  echo "先运行 scripts/fetch-vendor.sh，或用 SHERPA_ONNX_LIB_DIR 指定" >&2
  exit 1
fi

export SHERPA_ONNX_LIB_DIR="$LIB_ROOT"
exec cargo build "$@" --features sherpa
