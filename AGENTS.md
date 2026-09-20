# 协作约定

## Git 与发布

- 小步提交，一次提交只做一件事；提交信息用 `类型: 中文简述`，类型取 `feat` / `fix` /
  `docs` / `refactor` / `test` / `chore`。
- **不要自动推送、不要自动打标签或发版本**：只有明确说「推送」「发布版本」时才做。
- 不要把构建产物提交进仓库（`target/`、`dist/`、`data/` 已在 `.gitignore`）。

## 提交前必须通过

```bash
cargo fmt --check
cargo test                        # 单元 / 集成 / 架构（集成测试会绑本机端口，沙箱里需提权）
node --test tests/js/*.test.js    # 协议与前端纯逻辑
```

涉及界面/端到端时再加跑：

- `bash scripts/smoke.sh`：stub 引擎跑完整流程（录音 → 识别 → 导出四个文件）。
- `node scripts/e2e-browser.mjs`：真 Firefox 里验证渲染、点时间戳跳转、播放器加载（需要 firefox）。
- `bash scripts/smoke-model.sh`：真实模型识别中文音频并打印 RTF（需要 `vendor/models` 或 `data/models`）。
- `bash scripts/build-release.sh`：打 Linux 产物；Windows 需要 Windows 预编译库 + zig。

真引擎的构建要用 `scripts/build.sh`（它会设好 `SHERPA_ONNX_LIB_DIR`）。首次需要
`bash scripts/fetch-vendor.sh` 取预编译库（本机 github.com 不通时加 `--api`）。

## 代码约定

- edition 2024；用户可见文案与代码注释都用中文。
- 协议与纯逻辑放 `web/lib/`（可在 Node 里跑测试），`web/app.js` 只做接线与渲染。
- 识别引擎在独立 OS 线程里，任何时候都不要在 async 任务里直接跑推理。
- 运行时不得访问网络，唯一例外是模型/CUDA 引擎下载（`src/models/`、`src/gpu/`）。
- 除数据目录外不写任何文件；数据目录默认程序同级 `data/`，用 `--data-dir` 覆盖。
- 改动行为前先改 `DESIGN.md`（需求、决策、接口的唯一事实来源）。

## 本机环境

- 本机（Linux 容器）**没有 GPU**：CUDA 路径只能验证「探测失败 → 回落 CPU」，
  真机验证用 `scripts/verify-gpu.sh` 并把结果回填 `DESIGN.md`。
- 构建需要联网（cargo 依赖 + sherpa-onnx 预编译库下载）；运行时不需要。
