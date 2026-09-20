// 界面状态归约（纯函数，Node 里可测）。

import { mergeSentences } from "./protocol.js";

export function initialState() {
  return {
    clientId: null,
    session: null,
    sentences: [],
    segments: [],
    pending: 0,
    audioVersion: 0,
    provider: "cpu",
    device: "CPU",
    reason: "",
    modelReady: false,
    modelExtrasReady: false,
    modelDownloading: false,
    dataDir: "",
    recordingsDir: "",
    title: "语音笔记",
    recording: false,
    recorderClient: null,
    /** 服务端有没有把录音权给这个连接（hello 里告诉我们）。 */
    granted: null,
    toast: null,
    modelProgress: null,
  };
}

/** 应用一个 WS 事件，返回新状态（不修改入参）。 */
export function applyEvent(state, event) {
  if (!event || typeof event.type !== "string") return state;
  switch (event.type) {
    case "hello":
      return {
        ...state,
        clientId: event.client_id ?? state.clientId,
        recorderClient: event.recorder ? event.client_id : state.recorderClient,
        granted: Boolean(event.recorder),
      };
    case "status":
      return {
        ...state,
        session: event.session ?? null,
        pending: event.pending_jobs ?? 0,
        audioVersion: event.audio_version ?? state.audioVersion,
        provider: event.provider ?? state.provider,
        device: event.device ?? state.device,
        reason: event.engine_reason ?? state.reason,
        modelReady: Boolean(event.model_ready),
        modelExtrasReady: Boolean(event.model_extras_ready),
        modelDownloading: Boolean(event.model_downloading),
        dataDir: event.data_dir ?? state.dataDir,
        recordingsDir: event.recordings_dir ?? state.recordingsDir,
        title: event.title ?? state.title,
        recording: Boolean(event.recording),
        recorderClient: event.recorder_client ?? state.recorderClient,
        granted: event.recorder_client
          ? event.recorder_client === state.clientId
          : state.granted,
      };
    case "sentences_added":
      return {
        ...state,
        sentences: mergeSentences(state.sentences, event.sentences ?? []),
        audioVersion: event.audio_version ?? state.audioVersion,
      };
    case "segment_closed":
      return {
        ...state,
        segments: [...state.segments, event.segment].filter(Boolean),
      };
    case "model_progress":
      return {
        ...state,
        modelProgress: {
          file: event.file,
          downloaded: event.downloaded ?? 0,
          total: event.total ?? 0,
          finished: Boolean(event.finished),
        },
      };
    case "engine_changed":
      return {
        ...state,
        provider: event.provider ?? state.provider,
        device: event.device ?? state.device,
        reason: event.reason ?? state.reason,
      };
    case "error":
      return { ...state, toast: event.message ?? "出错了" };
    default:
      return state;
  }
}

/** 当前标签页是否持有录音权。 */
export function canRecord(state) {
  if (!state.clientId) return false;
  if (state.recorderClient) return state.recorderClient === state.clientId;
  return state.granted !== false;
}
