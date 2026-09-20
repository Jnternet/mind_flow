#!/usr/bin/env bash
# 把 CUDA 引擎装到 <数据目录>/runtime/cuda/。
#
# 三种来源，按优先级：
#   1) 本地已构建的产物：dist/mind_flow-cuda-engine-*.tar.gz
#   2) 指定 URL：CUDA_ENGINE_URL=https://... bash scripts/fetch-cuda-engine.sh
#   3) GitHub Release 资产：CUDA_ENGINE_URL 未设时尝试从 $MIND_FLOW_REPO 下载
#
# CUDA/cuDNN 运行时（约 2–3GB）需要自备，本脚本不下载。
set -euo pipefail
cd "$(dirname "$0")/.."

DATA_DIR="${DATA_DIR:-./data}"
TARGET="$DATA_DIR/runtime/cuda"
REPO="${MIND_FLOW_REPO:-Jnternet/mind_flow}"
VERSION="$(sed -n 's/^version = "\(.*\)"/\1/p' Cargo.toml | head -1)"
PLATFORM="$(uname -s | tr '[:upper:]' '[:lower:]')"
mkdir -p "$TARGET"

install_from() {
  local archive="$1"
  echo "-- 解压 $archive → $TARGET"
  local tmp
  tmp="$(mktemp -d)"
  case "$archive" in
    *.zip) unzip -q "$archive" -d "$tmp" ;;
    *) tar -xf "$archive" -C "$tmp" ;;
  esac
  # 归档里可能有一层目录，统一摊平到 runtime/cuda
  find "$tmp" -type f \( -name 'mind_flow-engine*' -o -name '*.so*' -o -name '*.dll' \) \
    -exec cp -a {} "$TARGET/" \;
  rm -rf "$tmp"
  chmod +x "$TARGET"/mind_flow-engine* 2>/dev/null || true
}

LOCAL="$(ls dist/mind_flow-cuda-engine-*-"$PLATFORM"*.tar.gz 2>/dev/null | tail -1 || true)"
if [[ -n "${CUDA_ENGINE_URL:-}" ]]; then
  ARCHIVE="$(mktemp /tmp/mind_flow-cuda-engine.XXXXXX.tar.gz)"
  echo "-- 下载 $CUDA_ENGINE_URL"
  curl -sSL --retry 5 --retry-delay 3 --retry-all-errors -o "$ARCHIVE" "$CUDA_ENGINE_URL"
  install_from "$ARCHIVE"
  rm -f "$ARCHIVE"
elif [[ -n "$LOCAL" ]]; then
  install_from "$LOCAL"
else
  echo "-- 本地没有构建产物，尝试从 GitHub Release 取"
  NAME="mind_flow-cuda-engine-v$VERSION-${PLATFORM}-x64-cuda12.tar.gz"
  URL="https://github.com/$REPO/releases/latest/download/$NAME"
  ARCHIVE="$(mktemp /tmp/mind_flow-cuda-engine.XXXXXX.tar.gz)"
  if curl -sSL --retry 3 --retry-all-errors -f -o "$ARCHIVE" "$URL"; then
    install_from "$ARCHIVE"
  else
    rm -f "$ARCHIVE"
    cat >&2 <<EOF
没取到 CUDA 引擎。可选做法：
  1) 自己构建：bash scripts/fetch-vendor.sh --cuda12 && bash scripts/build-release.sh --cuda=12
  2) 指定来源：CUDA_ENGINE_URL=<url> bash scripts/fetch-cuda-engine.sh
  3) 手动把 mind_flow-engine 与其 .so/.dll 放进 $TARGET
EOF
    exit 1
  fi
  rm -f "$ARCHIVE"
fi

echo "== 安装完成：$TARGET =="
ls -la "$TARGET"
cat <<'EOF'

提示：还需要本机有 NVIDIA 驱动 + CUDA 12.x/13.x + cuDNN 9 运行时；
     启动后界面状态条会显示当前推理设备，失败原因也会写在设置里。
EOF
