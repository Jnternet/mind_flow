# mind_flow 设计文档

本地语音笔记：浏览器采集麦克风 → 本机 Paraformer-zh 识别 → 每句一个可点击时间戳 →
增量落盘、随时纠错、结束时导出音频与文字。

本文档既是设计说明，也是需求与决策的唯一事实来源。改动行为前先改本文档。

## 目标与范围

要做的：

- 本地语音转文字（FunASR 导出的 Paraformer-zh 权重，经 sherpa-onnx 推理），不依赖云端、不依赖 Python。
- 每一句话对应一个语音时间戳，界面上可点击跳转到回放位置。
- 简洁美观的 GUI：只显示必要内容（状态条 / 句子流 / 播放器）。
- 识别结果可直接编辑纠错。
- 一次使用中可断续记录（按住空格说一句、松开；可反复），结束时本地保存音频与文字并自动命名。
- 按住空格开始录音（网页内生效）。
- 跨平台可移植免安装：解压即用，不装任何运行库（GPU 例外，见下）。
- 网页访问使用：本机浏览器打开 `http://127.0.0.1:<port>`。
- 推理设备优先 CUDA，不可用则 CPU。

不做的（v1 明确排除）：

- 音频文件导入、说话人分离、翻译、云端同步、替换词典/引擎热词。
- 局域网访问（非安全上下文拿不到麦克风权限）、全局系统热键（与输入法空格冲突）。
- GPU 的 TensorRT / DirectML / CoreML 后端（本版本只发布 CUDA 预编译）。
- 句子的合并/拆分（编辑只改文字，不动时间戳）。

## 架构

```
浏览器（麦克风 + 界面）
  │  getUserMedia → AudioWorklet → Int16 PCM
  │  WebSocket: 控制消息(JSON) + 音频帧(二进制)
  ▼
mind_flow 主进程（Rust / axum，仅监听 127.0.0.1）
  ├─ 会话状态机：段(segment) / 句(sentence) / audio_version
  ├─ 音频落盘：raw/<段>.pcm → 重采样 16k → audio.wav（增量重写头）
  ├─ 识别队列：mpsc → 独立 OS 线程持有引擎（CPU 内置）
  ├─ 可选 GPU：data/runtime/cuda/mind_flow-engine（子进程，--probe 自检）
  ├─ 模型下载器：SHA-256 校验 + Range 断点续传 + 解包
  └─ 导出：<名称>.wav / .txt / .json / .srt
```

要点：

- 前端是内嵌资源（`include_dir`），运行时零网络请求（除首次下载模型/CUDA 引擎）。
- 识别在独立 OS 线程里跑，不阻塞 axum 的事件循环；识别期间可以继续录下一段。
- `session.json` 是恢复的唯一事实来源，原子写（临时文件 + rename）。
- 引擎与界面之间的协议（`web/lib/protocol.js` / `src/engine.rs`）与前端状态机（`web/lib/state.js`）
  是可单测的纯逻辑，`web/app.js` 只做接线与渲染。

## 决策记录

| 决策 | 选择 | 理由 / 被否方案 |
| --- | --- | --- |
| 识别引擎 | sherpa-onnx 1.13.8（Rust crate）+ FunASR 导出的 Paraformer-zh ONNX | 打包 FunASR（Python）要每平台 1–1.5GB 随附运行时且无法交叉编译；已验证 Paraformer 结果带 token 级时间戳，句级时间戳可原生实现 |
| 句级时间戳 | Paraformer token 时间戳 + CT-Transformer 标点断句 | 纯 VAD 边界断句精度差；纯标点不断句无法给时间 |
| 解码时机 | 松开空格后整段离线识别 | 流式 paraformer-zh-streaming 准确率略低、时间戳弱；离线模型在 CPU 上已足够快，且识别不阻塞下一段 |
| 热键范围 | 仅网页内按住空格 | 全局热键需各平台原生实现，且与中文输入法空格选词冲突 |
| 音频形态 | 一次会话一个合并 WAV（段间 0.4s 静音） | 多文件时间戳跨文件、回放体验差 |
| 文字产物 | .txt + .json + .srt | txt 方便复制，json 保留时间戳以便二次开发/恢复，srt 便于做字幕 |
| 中途容错 | 每段增量落盘 + 会话恢复 | 只在保存时落盘会因刷新/崩溃丢失整场记录 |
| 模型投放 | 首次运行自动下载（可手动放置） | 内置进包会让每个平台产物多 300MB |
| 平台 | Linux x86_64 + Windows x86_64 | macOS 无法在 Linux 上交叉编译与验证，后置 |
| GPU 交付 | CPU 内置 + CUDA 引擎可选附件 | 默认包保持几十 MB、免安装；内置 CUDA 对无 N 卡用户是纯浪费 |
| GPU 选路 | 子进程 `--probe` 自检成功才启用，否则回落 CPU | 单进程内无法在不重链接的前提下切换 EP；子进程还能隔离 GPU 侧崩溃 |
| CUDA 运行时 | 不打包，启动检测 + 界面提示 | CUDA/cuDNN 约 2–3GB，且与驱动版本强相关，打包不现实 |
| 纠错 | 只做直接编辑 | 词典/热词需要引擎侧支持，收益与复杂度不匹配 |
| 命名 | `YYYY-MM-DD_HHMM_<标题>`，保存前可改 | 时间序即文件名序；首句命名会因识别错字污染文件名 |

## 接口与格式

### HTTP

| 方法 | 路径 | 说明 |
| --- | --- | --- |
| GET | `/` | 内嵌前端 |
| GET | `/api/state` | 会话 id、句子/段、`audio_version`、`pending_jobs`、模型状态、推理设备与原因、数据目录、录音权归属 |
| POST | `/api/session/start` | 新建会话（首次按住空格时前端也会隐式调用） |
| POST | `/api/session/finalize` | `{title}`，等队列跑完 → 导出四个文件 → 返回路径 |
| POST | `/api/session/discard` | 丢弃当前会话 |
| PATCH | `/api/sentences/{id}` | `{text}`，编辑纠错（空文本即该句不输出） |
| GET | `/api/sessions/current/audio` | 合并 WAV，支持 Range，`?v=N` 破缓存 |
| GET | `/api/models` | 模型状态 |
| POST | `/api/models/download` | 触发下载（进度经 WS 推送） |
| GET | `/api/recordings` | 已保存的会话列表（文件名/时长/句数） |

### WebSocket `/api/ws`

上行：`{"type":"start"}`、`{"type":"stop"}`、`{"type":"claim_recorder"}`，
以及紧随 `start` 之后的二进制 Int16LE PCM 帧（设备采样率）。

下行：

- `{"type":"status", ...}`：会话、待识别数、设备、模型状态
- `{"type":"segment_closed","segment":{...}}`
- `{"type":"sentences_added","sentences":[...],"audio_version":N}`
- `{"type":"model_progress","downloaded":N,"total":N,"phase":"..."}`
- `{"type":"engine_changed","provider":"cpu|cuda","device":"...","reason":"..."}`
- `{"type":"error","code":"...","message":"..."}`

### 文件格式

`.json` v1：

```json
{
  "version": 1,
  "title": "语音笔记",
  "created_at": "2026-09-20T16:30:12+08:00",
  "audio": "2026-09-20_1630_语音笔记.wav",
  "duration_ms": 12345,
  "sample_rate": 16000,
  "channel": 1,
  "sample_format": "s16le",
  "engine": {"name": "sherpa-onnx", "version": "1.13.8", "model": "...", "punctuation": "...", "vad": "silero-vad", "provider": "cpu", "device": "CPU"},
  "segments": [{"id": 1, "start_ms": 0, "end_ms": 4200}],
  "sentences": [{"id": "s-1-1", "segment_id": 1, "start_ms": 320, "end_ms": 2400, "text": "今天讨论了三件事。"}]
}
```

`.txt`：一句一行纯文本（空句跳过）。
`.srt`：标准字幕，`HH:MM:SS,mmm`，空句跳过。

### CLI

```
mind_flow [--data-dir DIR] [--model-dir DIR] [--port N] [--host IP] [--no-open]
          [--inference auto|cpu|cuda] [--engine real|stub]
```

隐藏模式：`--engine-server`（CUDA 引擎子进程模式）、`--probe`（自检并输出一行 JSON）。

## GPU 策略

- 主进程内置 CPU 引擎；`data/runtime/cuda/mind_flow-engine[.exe]` 存在时后台探测：
  以 `--probe` 启动，自检内容包括创建 CUDA provider 的 recognizer + 小样本解码。
- 探测成功 → 后续识别任务转发给子进程（同机回环 + 一次性令牌），界面显示设备名。
- 回落判定（`inference=auto`）：runtime 目录不存在 / 子进程启动失败 / probe 超时 8s /
  输出非法 / 协议版本不匹配 / 运行中崩溃 → 回落 CPU 并说明原因。
- `inference=cuda` 为强制模式：探测失败直接报错，不静默回落。
- 运行中不热切换；探测完成前到达的任务先用 CPU 处理。

## 里程碑

- [x] M1 文档落地：DESIGN.md / AGENTS.md / README.md / CHANGELOG.md
- [x] M2 Spike：sherpa-onnx 构建、时间戳标定、模型资产与校验和、Windows 链接路径
- [x] M3 骨架：axum + 内嵌前端 + 数据目录/配置 + 状态接口 + 模型下载 + stub 引擎
- [x] M4 采集链路：WS、AudioWorklet、空格语义、raw 落盘、重采样、合并 WAV、电平
- [x] M5 识别管线：工作线程、VAD 分块、Paraformer + 标点、绝对时间戳、恢复、事件
- [x] M6 GPU：CUDA 引擎构建目标、--probe 自检、子进程转发、回落提示、取用脚本
- [x] M7 文本界面：句子流、编辑、播放器与点句跳转、编辑态空格让位、多标签只读
- [x] M8 收尾：命名/导出/原子落地/丢弃、测试、打包脚本、产物

## 测试与验收

- `cargo test`：重采样、WAV 头重写与 Range、时间戳换算与断句、命名与重名、会话恢复、
  REST 处理函数、WS 编解码、`session.json` 原子写、引擎探测回落（启动失败/超时/非法输出/协议不匹配）。
- `node --test tests/js/*.test.js`：协议编解码、状态归约、空格键规则、时间戳格式化。
- `node scripts/e2e-browser.mjs`：Firefox + 假麦克风 + stub 引擎跑完整用户流程。
- 架构测试：前端必须内嵌（无 CDN/外链）、运行时除模型/CUDA 下载外无网络调用、除数据目录外不写文件。
- `scripts/smoke-model.sh`：真实模型冒烟（无模型时跳过）。
- 本机无 GPU：CUDA 路径需在真机用 `scripts/verify-gpu.sh` 验证并回填本文件。

## 假设与默认

- 中文为主、中英混合；Paraformer-zh 双语模型即可。
- CPU 线程数 `min(4, 可用核数)`；16 kHz 单声道 s16le；段间静音 0.4s；短于 300ms 的段丢弃。
- 单用户单会话；多标签页只有第一个连接的标签有录音权。
- 数据目录默认程序同级 `data/`（不可写时回退系统应用数据目录）。
- 模型首次下载约 300MB；`model_base_url`/`proxy` 可配镜像或代理，也可手动放置模型目录。

## 实测记录

### Spike（2026-09-20，本机 Fedora 容器 / i9-13980HX / 无 GPU）

**时间戳**：只有导出时带 `us_cif_peak` 输出的 Paraformer 才会给时间戳。逐个实测：

- `sherpa-onnx-paraformer-zh-2023-09-14`（含其 HF 上的 int8 版）：**有时间戳**（29 tokens → 29 个）。
- `sherpa-onnx-paraformer-zh-int8-2025-10-07`（Chuan 微调）：无时间戳（只有 logits 输出）。
- `sherpa-onnx-paraformer-zh-2024-03-09` / `-small-2024-03-09`：官方文档示例里 timestamps 全为空。

**时间戳单位**：秒（f32，每 token 一个），由 sherpa-onnx 源码 `offline-paraformer-greedy-search-decoder.cc`
的 `scale = 10.0 * 6 / 3 / 1000` 决定：10ms 帧移 × LFR 窗口 6 ÷ CIF 上采样 3 = **每帧 20ms**。
本机实测首个 token 0.36s、末个 5.10s（音频 5.615s），与官方文档 fp32 版数值一致（int8 有 ±40ms 抖动）。

**性能**（int8，4 线程，CPU）：模型加载 1.14–1.41s；6.78s 音频解码 145ms、5.61s 音频解码 209ms →
**RTF 0.021–0.037**，即 CPU 约 30 倍实时。标点模型加载 0.22s、每段 5–7ms。VAD 正常输出语音段。

**模型选型（已定）**：`sherpa-onnx-paraformer-zh-2023-09-14` 的 int8 导出（243MB）——唯一同时满足
「有时间戳 + 体积可接受」的 Paraformer-zh 导出；标点用
`sherpa-onnx-punct-ct-transformer-zh-en-vocab272727-2024-04-12-int8`（75MB）；VAD 用 `silero_vad.onnx`（0.6MB）。
合计约 320MB。

**下载源**：本容器里 `github.com` 不可达、`api.github.com` 可达但会限流；`hf-mirror.com` 稳定可达。
因此默认下载源用 hf-mirror 的直链（`model.int8.onnx` + `tokens.txt`），`model_base_url` 可换成
`https://huggingface.co` 或内网镜像。

**构建**：`sherpa-onnx-sys` 自带的下载器在本网络下会 `Unexpected EOF`，改成用 curl 预取
`sherpa-onnx-v1.13.8-linux-x64-static-lib.tar.bz2` 到 `vendor/`，再用 `SHERPA_ONNX_LIB_DIR` 指向其 `lib/`；
二进制里未引用符号时链接器不会把静态库拉进来，所以真正的链接发生在第一次调用 API 时。

### CUDA 真机验证（待补）

- 待在有 N 卡的机器上跑 `scripts/verify-gpu.sh`，回填设备名、provider、RTF 与回落原因。

### 端到端与产物验证（2026-09-20）

- **真实模型全链路**（`bash scripts/smoke-model.sh`，CPU）：
  5.61s 中文音频 → `对我做了介绍啊，那么我想说的是呢，` + `大家如果对我的研究感兴趣呢。`
  两句时间戳 `374–3234ms` / `3234–5014ms`，端到端 0.30s，**RTF 0.053**；
  标点模型输出 `model.int8.onnx`，VAD 为 `silero_vad.onnx`。
- **stub 全链路**（`bash scripts/smoke.sh`）：16kHz 与 48kHz 两段 → 合并音频 73644 字节 +
  txt/json/srt 四个产物齐全。
- **Rust 测试**：37 个单元 + 7 个 HTTP/WS 集成 + 6 个架构约束。
- **浏览器端到端**（`node scripts/e2e-browser.mjs`，真 Firefox + BiDi）：
  打开页面即恢复出已有 2 句（覆盖刷新恢复路径）、点第 1/2 句时间戳播放器分别跳到 0.00s / 1.40s、
  播放器加载 3.10s 合并音频、保存与丢弃入口可用。
- **产物**：`dist/mind_flow-v0.1.0-x86_64-unknown-linux-gnu.tar.gz`（14MB，单文件，
  解压即跑，数据目录自动落在程序同级 `data/`）。

### 与计划的偏差（实施中调整，均已落地）

1. **标点模型多了一条 HF 兜底源**：GitHub release 不可达时改用 `hf-mirror` 上的 fp32 导出
   （`model.onnx`，sha256 `e93593…c44bd`，可选、失败不阻断），`ModelPaths` 优先用 int8。
2. **新增 `sherpa-shared` 特性**：Windows 发行包与 CUDA 引擎都要链接共享库（DLL/so 随包），
   默认 Linux 包仍用 `sherpa`（静态、单文件）。
3. **新增 `GET /api/session`**：`/api/state` 只带计数，页面刷新后必须靠这个接口取回句子正文
   （端到端测试暴露的真实缺陷，已补集成测试）。
4. **浏览器端到端范围收窄**：本容器的网络代理会把「浏览器页面发起的带 body 请求」与
   Firefox 麦克风音频链路整段卡死（curl / Node / Rust 客户端均正常），因此 e2e 只验证浏览器侧
   不写数据的路径（渲染 / 跳转 / 播放 / 入口可用），写路径由 Rust 集成测试与 `smoke.sh` 覆盖，
   真机手测清单见 README。
5. **修掉两个测试暴露的真实缺陷**：引擎队列在引擎未就绪时会 `pop_front` 丢掉排队任务；
   末句 `end_ms` 误用句首时间而不是末 token 时间。

### 里程碑完成情况

除「CUDA 真机验证」外全部完成；Windows 产物需要 Windows 预编译库（本容器无）与真机验证，
`scripts/build-release.sh` 已就绪，缺库时会明确提示并跳过。
