import assert from "node:assert/strict";
import { test } from "node:test";

import { clock, humanSize, pendingLabel, percent } from "../../web/lib/format.js";

test("时间戳格式化", () => {
  assert.equal(clock(0), "00:00");
  assert.equal(clock(1500), "00:01");
  assert.equal(clock(65_000), "01:05");
  assert.equal(clock(3_665_000), "1:01:05");
  assert.equal(clock(-5), "00:00");
  assert.equal(clock(undefined), "00:00");
});

test("下载进度百分比", () => {
  assert.equal(percent(50, 100), 50);
  assert.equal(percent(200, 100), 100);
  assert.equal(percent(1, 0), null);
  assert.equal(percent(1, undefined), null);
});

test("体积显示", () => {
  assert.equal(humanSize(0), "0 B");
  assert.equal(humanSize(512), "512 B");
  assert.equal(humanSize(1536), "1.5 KB");
  assert.equal(humanSize(243_371_218), "232 MB");
});

test("待识别占位文案", () => {
  assert.equal(pendingLabel(2), "第 2 段识别中…");
});
