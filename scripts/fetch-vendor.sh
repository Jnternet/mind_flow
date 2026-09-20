#!/usr/bin/env bash
# 拉取 sherpa-onnx 预编译库（构建期依赖，不进仓库）。
#
# 默认走 GitHub release；本机 github.com 不通时用 --api 走 api.github.com 的资产接口。
# 也可以设置 SHERPA_ONNX_MIRROR 指定镜像前缀。
set -euo pipefail
cd "$(dirname "$0")/.."

VERSION="1.13.8"
PLATFORM="linux-x64-static-lib"
MODE="direct"
for arg in "$@"; do
  case "$arg" in
    --api) MODE="api" ;;
    --cuda12) PLATFORM="cuda-12.x-cudnn-9.x-onnxruntime1.28.2-linux-x64-gpu" ;;
    --cuda13) PLATFORM="cuda-13.x-cudnn-9.x-onnxruntime1.28.2-linux-x64-gpu" ;;
    -h|--help)
      sed -n '2,8p' "$0"
      exit 0
      ;;
    *) echo "未知参数：$arg" >&2; exit 2 ;;
  esac
done

ARCHIVE="sherpa-onnx-v${VERSION}-${PLATFORM}.tar.bz2"
DIR="vendor/sherpa-onnx"
mkdir -p "$DIR"
cd "$DIR"

if [[ -d "sherpa-onnx-v${VERSION}-${PLATFORM}/lib" ]]; then
  echo "已存在：sherpa-onnx-v${VERSION}-${PLATFORM}/lib"
  exit 0
fi

if [[ ! -f "$ARCHIVE" ]]; then
  if [[ "$MODE" == "api" ]]; then
    echo "-- 通过 GitHub API 取 $ARCHIVE"
    ASSET_ID="$(curl -sSL -H 'Accept: application/vnd.github+json' \
      "https://api.github.com/repos/k2-fsa/sherpa-onnx/releases/tags/v${VERSION}" \
      | python3 -c "
import json,sys
name = sys.argv[1]
release = json.load(sys.stdin)
for asset in release.get('assets', []):
    if asset['name'] == name:
        print(asset['id'])
        break
" "$ARCHIVE")"
    if [[ -z "$ASSET_ID" ]]; then
      echo "release 里没有 $ARCHIVE" >&2
      exit 1
    fi
    curl -sSL --retry 8 --retry-delay 3 --retry-all-errors -C - \
      -H 'Accept: application/octet-stream' \
      -o "$ARCHIVE" \
      "https://api.github.com/repos/k2-fsa/sherpa-onnx/releases/assets/$ASSET_ID"
  else
    echo "-- 直接下载 $ARCHIVE"
    curl -sSL --retry 5 --retry-delay 3 --retry-all-errors -C - -o "$ARCHIVE" \
      "${SHERPA_ONNX_MIRROR:-https://github.com/k2-fsa/sherpa-onnx/releases/download}/v${VERSION}/$ARCHIVE"
  fi
fi

echo "-- 解包 $ARCHIVE"
tar xjf "$ARCHIVE"
ls -d "sherpa-onnx-v${VERSION}-${PLATFORM}"/lib
