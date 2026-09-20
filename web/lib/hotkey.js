// 空格按住录音的判定规则（纯函数，Node 里可测）。

const EDITABLE_TAGS = new Set(["INPUT", "TEXTAREA", "SELECT"]);

/** 焦点是否在可编辑元素里。 */
export function isEditableTarget(target) {
  if (!target) return false;
  if (target.isContentEditable) return true;
  return EDITABLE_TAGS.has(target.tagName);
}

function isSpace(event) {
  return event.code === "Space" || event.key === " " || event.key === "Spacebar";
}

/**
 * 是否应该开始录音。
 * 编辑文字时空格要留给输入（中文输入法也要用空格选词），所以直接放行。
 */
export function shouldStartRecording(event, { editing = false, visible = true } = {}) {
  if (!event || !isSpace(event)) return false;
  if (event.repeat) return false;
  if (event.ctrlKey || event.altKey || event.metaKey || event.shiftKey) return false;
  if (editing) return false;
  if (!visible) return false;
  return true;
}

/** 是否应该停止录音（松开空格）。 */
export function shouldStopRecording(event) {
  return Boolean(event) && isSpace(event);
}

/** 需要强制收尾的情况：失焦 / 页面不可见。 */
export function shouldForceStop({ visible = true, focused = true } = {}) {
  return !visible || !focused;
}
