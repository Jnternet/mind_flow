# mind_flow

本地语音笔记：按住空格说一句，松开就变成文字；每句话都带语音时间戳，点一下就能跳回去听。
识别全部在本机完成（FunASR 导出的 Paraformer-zh 模型，经 sherpa-onnx 推理），不联网、不上传。

## 特点

- 按住空格说话，松开后立刻出文字；说完可以马上说下一段，识别在后台排队。
- 每句话对应一个时间戳，点击即跳到该句回放位置（录制中也能回放已录好的部分）。
- 文字可以直接改，改完立即保存。
- 每段结束就落盘，刷新页面、崩溃、断电都不会丢已录内容。
- 结束时一键导出：`音频.wav` + `文字.txt` + 时间戳 `json` + 字幕 `srt`，文件名自动生成。
- 解压即用，不需要安装任何运行库；推理优先用 NVIDIA GPU，没有就自动用 CPU。

## 快速开始

1. 下载对应平台的包并解压：
   - Linux：`mind_flow-vX.Y.Z-x86_64-unknown-linux-gnu.tar.gz`
   - Windows：`mind_flow-vX.Y.Z-x86_64-pc-windows-gnu.zip`
2. 运行 `mind_flow`（Windows 双击 `mind_flow.exe`），浏览器会自动打开 `http://127.0.0.1:8730`。
3. 首次启动会自动下载模型（约 300MB，含 PARAformer-zh 识别、标点恢复、VAD）。
   没网时可以手动把模型目录放到 `data/models/`（详见下方）。
4. 按住空格开始说，松开出字；说完点右下角「结束并保存」。

命令行参数：

```
mind_flow [--data-dir DIR] [--model-dir DIR] [--port N] [--host IP] [--no-open]
          [--inference auto|cpu|cuda] [--engine real|stub]
```

## 目录结构

```
mind_flow(.exe)
data/
  config.json                 # 端口、数据目录、推理设备、下载源
  models/                     # 识别/标点/VAD 模型
  runtime/cuda/               # 可选的 CUDA 引擎（见下）
  sessions/<id>/              # 进行中的会话（audio.wav + session.json + raw/）
  recordings/                 # 保存好的 .wav/.txt/.json/.srt
```

## 用 GPU 加速（可选）

默认包只带 CPU 引擎。想用 NVIDIA GPU：

1. 确认已装 NVIDIA 驱动，并安装 **CUDA 12.x（或 13.x）+ cuDNN 9** 运行时。
2. 把 CUDA 引擎装到 `data/runtime/cuda/`（该目录里要有 `mind_flow-engine` 和它的 `.so`/`.dll`）：

   ```bash
   bash scripts/fetch-cuda-engine.sh                       # 从发布附件取
   CUDA_ENGINE_URL=<自己的包地址> bash scripts/fetch-cuda-engine.sh
   # 或者自己构建：
   bash scripts/fetch-vendor.sh --cuda12 && bash scripts/build-release.sh --cuda=12
   ```

3. 重启 `mind_flow`：界面状态条会显示当前推理设备（如 `CUDA · RTX 4060`）。
   探测失败时自动回落 CPU，并在界面写明原因；想强制 GPU 用 `--inference cuda`。
4. 真机验证（会打印设备名、provider、装载耗时，并提示回填 DESIGN.md）：

   ```bash
   bash scripts/verify-gpu.sh
   ```

Windows 上把引擎包里的 `mind_flow-engine.exe` 与所有 DLL 放到 `data\runtime\cuda\` 即可，
不需要改配置。

## 离线使用 / 手动放模型

- 有网机器：把 `data/models/` 整个目录拷到目标机器同样位置。
- 或者用 `--model-dir` 指向外部模型目录。
- 内网镜像：在 `data/config.json` 里设置 `model_base_url`（前缀替换 GitHub release 地址）
  和 `proxy`。

## 从源码构建

```bash
bash scripts/fetch-vendor.sh --api   # 取 sherpa-onnx 预编译库（github 不通时走 API）
bash scripts/build.sh --release      # 带真实引擎的二进制
bash scripts/test.sh                 # 格式 + Rust 全部测试 + 前端纯逻辑
bash scripts/smoke.sh                # stub 引擎跑完整流程
bash scripts/smoke-model.sh          # 真实模型识别并打印 RTF
node scripts/e2e-browser.mjs         # 真 Firefox 里验证界面行为
bash scripts/build-release.sh        # 打 Linux（可选 Windows/CUDA）产物到 dist/
```

发布到 GitHub（需要带写权限的 token，只放环境变量里，不写进仓库）：

```bash
GITHUB_TOKEN=xxx bash scripts/publish.sh "chore: 这次改了什么"
bash scripts/publish.sh --dry-run "chore: 只演练不推送"
```

模型目录布局（`data/models/`，也是 `vendor/models/` 的样子）：

```
models/paraformer-zh-2023-09-14-int8/{model.int8.onnx,tokens.txt}
models/punct-ct-transformer-zh-en-vocab272727-2024-04-12/{model.int8.onnx|model.onnx}
models/silero_vad.onnx
```

## 手测清单（自动化覆盖不到的部分）

自动化测试覆盖了协议、落盘、导出与界面渲染；下面这些请在真机浏览器里过一遍
（本项目的开发容器没有声卡，浏览器麦克风链路与页面发起的写请求无法在容器里稳定验证）：

1. 按住空格说话 2 秒再松开：出现「正在录音…」→ 松开后出灰色占位 → 变成文字。
2. 正在编辑某句文字时按空格：只输入空格、不触发录音，顶部提示「正在编辑，空格不录音」。
3. 点句子时间戳：播放器跳到该句位置开始播；录制下一段时再点，也能跳到已录部分。
4. 改一句文字后按 Enter 或点别处：`data/sessions/<会话>/session.json` 里立刻变成新文字。
5. 录到一半按 F5 刷新浏览器：已识别的句子仍在，继续按住空格能从原时间轴接着录。
6. 填标题后点「结束并保存」：`data/recordings/` 里出现 4 个同名文件，wav 能直接播放。
7. 同时开两个标签页：只有第一个能录音，第二个提示「另一个标签页正在录音，本页只读」。
8. 有 N 卡的机器：装好 CUDA 引擎后确认状态条显示 `CUDA · <显卡>`；拔掉引擎目录应自动回落 CPU 并说明原因。

## 已知限制（v1）

- 只在网页内按住空格生效（系统级全局热键会与输入法空格选词冲突，暂不做）。
- 只监听本机回环地址，不做局域网访问（非安全上下文拿不到麦克风权限）。
- 不支持导入音频文件、说话人分离、翻译、云端同步；纠错靠直接编辑文字。
- 编辑只改文字，不移动时间戳，也不支持句子合并/拆分。

架构与决策记录见 [DESIGN.md](DESIGN.md)。
