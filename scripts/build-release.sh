#!/usr/bin/env bash
# 打发布产物到 dist/：
#   Linux  x86_64 静态链接（单文件，解压即用）
#   Windows x86_64 交叉编译（共享库方式，包里带 DLL）
#   CUDA 引擎（可选）：--cuda=12|13，需要先 scripts/fetch-vendor.sh --cuda12
#
# 只做本机能验证的产物；某个平台缺预编译库时跳过并给出提示。
set -euo pipefail
cd "$(dirname "$0")/.."

VERSION="$(sed -n 's/^version = "\(.*\)"/\1/p' Cargo.toml | head -1)"
OUT="dist"
SHERPA_VERSION="${SHERPA_VERSION:-1.13.8}"
CUDA=""
for arg in "$@"; do
  case "$arg" in
    --cuda=12) CUDA="12" ;;
    --cuda=13) CUDA="13" ;;
    -h|--help) sed -n '2,8p' "$0"; exit 0 ;;
    *) echo "未知参数：$arg" >&2; exit 2 ;;
  esac
done

mkdir -p "$OUT"
echo "== mind_flow v$VERSION =="

# ---------------------------------------------------------------- Linux
LINUX_LIB="$PWD/vendor/sherpa-onnx/sherpa-onnx-v${SHERPA_VERSION}-linux-x64-static-lib/lib"
if [[ -d "$LINUX_LIB" ]]; then
  echo "-- Linux x86_64（静态链接，单文件）"
  SHERPA_ONNX_LIB_DIR="$LINUX_LIB" cargo build --release --locked \
    --target x86_64-unknown-linux-gnu --features sherpa
  PKG="mind_flow-v$VERSION-x86_64-unknown-linux-gnu"
  rm -rf "$OUT/$PKG" && mkdir -p "$OUT/$PKG"
  cp target/x86_64-unknown-linux-gnu/release/mind_flow "$OUT/$PKG/"
  cp README.md DESIGN.md "$OUT/$PKG/"
  tar -C "$OUT" -czf "$OUT/$PKG.tar.gz" "$PKG"
  rm -rf "$OUT/$PKG"
else
  echo "-- 跳过 Linux：缺少 $LINUX_LIB（先 scripts/fetch-vendor.sh）"
fi

# ---------------------------------------------------------------- Windows
WIN_LIB="${SHERPA_WIN_LIB_DIR:-}"
if [[ -z "$WIN_LIB" ]]; then
  for candidate in "$PWD/vendor/sherpa-onnx/sherpa-onnx-v${SHERPA_VERSION}-win-x64-shared-MD-Release-lib/lib" \
                   "$PWD/vendor/sherpa-onnx/sherpa-onnx-v${SHERPA_VERSION}-win-x64-shared-MD-Release/lib"; do
    [[ -d "$candidate" ]] && WIN_LIB="$candidate" && break
  done
fi
if [[ -n "$WIN_LIB" && -d "$WIN_LIB" ]]; then
  if command -v cargo-zigbuild >/dev/null 2>&1 && command -v zig >/dev/null 2>&1; then
    echo "-- Windows x86_64（共享库，包里带 DLL）"
    SHERPA_ONNX_LIB_DIR="$WIN_LIB" cargo zigbuild --release --locked \
      --target x86_64-pc-windows-gnu --features sherpa-shared
    PKG="mind_flow-v$VERSION-x86_64-pc-windows-gnu"
    rm -rf "$OUT/$PKG" && mkdir -p "$OUT/$PKG"
    cp target/x86_64-pc-windows-gnu/release/mind_flow.exe "$OUT/$PKG/"
    # 运行期需要的 DLL 与 exe 放在同一层（Windows 会先找 exe 同级目录）
    find "$WIN_LIB" -maxdepth 2 -name '*.dll' -exec cp {} "$OUT/$PKG/" \;
    cp README.md "$OUT/$PKG/"
    (cd "$OUT" && zip -qr "$PKG.zip" "$PKG")
    rm -rf "$OUT/$PKG"
  else
    echo "-- 跳过 Windows：缺少 cargo-zigbuild / zig"
  fi
else
  echo "-- 跳过 Windows：缺少预编译库（设置 SHERPA_WIN_LIB_DIR 指向 win-x64 的 lib 目录）"
fi

# ---------------------------------------------------------------- CUDA 引擎
if [[ -n "$CUDA" ]]; then
  CUDA_LIB="$PWD/vendor/sherpa-onnx/sherpa-onnx-v${SHERPA_VERSION}-cuda-${CUDA}.x-cudnn-9.x-onnxruntime1.28.2-linux-x64-gpu/lib"
  if [[ -d "$CUDA_LIB" ]]; then
    echo "-- CUDA $CUDA.x 引擎（Linux）"
    SHERPA_ONNX_LIB_DIR="$CUDA_LIB" cargo build --release --locked \
      --target x86_64-unknown-linux-gnu --features sherpa-cuda --bin mind_flow-engine
    PKG="mind_flow-cuda-engine-v$VERSION-linux-x64-cuda$CUDA"
    rm -rf "$OUT/$PKG" && mkdir -p "$OUT/$PKG"
    cp target/x86_64-unknown-linux-gnu/release/mind_flow-engine "$OUT/$PKG/"
    find "$CUDA_LIB" -maxdepth 1 -name '*.so*' -exec cp -a {} "$OUT/$PKG/" \;
    cat > "$OUT/$PKG/README.md" <<EOF
# mind_flow CUDA 引擎（CUDA $CUDA.x + cuDNN 9）

解压到 \`<数据目录>/runtime/cuda/\`（默认就是 mind_flow 同级的 \`data/runtime/cuda/\`），
重启 mind_flow 后界面状态条会显示 CUDA 设备；探测失败会自动回落 CPU 并提示原因。

需要本机已装 NVIDIA 驱动与 CUDA $CUDA.x / cuDNN 9 运行时（不随本包提供）。
EOF
    tar -C "$OUT" -czf "$OUT/$PKG.tar.gz" "$PKG"
    rm -rf "$OUT/$PKG"
  else
    echo "-- 跳过 CUDA 引擎：缺少 $CUDA_LIB（先 scripts/fetch-vendor.sh --cuda$CUDA）"
  fi
fi

echo "-- 校验和"
if compgen -G "$OUT/mind_flow-*" >/dev/null; then
  (cd "$OUT" && sha256sum mind_flow-* > "SHA256SUMS-$VERSION.txt")
fi
ls -lh "$OUT" | head -20
