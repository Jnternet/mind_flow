# 更新日志

## v0.1.0

- 首版：按住空格录音、松开识别，句子级时间戳点击跳转，文字直接编辑。
- 每段增量落盘，刷新/崩溃可恢复；结束时导出 wav + txt + json + srt 并自动命名。
- 推理优先 CUDA、不可用回落 CPU（CUDA 引擎为可选附件，需自备 CUDA/cuDNN 运行时）。
- 模型首次运行自动下载（支持镜像/代理/手动放置），运行时不联网。
- 模型：FunASR 导出的 Paraformer-zh（带 token 时间戳的 int8 版）+ CT-Transformer 标点 + Silero VAD。
- 产物：Linux 单文件 tar.gz（静态链接）；Windows 交叉编译包（共享库 + DLL）。
