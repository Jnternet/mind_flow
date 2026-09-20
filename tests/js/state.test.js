import assert from "node:assert/strict";
import { test } from "node:test";

import { applyEvent, canRecord, initialState } from "../../web/lib/state.js";

test("status 事件刷新设备与模型状态", () => {
  const state = applyEvent(initialState(), {
    type: "status",
    session: "abc",
    pending_jobs: 2,
    audio_version: 3,
    provider: "cuda",
    device: "RTX 4060",
    engine_reason: "启动",
    model_ready: true,
    model_extras_ready: false,
    model_downloading: false,
    data_dir: "/tmp/data",
    recordings_dir: "/tmp/data/recordings",
    title: "语音笔记",
    recording: true,
    recorder_client: "c-1",
  });
  assert.equal(state.session, "abc");
  assert.equal(state.pending, 2);
  assert.equal(state.audioVersion, 3);
  assert.equal(state.provider, "cuda");
  assert.equal(state.device, "RTX 4060");
  assert.equal(state.modelReady, true);
  assert.equal(state.modelExtrasReady, false);
  assert.equal(state.recording, true);
});

test("sentences_added 追加句子并推进音频版本", () => {
  const base = applyEvent(initialState(), { type: "status", audio_version: 1 });
  const state = applyEvent(base, {
    type: "sentences_added",
    audio_version: 2,
    sentences: [{ id: "s-1-1", start_ms: 0, end_ms: 900, text: "你好。" }],
  });
  assert.equal(state.sentences.length, 1);
  assert.equal(state.audioVersion, 2);
});

test("错误事件变成提示，状态事件清掉提示", () => {
  const withError = applyEvent(initialState(), { type: "error", message: "识别失败" });
  assert.equal(withError.toast, "识别失败");
  const cleared = applyEvent({ ...withError, toast: null }, { type: "status" });
  assert.equal(cleared.toast, null);
});

test("模型进度事件记录进度", () => {
  const state = applyEvent(initialState(), {
    type: "model_progress",
    file: "model.int8.onnx",
    downloaded: 100,
    total: 400,
    finished: false,
  });
  assert.deepEqual(state.modelProgress, {
    file: "model.int8.onnx",
    downloaded: 100,
    total: 400,
    finished: false,
  });
});

test("status 里的模型目录/缺失清单/错误会被记住", () => {
  const state = applyEvent(initialState(), {
    type: "status",
    model_ready: false,
    model_dir: "D:/mind_flow/data/models",
    model_missing: ["paraformer-zh-2023-09-14-int8/model.int8.onnx"],
    last_error: "装载识别引擎失败：模型目录不存在",
  });
  assert.equal(state.modelDir, "D:/mind_flow/data/models");
  assert.deepEqual(state.modelMissing, ["paraformer-zh-2023-09-14-int8/model.int8.onnx"]);
  assert.equal(state.lastError, "装载识别引擎失败：模型目录不存在");
});

test("模型就绪后错误会被清掉", () => {
  const broken = applyEvent(initialState(), { type: "status", last_error: "出错了" });
  const fixed = applyEvent(broken, { type: "status", model_ready: true, last_error: null });
  assert.equal(fixed.lastError, null);
  assert.equal(fixed.modelReady, true);
});

test("录音权：第一个标签独占", () => {
  const waiting = applyEvent(initialState(), { type: "hello", client_id: "c-2", recorder: false });
  assert.equal(canRecord(waiting), false, "没有录音权时不能录");
  const holder = applyEvent(waiting, { type: "status", recorder_client: "c-1" });
  assert.equal(canRecord(holder), false);
  const own = applyEvent(holder, { type: "status", recorder_client: "c-2" });
  assert.equal(canRecord(own), true);
  const free = applyEvent(own, { type: "status", recorder_client: null });
  assert.equal(canRecord(free), true, "没人持有时可以接过来");
});

test("未知事件不影响状态", () => {
  const before = initialState();
  assert.equal(applyEvent(before, { type: "什么鬼" }), before);
  assert.equal(applyEvent(before, null), before);
});
