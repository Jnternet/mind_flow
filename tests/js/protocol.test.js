import assert from "node:assert/strict";
import { test } from "node:test";

import {
  decodeEvent,
  encodeControl,
  floatToInt16,
  mergeSentences,
  peakOf,
} from "../../web/lib/protocol.js";

test("控制消息编码成 JSON", () => {
  assert.equal(encodeControl("start"), '{"type":"start"}');
  assert.equal(encodeControl("stop", { reason: "blur" }), '{"type":"stop","reason":"blur"}');
});

test("解码下行事件并拒绝脏数据", () => {
  assert.equal(decodeEvent('{"type":"status"}').type, "status");
  assert.equal(decodeEvent("不是 JSON"), null);
  assert.equal(decodeEvent("[]"), null);
  assert.equal(decodeEvent('{"no_type":1}'), null);
  assert.equal(decodeEvent(null), null);
});

test("句子按时间合并去重", () => {
  const existing = [
    { id: "s-1-1", start_ms: 0, end_ms: 100, text: "一" },
    { id: "s-1-2", start_ms: 200, end_ms: 300, text: "二" },
  ];
  const added = [
    { id: "s-1-1", start_ms: 0, end_ms: 150, text: "一改" },
    { id: "s-2-1", start_ms: 900, end_ms: 1000, text: "三" },
  ];
  const merged = mergeSentences(existing, added);
  assert.equal(merged.length, 3);
  assert.equal(merged[0].text, "一改", "同 id 应被后来的覆盖");
  assert.deepEqual(
    merged.map((item) => item.id),
    ["s-1-1", "s-1-2", "s-2-1"],
  );
});

test("Float32 转 Int16 不削顶", () => {
  const out = floatToInt16(new Float32Array([0, 1, -1, 0.5, -0.5, 2, -2]));
  assert.deepEqual([...out], [0, 32767, -32768, 16383, -16384, 32767, -32768]);
});

test("峰值用于电平指示", () => {
  assert.equal(peakOf(new Int16Array([])), 0);
  assert.equal(peakOf(new Int16Array([0, 0])), 0);
  assert.equal(peakOf(new Int16Array([16384, -100])), 0.5);
  assert.equal(peakOf(new Int16Array([-32768])), 1);
});
