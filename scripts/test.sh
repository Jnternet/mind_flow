#!/usr/bin/env bash
# 全部测试：格式检查、Rust（单元 + 集成 + 架构）、前端纯逻辑。
set -euo pipefail
cd "$(dirname "$0")/.."

echo "== cargo fmt --check =="
cargo fmt --check

echo "== cargo test =="
cargo test

echo "== node --test =="
node --test tests/js/*.test.js

echo "== 全部通过 =="
