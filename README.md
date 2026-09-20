# mind_flow · 本地语音笔记

按住空格说一句，松开就变成文字；每句话都带语音时间戳，点一下就能跳回去听。
识别全部在这台电脑上完成（FunASR 导出的 Paraformer-zh + sherpa-onnx 推理），不联网、不上传。

> **一句话总结安装**：下载解压就能跑。程序本体不需要任何运行库（不需要 Python / FFmpeg / Node /
> Docker）；唯一要额外下载的是**模型（约 320MB，首次启动自动下载）**；
> 想要 GPU 加速才需要另外装 NVIDIA 驱动 + CUDA + cuDNN（可选，见第 4 节）。

---

## 1. 需要额外下载 / 安装的东西（先看这里）

### 1.1 必装清单

| 项目 | 要做什么 | 大小 | 官方地址 |
| --- | --- | --- | --- |
| 程序本体 | 下载发布包，解压即用（Windows 双击 exe，Linux `./mind_flow`） | 约 15–40MB | 本仓库 [Releases](https://github.com/Jnternet/mind_flow/releases) |
| 浏览器 | 用系统自带浏览器打开界面，**不需要装插件**；建议 Chrome / Edge / Firefox 较新版本（需支持 AudioWorklet） | 0 | [Chrome](https://www.google.com/chrome/) · [Edge](https://www.microsoft.com/edge) · [Firefox](https://www.mozilla.org/firefox/) |
| 麦克风 | 第一次录音时浏览器会弹权限询问，点「允许」；笔记本自带麦克风即可 | 0 | — |
| **识别模型** | **首次启动自动下载**（约 320MB，之后完全离线）；也可手动下载，见第 3 节 | 约 320MB | [sherpa-onnx 模型发布页](https://github.com/k2-fsa/sherpa-onnx/releases/tag/asr-models) · [HuggingFace 镜像](https://hf-mirror.com/csukuangfj/sherpa-onnx-paraformer-zh-2023-09-14) |

**不需要装的东西**（避免走弯路）：Python、Anaconda、PyTorch、FFmpeg、Node.js、Docker、CUDA（除非要 GPU 加速）、
各类 C++ 运行库。程序是单个可执行文件：Linux 版已把识别库静态链接进去，只依赖系统自带的
glibc / libstdc++（几乎所有发行版都有）；Windows 版只用系统自带 DLL。

### 1.2 想要 GPU 加速才需要（可选，四样缺一不可）

CPU 已经够快（实测约 20–30 倍实时：5.6 秒的语音端到端 0.3 秒），**不做这一步完全不影响使用**；
做了才会走显卡推理，界面状态条会显示 `CUDA · 显卡名`。

| # | 需要下载/安装 | 说明 | 官网 |
| --- | --- | --- | --- |
| 1 | **NVIDIA 显卡驱动** | 装完命令行 `nvidia-smi` 能打印出表格才算好 | [GeForce 驱动（中文）](https://www.nvidia.cn/geforce/drivers/) · [全部驱动](https://www.nvidia.com/Download/index.aspx) |
| 2 | **CUDA Toolkit 12.x 或 13.x** | 选与驱动匹配的版本（12.x 兼容面更广） | [下载最新](https://developer.nvidia.com/cuda-downloads) · [历史版本归档](https://developer.nvidia.com/cuda-toolkit-archive) |
| 3 | **cuDNN 9** | 需要注册 NVIDIA 账号；选 for CUDA 12.x / 13.x 的 9.x | [cuDNN 下载](https://developer.nvidia.com/cudnn) |
| 4 | **mind_flow 的 CUDA 引擎** | 用本仓库脚本取/构建，放进 `data/runtime/cuda/` | 见第 4 节；底层依赖 [sherpa-onnx 预编译库](https://github.com/k2-fsa/sherpa-onnx/releases/tag/v1.13.8) |

这三个 NVIDIA 组件加起来约 2–3GB 安装体积，是 GPU 路线的固有代价。程序不会替你安装它们，
也绝不会偷偷下载；装不全就自动用 CPU。

---

## 2. 三步上手教程

### 第 1 步：下载并解压程序

- **Windows**：到 [Releases](https://github.com/Jnternet/mind_flow/releases) 下载
  `mind_flow-vX.Y.Z-x86_64-pc-windows-gnu.zip`，解压到任意目录（例如 `D:\mind_flow\`），
  目录里会有 `mind_flow.exe` 和若干 `.dll`，**双击 `mind_flow.exe` 即可**。
- **Linux**：下载 `mind_flow-vX.Y.Z-x86_64-unknown-linux-gnu.tar.gz`（压缩包约 14MB，解压后约 34MB），
  执行 `tar xzf mind_flow-*.tar.gz && cd mind_flow-*`，然后 `./mind_flow`。

> 如果 [Releases](https://github.com/Jnternet/mind_flow/releases) 里暂时还没有产物（项目还在早期，
> 产物需要单独发版），可以按第 8 节从源码构建，命令是 `scripts/build.sh --release`。

程序目录就是它的全部：数据、模型、录音都放在同级的 `data/` 里，删掉目录即卸载干净。

### 第 2 步：启动，等模型下好

启动后会自动打开浏览器（默认 `http://127.0.0.1:8730`；端口被占用会自动顺延，终端会打印实际地址）。

**首次启动会发生什么**：

1. 界面顶部出现「还没准备好模型」提示条，点「开始下载」（或等它自动开始）；
2. 下载约 320MB：识别模型 243MB + 标点 76MB + VAD 0.6MB，提示条上显示实时百分比；
3. 下完自动装载引擎，提示条消失，状态条显示 `CPU` 或 `CUDA · 显卡名`；
4. 之后再启动就是秒开，全程不再联网。

下载失败或没网：见 [3.3 手动下载](#33-手动下载模型没网或下载失败时) 与 [3.4 内网 / 代理](#34-内网--代理--自定义下载源)。

### 第 3 步：按住空格说话

1. **按住空格**（或按住界面里的「按住录音」按钮）说一句，**松开**；
2. 松开后这一段进入识别队列（状态条显示「识别中 N」），出结果后自动插入文字流；
   识别期间可以立刻按住空格说下一句，互不阻塞；
3. 想听某一句：**点那句话前面的时间戳**，播放器跳到该位置开始播（录制中也能回放已录部分）；
4. 识别错了：**直接点文字改**，按 Enter 或点别处立刻落盘；
5. 说完：填标题 → **「结束并保存」**；不想要就点「丢弃」。

**界面速查**

| 操作 | 效果 |
| --- | --- |
| 按住 `空格` | 开始录音（正在编辑文字时空格正常输入，不会误触发） |
| 松开 `空格` / 窗口失焦 / 切走标签页 | 结束这一段并送去识别 |
| 点句子前的 `00:12` | 播放器跳到这句话开始播放 |
| 点句子文字 | 进入编辑：`Enter` 保存，`Esc` 放弃修改 |
| 底部播放键 / 进度条 | 播放、暂停、拖动回放整场录音 |
| 「结束并保存」 | 等识别跑完，导出 4 个文件到 `data/recordings/` |
| 「丢弃」 | 删掉这次会话（二次确认） |
| 右上角「设置」 | 数据目录、模型状态、当前推理设备，切换 `自动 / 强制 GPU / 只用 CPU` |

**保存出来的东西**（在 `data/recordings/`，文件名 = `日期_时间_标题`，重名自动加 `-2`）：

| 文件 | 内容 |
| --- | --- |
| `2026-09-20_1630_语音笔记.wav` | 整场录音合并成的音频（16kHz 单声道，段间自动补 0.4 秒静音） |
| `….txt` | 纯文字，一句一行，方便直接复制 |
| `….json` | 每句话的文字 + 起止毫秒 + 段落与引擎信息，程序以后能再读 |
| `….srt` | 标准字幕，可直接拖进剪映 / PR 当字幕 |

---

## 3. 模型：自动下载与手动下载

模型目录默认是程序同级 `data/models/`（可用 `--model-dir` 换位置）。

### 3.1 默认行为

启动时若发现模型不完整，会自动从国内可达的镜像（`https://hf-mirror.com`）下载；
每个文件都校验 SHA-256，中断会断点续传，校验不通过会重下。

### 3.2 模型清单、官方地址与放置位置

| 用途 | 存放位置（相对 `data/`） | 大小 | SHA-256 | 官方来源 |
| --- | --- | --- | --- | --- |
| 识别（Paraformer-zh，带逐字时间戳的 int8 版） | `models/paraformer-zh-2023-09-14-int8/model.int8.onnx` | 243,371,218 B | `f36a0433bcf096bd6d6f11b80a3ac8bed110bdca632fe0d731df8d1a84475945` | [HuggingFace](https://huggingface.co/csukuangfj/sherpa-onnx-paraformer-zh-2023-09-14) · [国内镜像](https://hf-mirror.com/csukuangfj/sherpa-onnx-paraformer-zh-2023-09-14) |
| 识别词表 | `models/paraformer-zh-2023-09-14-int8/tokens.txt` | 75,756 B | `59aba8873a2ed1e122c25fee421e25f283b63290efbde85c1f01a853d83cb6e6` | 同上仓库 |
| 标点恢复（CT-Transformer int8，首选） | `models/punct-ct-transformer-zh-en-vocab272727-2024-04-12/model.int8.onnx` | 75,519,198 B | `65a3fb9f5ad7bfb96bf69e0dc4481df97f6ee60513c1d94ce981ba6effd524b1` | [GitHub punctuation-models](https://github.com/k2-fsa/sherpa-onnx/releases/tag/punctuation-models) |
| 标点恢复（fp32，GitHub 打不开时的备选） | `models/punct-ct-transformer-zh-en-vocab272727-2024-04-12/model.onnx` | 294,372,519 B | `e93593a6dbd69a07f8734ef269dbe861a379755f8d1c8354719432116f2c44bd` | [HuggingFace](https://huggingface.co/csukuangfj/sherpa-onnx-punct-ct-transformer-zh-en-vocab272727-2024-04-12) · [国内镜像](https://hf-mirror.com/csukuangfj/sherpa-onnx-punct-ct-transformer-zh-en-vocab272727-2024-04-12) |
| 人声检测 VAD（Silero） | `models/silero_vad.onnx` | 643,854 B | `9e2449e1087496d8d4caba907f23e0bd3f78d91fa552479bb9c23ac09cbb1fd6` | [GitHub asr-models](https://github.com/k2-fsa/sherpa-onnx/releases/tag/asr-models) |

说明：

- 前两个文件是**必需**的；标点与 VAD 缺失程序仍能跑（退化成按静音断句 / 整段识别），
  界面会显示「识别可用（标点/VAD 缺失）」。
- 标点两个文件**只需其一**，程序优先用 int8。
- 模型来自 FunASR 生态：算法源自 [FunASR / Paraformer](https://github.com/modelscope/FunASR)
  （[ModelScope 原始模型](https://www.modelscope.cn/models/iic/speech_paraformer-large_asr_nat-zh-cn-16k-common-vocab8404-pytorch)），
  由 [sherpa-onnx](https://github.com/k2-fsa/sherpa-onnx) 导出为 ONNX 供本程序推理。

### 3.3 手动下载模型（没网或下载失败时）

在程序目录执行（Windows 把 `mkdir -p` 换成 `md`，`curl` 用 `curl.exe` 同样参数即可）：

```bash
cd <程序目录>
mkdir -p data/models/paraformer-zh-2023-09-14-int8 \
         data/models/punct-ct-transformer-zh-en-vocab272727-2024-04-12
B=https://hf-mirror.com        # 打不开就换成 https://huggingface.co

# 1) 识别模型（必需，两个文件）
curl -L -C - -o data/models/paraformer-zh-2023-09-14-int8/model.int8.onnx \
  $B/csukuangfj/sherpa-onnx-paraformer-zh-2023-09-14/resolve/main/model.int8.onnx
curl -L -C - -o data/models/paraformer-zh-2023-09-14-int8/tokens.txt \
  $B/csukuangfj/sherpa-onnx-paraformer-zh-2023-09-14/resolve/main/tokens.txt

# 2) 标点模型（二选一）
# 2a) 首选 int8：来自 GitHub release（GitHub 打不开就改用 2b）
curl -L -C - -o punct.tar.bz2 \
  https://github.com/k2-fsa/sherpa-onnx/releases/download/punctuation-models/sherpa-onnx-punct-ct-transformer-zh-en-vocab272727-2024-04-12-int8.tar.bz2
tar xjf punct.tar.bz2
cp sherpa-onnx-punct-ct-transformer-zh-en-vocab272727-2024-04-12-int8/model.int8.onnx \
   data/models/punct-ct-transformer-zh-en-vocab272727-2024-04-12/
# 2b) 备选 fp32：来自 HuggingFace 镜像
curl -L -C - -o data/models/punct-ct-transformer-zh-en-vocab272727-2024-04-12/model.onnx \
  $B/csukuangfj/sherpa-onnx-punct-ct-transformer-zh-en-vocab272727-2024-04-12/resolve/main/model.onnx

# 3) VAD（可选，仅 0.6MB）
curl -L -C - -o data/models/silero_vad.onnx \
  https://github.com/k2-fsa/sherpa-onnx/releases/download/asr-models/silero_vad.onnx
```

**校验是否下完整**（大小对不上就是没下完，重跑上面命令会续传）：

```bash
sha256sum data/models/paraformer-zh-2023-09-14-int8/model.int8.onnx        # Linux / macOS
certutil -hashfile data\models\paraformer-zh-2023-09-14-int8\model.int8.onnx SHA256   # Windows
```

对照 3.2 表格里的 SHA-256；程序启动时也会自己再校验一次，不一致会自动重下。

### 3.4 内网 / 代理 / 自定义下载源

编辑 `data/config.json`：

```json
{
  "model_base_url": "https://hf-mirror.com",
  "proxy": "http://192.168.1.10:7890"
}
```

- `model_base_url`：替换掉所有 HuggingFace 直链的前缀，填你司内网镜像或自建反代即可。
- `proxy`：只影响模型下载，填 HTTP 代理；留空表示直连。
- 已装好的机器：把整个 `data/models/` 拷到目标机器同样位置，程序会跳过下载，完全离线可用。

---

## 4. GPU 加速安装教程（可选，进阶）

前提是 **NVIDIA 独立显卡**。程序启动时会自动探测：探不到或依赖没装全就**静默回落 CPU**，
并在界面「设置」里写明原因。想强制走 GPU、失败直接报错，用 `--inference cuda` 启动。

### 第 1 步：安装 NVIDIA 驱动

到 [NVIDIA 驱动下载（中文）](https://www.nvidia.cn/geforce/drivers/)（专业卡用
[这里](https://www.nvidia.com/Download/index.aspx)）按型号下载安装，重启后验证：

```bash
nvidia-smi      # 能打印显卡名、驱动版本、显存即 OK
```

### 第 2 步：安装 CUDA Toolkit（12.x 或 13.x）

程序对应两套：**CUDA 12.x + cuDNN 9** 与 **CUDA 13.x + cuDNN 9**（基于 onnxruntime 1.28.2）。
驱动较新用 13.x，想要兼容面更广用 12.x。

- 下载：[最新版](https://developer.nvidia.com/cuda-downloads) ·
  [历史版本归档（推荐，从这里挑 12.x 的具体小版本）](https://developer.nvidia.com/cuda-toolkit-archive)
- 驱动与 CUDA 的版本对应（驱动太老会报 `CUDA driver version is insufficient`）：
  [CUDA Release Notes 版本对应表](https://docs.nvidia.com/cuda/cuda-toolkit-release-notes/index.html)

### 第 3 步：安装 cuDNN 9

- 下载（需注册 NVIDIA 开发者账号）：[https://developer.nvidia.com/cudnn](https://developer.nvidia.com/cudnn)
- 选 **for CUDA 12.x / 13.x 的 9.x 版本**；支持矩阵见
  [cuDNN Support Matrix](https://docs.nvidia.com/deeplearning/cudnn/backend/latest/reference/support-matrix.html)
- 把解压出来的 `bin/`（Windows 的 `cudnn*.dll`）和 `lib/`（Linux 的 `libcudnn*.so*`）
  放进 CUDA 安装目录对应子目录，或放进系统搜索路径（Windows `PATH`、Linux `LD_LIBRARY_PATH`）。
- 依赖细节见 [ONNX Runtime CUDA EP 文档](https://onnxruntime.ai/docs/execution-providers/CUDA-ExecutionProvider.html)。

### 第 4 步：安装 mind_flow 的 CUDA 引擎

目标：让 `data/runtime/cuda/` 里出现 `mind_flow-engine`（Windows 为 `mind_flow-engine.exe`）
以及它依赖的 `libonnxruntime.so`、`libsherpa-onnx-c-api.so`（Windows 为同目录 `.dll`）。
**引擎和库必须放在同一个目录**（程序已内置 `$ORIGIN` 查找路径）。

```bash
# 方式 A：本机自己构建（当前推荐，因为发布附件还没上传）
bash scripts/fetch-vendor.sh --cuda12      # 取 CUDA 12.x 版 sherpa-onnx 预编译库（约 243MB）
bash scripts/build-release.sh --cuda=12    # 产出 dist/mind_flow-cuda-engine-v0.1.0-linux-x64-cuda12.tar.gz
bash scripts/fetch-cuda-engine.sh          # 把上面的产物装到 data/runtime/cuda/

# 方式 B：直接给一个引擎包地址
CUDA_ENGINE_URL=https://<你的地址>/mind_flow-cuda-engine.tar.gz bash scripts/fetch-cuda-engine.sh

# 方式 C：手动解压，把 mind_flow-engine 与它的 .so/.dll 一起放进 data/runtime/cuda/
```

Windows：先准备 Windows 版预编译库再交叉编译
（`SHERPA_WIN_LIB_DIR=<win-x64 的 lib 目录> bash scripts/build-release.sh`），
然后把 `mind_flow-engine.exe` 和所有 DLL 放进 `data\runtime\cuda\`。

### 第 5 步：确认 GPU 生效

1. 重启 `mind_flow`，界面右上角状态条显示 `CUDA · 你的显卡名` 即成功；
2. 没生效时打开「设置」，里面写着当前设备与失败原因（缺哪个库、驱动太旧、探测超时…）；
3. 一键自检（打印设备名、provider、装载耗时）：

   ```bash
   bash scripts/verify-gpu.sh
   ```

### GPU 常见报错

| 现象 | 原因与处理 |
| --- | --- |
| `CUDA driver version is insufficient` | 驱动太旧：升级驱动，或改用更低版本的 CUDA（12.x） |
| 找不到 `cudnn64_9.dll` / `libcudnn.so.9` | cuDNN 没装或没加进搜索路径（第 3 步） |
| 自检超时（8 秒）后回落 CPU | 首次创建 CUDA 上下文较慢；重试一次，仍失败看「设置」里的原因 |
| 状态一直显示 CPU 且没有原因 | 确认 `data/runtime/cuda/mind_flow-engine`（Windows 为 `.exe`）存在且可执行 |
| Linux 报找不到 `libonnxruntime.so` | 引擎与 `.so` 必须在同一目录，别只拷可执行文件 |

---

## 5. 目录结构与命令行参数

```
mind_flow(.exe)
data/
  config.json                 # 端口、下载源、代理、推理设备偏好
  models/                     # 识别 / 标点 / VAD 模型（第 3 节）
  runtime/cuda/               # 可选的 CUDA 引擎（第 4 节）
  sessions/<会话id>/           # 进行中的会话：audio.wav + session.json + seg/ + raw/
  recordings/                 # 保存好的 .wav / .txt / .json / .srt
```

```
mind_flow [--data-dir DIR] [--model-dir DIR] [--port N] [--host IP] [--no-open]
          [--inference auto|cpu|cuda] [--engine real|stub]
```

| 参数 | 说明 |
| --- | --- |
| `--data-dir` | 换数据目录（默认程序同级 `data/`；不可写时自动回退用户目录） |
| `--model-dir` | 换模型目录（默认 `<data>/models`） |
| `--port` / `--host` | 换监听端口/地址（默认 `127.0.0.1:8730`，占用会顺延最多 20 次） |
| `--no-open` | 不自动打开浏览器 |
| `--inference` | `auto`（默认，优先 GPU）/ `cpu` / `cuda`（强制，失败报错） |
| `--engine stub` | 隐藏的测试桩引擎（确定性文本），供自动化测试用 |

---

## 6. 常见问题

**Q：浏览器没自动打开？** 手动访问终端打印的地址（默认 `http://127.0.0.1:8730`）。

**Q：按住空格没反应，或提示「另一个标签页正在录音」？**
① 确认焦点在页面上；② 只在打开本程序的标签页里生效；③ 同一时间只允许一个标签页录音，
关掉多余标签页；④ 正在编辑句子时空格是输入空格，不会录音（刻意设计）。

**Q：识别不出字 / 一直「识别中」？**
① 看是否提示「模型不完整」，是就等下载完或按第 3 节手动放模型；② 麦克风是否被会议/直播软件占用；
③ 说话不足 0.3 秒会被当误触丢掉；④ 麦克风音量太小或离得远。

**Q：模型下载慢或失败？** 换 `model_base_url`、挂代理（3.4），或按 3.3 手动下载；
识别模型只有 HuggingFace 源，标点另有 GitHub 源；都不通就从装好的机器拷 `data/models/`。

**Q：录制中浏览器崩了 / 断电会丢吗？** 不会。每一段说完就已经写进 `data/sessions/<会话id>/`，
重开程序会自动恢复，连「录了但还没识别完」的段落都会重新识别。

**Q：怎么备份 / 清理 / 卸载？** 录音都在 `data/recordings/`；想恢复出厂就删 `data/`；
卸载 = 删除程序目录。

**Q：能在局域网另一台电脑或手机上用吗？** v1 不行：程序只监听本机，且浏览器要求安全上下文
（局域网 IP 属非安全上下文）才给麦克风权限。

**Q：占多少空间？** 程序约 15–40MB，模型约 320MB；录音约 **1.9MB/分钟**（1 小时约 115MB）。

---

## 7. 已知限制（v1）

- 按住空格只在网页内生效（系统级全局热键会与输入法空格选词冲突）。
- 只监听本机回环地址，不做局域网访问。
- 不支持导入已有音频文件、说话人分离、翻译、云端同步。
- 纠错靠直接编辑文字；编辑不移动时间戳，也不支持句子合并/拆分。
- GPU 仅支持 NVIDIA CUDA，且需自备 CUDA/cuDNN（第 4 节）。

---

## 8. 开发：从源码构建与发布

```bash
bash scripts/fetch-vendor.sh --api   # 取 sherpa-onnx 预编译库（github 不通时走 API）
bash scripts/build.sh --release      # 带真实引擎的二进制
bash scripts/test.sh                 # 格式 + Rust 全部测试 + 前端纯逻辑
bash scripts/smoke.sh                # stub 引擎跑完整流程（送音频 → 出句子 → 导出四个文件）
bash scripts/smoke-model.sh          # 真实模型识别并打印 RTF
node scripts/e2e-browser.mjs         # 真 Firefox 里验证界面行为
bash scripts/build-release.sh        # 打 Linux（可选 Windows/CUDA）产物到 dist/
```

发布到 GitHub（token 只放环境变量，不写进仓库）：

```bash
GITHUB_TOKEN=xxx bash scripts/publish.sh "chore: 这次改了什么"
bash scripts/publish.sh --dry-run "chore: 只演练不推送"
```

模型目录布局速查：

```
models/paraformer-zh-2023-09-14-int8/{model.int8.onnx,tokens.txt}
models/punct-ct-transformer-zh-en-vocab272727-2024-04-12/{model.int8.onnx|model.onnx}
models/silero_vad.onnx
```

### 真机手测清单（自动化覆盖不到的部分）

自动化测试覆盖协议、落盘、导出与界面渲染；下面这些请在真机浏览器里过一遍
（开发容器没有声卡，浏览器麦克风链路与页面发起的写请求无法在容器里稳定验证）：

1. 按住空格说 2 秒再松开：出现「正在录音…」→ 松开出灰色占位 → 变成文字。
2. 正在编辑某句文字时按空格：只输入空格、不触发录音。
3. 点句子时间戳：播放器跳到该句开始播；录制下一句时再点也能跳到已录部分。
4. 改一句文字后按 Enter 或点别处：`data/sessions/<会话>/session.json` 立刻变成新文字。
5. 录到一半按 F5 刷新：已识别句子仍在，继续按住空格能从原时间轴接着录。
6. 填标题后点「结束并保存」：`data/recordings/` 出现 4 个同名文件，wav 能直接播放。
7. 同时开两个标签页：只有第一个能录音，第二个提示「另一个标签页正在录音，本页只读」。
8. 有 N 卡的机器：按第 4 节装好后状态条显示 `CUDA · <显卡>`；挪走引擎目录应自动回落 CPU 并说明原因。

架构与决策记录见 [DESIGN.md](DESIGN.md)，协作与提交约定见 [AGENTS.md](AGENTS.md)。
