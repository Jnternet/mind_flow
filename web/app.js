// 界面接线：麦克风采集、WS、渲染、播放器、编辑。

import { clock, humanSize, pendingLabel, percent } from "/lib/format.js";
import {
  isEditableTarget,
  shouldForceStop,
  shouldStartRecording,
  shouldStopRecording,
} from "/lib/hotkey.js";
import { decodeEvent, encodeControl, peakOf } from "/lib/protocol.js";
import { applyEvent, canRecord, initialState } from "/lib/state.js";

// AudioWorklet 代码内联成字符串，保证整个前端只有内嵌资源、没有额外文件请求。
const WORKLET = `
class MindFlowCapture extends AudioWorkletProcessor {
  process(inputs) {
    const channel = inputs[0] && inputs[0][0];
    if (channel) {
      const out = new Int16Array(channel.length);
      for (let i = 0; i < channel.length; i += 1) {
        const value = Math.max(-1, Math.min(1, channel[i]));
        out[i] = value < 0 ? value * 32768 : value * 32767;
      }
      this.port.postMessage(out.buffer, [out.buffer]);
    }
    return true;
  }
}
registerProcessor("mind-flow-capture", MindFlowCapture);
`;

const el = {
  stateText: document.getElementById("state-text"),
  hint: document.getElementById("hint"),
  recDot: document.getElementById("rec-dot"),
  chipDevice: document.getElementById("chip-device"),
  chipPending: document.getElementById("chip-pending"),
  chipCount: document.getElementById("chip-count"),
  level: document.getElementById("level").firstElementChild,
  banner: document.getElementById("model-banner"),
  bannerTitle: document.getElementById("banner-title"),
  bannerModelDir: document.getElementById("banner-model-dir"),
  modelText: document.getElementById("model-text"),
  download: document.getElementById("btn-download"),
  rescan: document.getElementById("btn-rescan"),
  openDiagnostics: document.getElementById("btn-open-diagnostics"),
  toast: document.getElementById("toast"),
  sentences: document.getElementById("sentences"),
  emptyTip: document.getElementById("empty-tip"),
  play: document.getElementById("btn-play"),
  seek: document.getElementById("seek"),
  timeNow: document.getElementById("time-now"),
  timeTotal: document.getElementById("time-total"),
  audio: document.getElementById("audio"),
  title: document.getElementById("title"),
  finish: document.getElementById("btn-finish"),
  discard: document.getElementById("btn-discard"),
  drawer: document.getElementById("drawer"),
  settings: document.getElementById("btn-settings"),
  closeDrawer: document.getElementById("btn-close-drawer"),
  hold: document.getElementById("btn-hold"),
  inference: document.getElementById("inference"),
  engineDetail: document.getElementById("engine-detail"),
  appVersion: document.getElementById("app-version"),
  dataDir: document.getElementById("data-dir"),
  modelDir: document.getElementById("model-dir"),
  modelMissing: document.getElementById("model-missing"),
  rowMissing: document.getElementById("row-missing"),
  rowError: document.getElementById("row-error"),
  lastError: document.getElementById("last-error"),
  diagnostics: document.getElementById("btn-diagnostics"),
  modelStatus: document.getElementById("model-status"),
};

let state = initialState();
let socket = null;
let audioContext = null;
let captureNode = null;
let mediaStream = null;
let recording = false;
let uploadedVersion = -1;

// ---------- 渲染 ----------

function render() {
  const label = state.recording ? "正在录音…" : state.pending > 0 ? "识别中…" : "准备就绪";
  el.stateText.textContent = label;
  el.recDot.classList.toggle("on", state.recording);
  el.hint.textContent = state.recording
    ? "松开空格结束这一句"
    : canRecord(state)
      ? "按住空格开始说话，松开出字"
      : "另一个标签页正在录音，本页只读";
  el.chipDevice.textContent =
    state.provider === "cuda" ? `CUDA · ${state.device}` : state.device || "CPU";
  el.chipDevice.title = state.reason || el.chipDevice.textContent;
  el.chipPending.hidden = state.pending === 0;
  el.chipPending.textContent = `识别中 ${state.pending}`;
  el.chipCount.textContent = `${state.sentences.length} 句`;
  el.dataDir.textContent = state.dataDir || "-";
  el.modelDir.textContent = state.modelDir || "-";
  el.bannerModelDir.textContent = state.modelDir || "-";
  el.engineDetail.textContent = `${state.device}（${state.reason || "内置引擎"}）`;
  el.appVersion.textContent = state.version || "-";
  el.modelStatus.textContent = state.modelReady
    ? state.modelExtrasReady
      ? "已就绪"
      : "识别可用（标点/VAD 缺失）"
    : state.modelDownloading
      ? "下载中…"
      : "缺失，需要下载";
  // 三种情况都要看得见：缺文件 / 文件在但引擎装载失败 / 还没就绪
  const missing = state.modelMissing ?? [];
  const bannerNeeded =
    !state.modelDownloading && (!state.modelReady || Boolean(state.lastError));
  el.banner.hidden = !bannerNeeded;
  if (state.modelDownloading) {
    el.bannerTitle.textContent = "正在下载模型";
  } else if (missing.length > 0) {
    el.bannerTitle.textContent = "缺少模型文件";
  } else if (state.lastError) {
    el.bannerTitle.textContent = "模型读不进来";
  } else {
    el.bannerTitle.textContent = "模型未就绪";
  }
  if (!state.modelDownloading) {
    el.modelText.textContent =
      missing.length > 0
        ? `缺少：${missing.join(" · ")}`
        : state.lastError || "点「重新检查」重试；仍不行就打开诊断信息发我";
  }
  el.rowMissing.hidden = missing.length === 0;
  el.modelMissing.textContent = missing.join(" · ") || "-";
  el.rowError.hidden = !state.lastError;
  el.lastError.textContent = state.lastError || "-";
  if (state.modelDownloading && state.modelProgress) {
    const value = percent(state.modelProgress.downloaded, state.modelProgress.total);
    const size = `${humanSize(state.modelProgress.downloaded)} / ${humanSize(state.modelProgress.total)}`;
    el.modelText.textContent = `${state.modelProgress.file} ${size}${
      value === null ? "" : `（${value}%）`
    }`;
  }
  el.toast.hidden = !state.toast;
  if (state.toast) el.toast.textContent = state.toast;
  if (document.activeElement !== el.title) el.title.value = state.title;
  el.finish.disabled = !state.session;
  renderSentences();
  refreshAudio();
}

function renderSentences() {
  const active = currentSentenceId();
  el.emptyTip.hidden = state.sentences.length > 0;
  const wanted = new Set(state.sentences.map((sentence) => sentence.id));
  for (const node of [...el.sentences.children]) {
    if (node.classList.contains("pending-row")) continue;
    if (!wanted.has(node.dataset.id)) node.remove();
  }
  for (const sentence of state.sentences) {
    let node = el.sentences.querySelector(`li[data-id="${sentence.id}"]`);
    if (!node) {
      node = document.createElement("li");
      node.dataset.id = sentence.id;
      const stamp = document.createElement("button");
      stamp.type = "button";
      stamp.className = "stamp";
      stamp.addEventListener("click", () => jumpTo(sentence));
      const text = document.createElement("div");
      text.className = "text";
      text.contentEditable = "true";
      text.spellcheck = false;
      text.addEventListener("focus", () => {
        document.body.dataset.editing = "1";
      });
      text.addEventListener("blur", async () => {
        document.body.dataset.editing = "";
        const value = text.textContent.trim();
        if (value !== sentence.text) await saveSentence(sentence.id, value);
      });
      text.addEventListener("keydown", (event) => {
        if (event.key === "Enter" && !event.shiftKey) {
          event.preventDefault();
          text.blur();
        } else if (event.key === "Escape") {
          text.textContent = sentence.text;
          text.blur();
        }
      });
      node.append(stamp, text);
      el.sentences.append(node);
    }
    const stamp = node.querySelector(".stamp");
    if (stamp.textContent !== clock(sentence.start_ms)) stamp.textContent = clock(sentence.start_ms);
    const text = node.querySelector(".text");
    if (document.activeElement !== text && text.textContent !== sentence.text) {
      text.textContent = sentence.text;
    }
    text.classList.toggle("empty", sentence.text.trim().length === 0);
    node.classList.toggle("active", active === sentence.id);
  }
  el.sentences.querySelectorAll("li.pending-row").forEach((node) => node.remove());
  if (state.pending > 0) {
    const node = document.createElement("li");
    node.className = "pending-row";
    const span = document.createElement("span");
    span.className = "pending";
    span.textContent = pendingLabel(state.segments.length + 1);
    node.append(span);
    el.sentences.append(node);
  }
}

function currentSentenceId() {
  const now = el.audio.currentTime * 1000;
  let id = null;
  for (const sentence of state.sentences) {
    if (sentence.start_ms <= now + 1) id = sentence.id;
    else break;
  }
  return id;
}

function refreshAudio() {
  if (state.audioVersion === uploadedVersion) return;
  if (!state.session || state.audioVersion === 0) return;
  uploadedVersion = state.audioVersion;
  const wasPlaying = !el.audio.paused;
  const position = el.audio.currentTime;
  el.audio.src = `/api/sessions/current/audio?v=${state.audioVersion}`;
  el.audio.addEventListener(
    "loadedmetadata",
    () => {
      if (position > 0 && position < el.audio.duration) el.audio.currentTime = position;
      if (wasPlaying) void el.audio.play();
    },
    { once: true },
  );
}

function jumpTo(sentence) {
  if (state.audioVersion !== uploadedVersion) refreshAudio();
  const target = sentence.start_ms / 1000;
  const apply = () => {
    el.audio.currentTime = target;
    void el.audio.play();
  };
  if (el.audio.readyState >= 1) apply();
  else el.audio.addEventListener("loadedmetadata", apply, { once: true });
}

async function saveSentence(id, text) {
  try {
    await fetch(`/api/sentences/${encodeURIComponent(id)}`, {
      method: "PATCH",
      headers: { "content-type": "application/json" },
      body: JSON.stringify({ text }),
    });
  } catch (error) {
    console.error("保存失败", error);
  }
}

// ---------- WebSocket ----------

function connect(rate) {
  const protocol = location.protocol === "https:" ? "wss" : "ws";
  socket = new WebSocket(`${protocol}://${location.host}/api/ws?rate=${rate}`);
  socket.addEventListener("message", (message) => {
    if (typeof message.data !== "string") return;
    const event = decodeEvent(message.data);
    if (!event) return;
    state = applyEvent(state, event);
    render();
    if (event.type === "error") {
      window.setTimeout(() => {
        state = { ...state, toast: null };
        render();
      }, 4000);
    }
  });
  socket.addEventListener("open", () => socket.send(encodeControl("claim_recorder")));
  socket.addEventListener("close", () => window.setTimeout(() => connect(rate), 1500));
}

function sendControl(type) {
  if (socket && socket.readyState === WebSocket.OPEN) socket.send(encodeControl(type));
}

// ---------- 麦克风 ----------

async function ensureMic() {
  if (captureNode) return;
  mediaStream = await navigator.mediaDevices.getUserMedia({
    audio: { channelCount: 1, echoCancellation: true, noiseSuppression: true },
  });
  audioContext = new AudioContext({ sampleRate: 16000 });
  const blob = new Blob([WORKLET], { type: "application/javascript" });
  await audioContext.audioWorklet.addModule(URL.createObjectURL(blob));
  captureNode = new AudioWorkletNode(audioContext, "mind-flow-capture");
  captureNode.port.onmessage = (message) => {
    const pcm = new Int16Array(message.data);
    if (recording && socket && socket.readyState === WebSocket.OPEN) socket.send(pcm.buffer);
    el.level.style.width = `${Math.min(100, peakOf(pcm) * 140)}%`;
  };
  const source = audioContext.createMediaStreamSource(mediaStream);
  source.connect(captureNode);
  const silent = audioContext.createGain();
  silent.gain.value = 0;
  captureNode.connect(silent).connect(audioContext.destination);
  if (audioContext.state === "suspended") await audioContext.resume();
}

async function startRecording() {
  if (recording || !canRecord(state)) return;
  try {
    await ensureMic();
  } catch (error) {
    state = { ...state, toast: `麦克风不可用：${error.message}` };
    render();
    return;
  }
  recording = true;
  state = { ...state, recording: true };
  sendControl("start");
  render();
}

function stopRecording() {
  if (!recording) return;
  recording = false;
  state = { ...state, recording: false };
  sendControl("stop");
  el.level.style.width = "0%";
  render();
}

// ---------- 按键与指针 ----------

window.addEventListener("keydown", (event) => {
  const editing = isEditableTarget(event.target);
  if (shouldStartRecording(event, { editing, visible: document.visibilityState === "visible" })) {
    event.preventDefault();
    void startRecording();
  }
});

window.addEventListener("keyup", (event) => {
  if (shouldStopRecording(event)) stopRecording();
});

window.addEventListener("blur", () => {
  if (shouldForceStop({ focused: false })) stopRecording();
});

document.addEventListener("visibilitychange", () => {
  if (shouldForceStop({ visible: document.visibilityState === "visible" })) stopRecording();
});

document.addEventListener("pointerup", () => stopRecording());
document.addEventListener("pointercancel", () => stopRecording());
el.hold.addEventListener("pointerdown", (event) => {
  event.preventDefault();
  void startRecording();
});
el.hold.addEventListener("pointerleave", () => stopRecording());

// ---------- 播放器 ----------

el.play.addEventListener("click", () => {
  if (el.audio.paused) void el.audio.play();
  else el.audio.pause();
});
el.audio.addEventListener("play", () => {
  el.play.textContent = "❚❚";
});
el.audio.addEventListener("pause", () => {
  el.play.textContent = "▶";
});
el.audio.addEventListener("timeupdate", () => {
  el.timeNow.textContent = clock(el.audio.currentTime * 1000);
  el.seek.value = String(Math.floor(el.audio.currentTime * 1000));
  renderSentences();
});
el.audio.addEventListener("loadedmetadata", () => {
  const duration = Number.isFinite(el.audio.duration) ? el.audio.duration * 1000 : 0;
  el.seek.max = String(Math.floor(duration));
  el.timeTotal.textContent = clock(duration);
});
el.seek.addEventListener("input", () => {
  el.audio.currentTime = Number(el.seek.value) / 1000;
});

// ---------- 按钮 ----------

el.finish.addEventListener("click", async () => {
  if (!state.session) return;
  el.finish.disabled = true;
  try {
    const response = await fetch("/api/session/finalize", {
      method: "POST",
      headers: { "content-type": "application/json" },
      body: JSON.stringify({ title: el.title.value }),
    });
    const payload = await response.json();
    if (!response.ok) throw new Error(payload.message ?? "保存失败");
    resetSession(`已保存 ${payload.files.length} 个文件到 ${state.recordingsDir}`);
  } catch (error) {
    state = { ...state, toast: `保存失败：${error.message}` };
    render();
  } finally {
    el.finish.disabled = false;
  }
});

el.discard.addEventListener("click", async () => {
  if (!state.session) return;
  if (!window.confirm("丢弃这次会话？已录的音频和文字都会被删除。")) return;
  await fetch("/api/session/discard", { method: "POST" });
  resetSession("已丢弃本次会话");
});

function resetSession(message) {
  state = {
    ...state,
    toast: message,
    sentences: [],
    segments: [],
    session: null,
    audioVersion: 0,
    pending: 0,
  };
  uploadedVersion = -1;
  el.audio.removeAttribute("src");
  el.audio.load();
  render();
}

el.download.addEventListener("click", async () => {
  await fetch("/api/models/download", { method: "POST" });
  state = { ...state, modelDownloading: true };
  render();
});

el.rescan.addEventListener("click", async () => {
  el.modelText.textContent = "正在重新检查…";
  const response = await fetch("/api/models/rescan", { method: "POST" });
  const result = await response.json();
  if (!response.ok) {
    state = { ...state, toast: "重新检查失败" };
  } else if (result.ready) {
    state = { ...state, toast: "模型已就绪", modelReady: true, modelMissing: [] };
  } else {
    state = {
      ...state,
      toast: `仍然缺：${(result.missing ?? []).join(" · ")}`,
      modelMissing: result.missing ?? [],
    };
  }
  render();
});

el.openDiagnostics.addEventListener("click", () => {
  window.open("/api/diagnostics", "_blank");
});
el.diagnostics.addEventListener("click", () => {
  window.open("/api/diagnostics", "_blank");
});

el.settings.addEventListener("click", () => {
  el.drawer.hidden = !el.drawer.hidden;
});
el.closeDrawer.addEventListener("click", () => {
  el.drawer.hidden = true;
});
el.inference.addEventListener("change", async () => {
  await fetch("/api/config", {
    method: "POST",
    headers: { "content-type": "application/json" },
    body: JSON.stringify({ inference: el.inference.value }),
  });
});

async function boot() {
  const response = await fetch("/api/state");
  const status = await response.json();
  // 当前会话的句子/段落（刷新页面后靠它恢复界面）
  const session = await fetch("/api/session").then((r) => r.json());
  let rate = 16000;
  try {
    const probe = new AudioContext({ sampleRate: 16000 });
    rate = probe.sampleRate;
    await probe.close();
  } catch {
    rate = 48000;
  }
  state = applyEvent(state, { type: "status", ...status });
  if (session) {
    state = {
      ...state,
      session: session.id,
      title: session.title || state.title,
      sentences: session.sentences ?? [],
      segments: session.segments ?? [],
      audioVersion: session.audio_version ?? 0,
      duration: session.duration_ms ?? 0,
    };
  }
  render();
  connect(rate);
}

void boot();
