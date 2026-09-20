#!/usr/bin/env bash
# 打发布产物到 dist/：
#   Linux  x86_64 静态链接（单文件，解压即用）
#   Windows x86_64 交叉编译（共享库方式，包里带 DLL；优先 MT 版＝不需要 VC++ 运行库）
#   CUDA 引擎（可选）：--cuda=12|13，需要先 scripts/fetch-vendor.sh --cuda12
#   离线包（可选）：--with-models，把约 320MB 模型一起装进包里，解压即用、无需联网下载
#
# 只做本机能验证的产物；某个平台缺预编译库时跳过并给出提示。
set -euo pipefail
cd "$(dirname "$0")/.."

VERSION="$(sed -n 's/^version = "\(.*\)"/\1/p' Cargo.toml | head -1)"
OUT="dist"
SHERPA_VERSION="${SHERPA_VERSION:-1.13.8}"
CUDA=""
WITH_MODELS=""
for arg in "$@"; do
  case "$arg" in
    --cuda=12) CUDA="12" ;;
    --cuda=13) CUDA="13" ;;
    --with-models) WITH_MODELS="1" ;;
    -h|--help) sed -n '2,8p' "$0"; exit 0 ;;
    *) echo "未知参数：$arg" >&2; exit 2 ;;
  esac
done

mkdir -p "$OUT"
echo "== mind_flow v$VERSION =="

# 把内置模型（含 16k 示例音频）塞进包里的 <包>/data/ 下。只拷程序真正用到的文件。
pack_models() {
  local pkg_dir="$1"
  local models="$PWD/vendor/models"
  local asr="$models/paraformer-zh-2023-09-14-int8"
  local punct="$models/punct-ct-transformer-zh-en-vocab272727-2024-04-12"
  local target="$pkg_dir/data/models"
  [[ -n "$WITH_MODELS" ]] || return 0
  if [[ ! -f "$asr/model.int8.onnx" || ! -f "$asr/tokens.txt" || ! -f "$punct/model.int8.onnx" ]]; then
    echo "!! 本地模型不全，无法打离线包（先 scripts/prepare-vendor-models.sh）" >&2
    return 1
  fi
  echo "-- 内置模型（约 320MB）"
  mkdir -p "$target/paraformer-zh-2023-09-14-int8" "$target/punct-ct-transformer-zh-en-vocab272727-2024-04-12"
  cp "$asr/model.int8.onnx" "$asr/tokens.txt" "$target/paraformer-zh-2023-09-14-int8/"
  cp "$punct/model.int8.onnx" "$target/punct-ct-transformer-zh-en-vocab272727-2024-04-12/"
  [[ -f "$models/silero_vad.onnx" ]] && cp "$models/silero_vad.onnx" "$target/"
  [[ -f "$asr/0.wav" ]] && cp "$asr/0.wav" "$pkg_dir/示例语音_0.wav"
  cat > "$target/来源与校验.txt" <<EOF
这些模型已随包提供，程序启动时不会联网下载。若想自行核对，SHA-256 如下：

f36a0433bcf096bd6d6f11b80a3ac8bed110bdca632fe0d731df8d1a84475945  paraformer-zh-2023-09-14-int8/model.int8.onnx
59aba8873a2ed1e122c25fee421e25f283b63290efbde85c1f01a853d83cb6e6  paraformer-zh-2023-09-14-int8/tokens.txt
65a3fb9f5ad7bfb96bf69e0dc4481df97f6ee60513c1d94ce981ba6effd524b1  punct-ct-transformer-zh-en-vocab272727-2024-04-12/model.int8.onnx
9e2449e1087496d8d4caba907f23e0bd3f78d91fa552479bb9c23ac09cbb1fd6  silero_vad.onnx

来源：
  识别模型  https://hf-mirror.com/csukuangfj/sherpa-onnx-paraformer-zh-2023-09-14
  标点模型  https://github.com/k2-fsa/sherpa-onnx/releases/tag/punctuation-models
  VAD 模型  https://github.com/k2-fsa/sherpa-onnx/releases/tag/asr-models
算法来自 FunASR/Paraformer，经 sherpa-onnx 导出为 ONNX。
EOF
}

# ---------------------------------------------------------------- Linux
LINUX_LIB="$PWD/vendor/sherpa-onnx/sherpa-onnx-v${SHERPA_VERSION}-linux-x64-static-lib/lib"
if [[ -d "$LINUX_LIB" ]]; then
  echo "-- Linux x86_64（静态链接，单文件）"
  SHERPA_ONNX_LIB_DIR="$LINUX_LIB" cargo build --release --locked \
    --target x86_64-unknown-linux-gnu --features sherpa
  PKG="mind_flow-v$VERSION-x86_64-unknown-linux-gnu"
  rm -rf "$OUT/$PKG" && mkdir -p "$OUT/$PKG"
  cp target/x86_64-unknown-linux-gnu/release/mind_flow "$OUT/$PKG/"
  cp README.md DESIGN.md docs/测试说明.md "$OUT/$PKG/"
  mkdir -p "$OUT/$PKG/docs" && mv "$OUT/$PKG/测试说明.md" "$OUT/$PKG/docs/"
  pack_models "$OUT/$PKG"
  if [[ -n "$WITH_MODELS" ]]; then
    PKG="mind_flow-v$VERSION-x86_64-unknown-linux-gnu-offline"
    mv "$OUT/mind_flow-v$VERSION-x86_64-unknown-linux-gnu" "$OUT/$PKG"
  fi
  tar -C "$OUT" -czf "$OUT/$PKG.tar.gz" "$PKG"
  rm -rf "$OUT/$PKG"
else
  echo "-- 跳过 Linux：缺少 $LINUX_LIB（先 scripts/fetch-vendor.sh）"
fi

# ---------------------------------------------------------------- Windows
WIN_LIB="${SHERPA_WIN_LIB_DIR:-}"
if [[ -z "$WIN_LIB" ]]; then
  for candidate in \
    "$PWD/vendor/sherpa-onnx-win/sherpa-onnx-v${SHERPA_VERSION}-win-x64-shared-MT-Release/lib" \
    "$PWD/vendor/sherpa-onnx-win/sherpa-onnx-v${SHERPA_VERSION}-win-x64-shared-MD-Release/lib" \
    "$PWD/vendor/sherpa-onnx/sherpa-onnx-v${SHERPA_VERSION}-win-x64-shared-MT-Release-lib/lib" \
    "$PWD/vendor/sherpa-onnx/sherpa-onnx-v${SHERPA_VERSION}-win-x64-shared-MD-Release-lib/lib"; do
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
    # 可选的 GPU 引擎：先放在包里，要测 GPU 时自己挪到 data/runtime/cuda/
    cp target/x86_64-pc-windows-gnu/release/mind_flow-engine.exe "$OUT/$PKG/" 2>/dev/null || true
    # 运行期需要的 DLL 与 exe 放在同一层（Windows 会先找 exe 同级目录）
    find "$WIN_LIB" -maxdepth 2 -name '*.dll' -exec cp {} "$OUT/$PKG/" \;
    cp README.md "$OUT/$PKG/"
    mkdir -p "$OUT/$PKG/docs" && cp docs/测试说明.md "$OUT/$PKG/docs/"
    pack_models "$OUT/$PKG"
    if [[ -n "$WITH_MODELS" ]]; then
      PKG="mind_flow-v$VERSION-x86_64-pc-windows-gnu-offline"
      mv "$OUT/mind_flow-v$VERSION-x86_64-pc-windows-gnu" "$OUT/$PKG"
    fi
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
