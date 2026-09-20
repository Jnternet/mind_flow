import assert from "node:assert/strict";
import { test } from "node:test";

import {
  isEditableTarget,
  shouldForceStop,
  shouldStartRecording,
  shouldStopRecording,
} from "../../web/lib/hotkey.js";

const 空格 = (extra = {}) => ({ code: "Space", key: " ", repeat: false, ...extra });
const 字母 = (extra = {}) => ({ code: "KeyA", key: "a", repeat: false, ...extra });

test("空格在非编辑状态下开始录音", () => {
  assert.equal(shouldStartRecording(空格(), { editing: false }), true);
});

test("正在编辑文字时空格让位给输入", () => {
  assert.equal(shouldStartRecording(空格(), { editing: true }), false);
});

test("长按的重复事件不重复触发", () => {
  assert.equal(shouldStartRecording(空格({ repeat: true }), { editing: false }), false);
});

test("带修饰键不算按住说话", () => {
  for (const key of ["ctrlKey", "altKey", "metaKey", "shiftKey"]) {
    assert.equal(shouldStartRecording(空格({ [key]: true }), { editing: false }), false, key);
  }
});

test("非空格键与不可见页面不触发", () => {
  assert.equal(shouldStartRecording(字母(), { editing: false }), false);
  assert.equal(shouldStartRecording(空格(), { editing: false, visible: false }), false);
  assert.equal(shouldStartRecording(null, { editing: false }), false);
});

test("松开空格停止", () => {
  assert.equal(shouldStopRecording(空格()), true);
  assert.equal(shouldStopRecording(字母()), false);
  assert.equal(shouldStopRecording(null), false);
});

test("失焦或页面隐藏要强制收尾", () => {
  assert.equal(shouldForceStop({ visible: false }), true);
  assert.equal(shouldForceStop({ focused: false }), true);
  assert.equal(shouldForceStop({}), false);
});

test("识别可编辑元素", () => {
  assert.equal(isEditableTarget({ tagName: "INPUT" }), true);
  assert.equal(isEditableTarget({ tagName: "TEXTAREA" }), true);
  assert.equal(isEditableTarget({ tagName: "DIV", isContentEditable: true }), true);
  assert.equal(isEditableTarget({ tagName: "DIV" }), false);
  assert.equal(isEditableTarget(null), false);
});
