// 与后端 WS 的协议编解码（纯函数，Node 里可测）。

/** 编码上行控制消息。 */
export function encodeControl(type, extra = {}) {
  return JSON.stringify({ type, ...extra });
}

/** 解析下行事件；不是合法事件时返回 null。 */
export function decodeEvent(raw) {
  if (typeof raw !== "string") return null;
  try {
    const value = JSON.parse(raw);
    if (!value || typeof value.type !== "string") return null;
    return value;
  } catch {
    return null;
  }
}

/** 把句子按时间排序并按 id 去重合并。 */
export function mergeSentences(existing, added) {
  const map = new Map();
  for (const sentence of [...(existing ?? []), ...(added ?? [])]) {
    if (!sentence || typeof sentence.id !== "string") continue;
    map.set(sentence.id, sentence);
  }
  return [...map.values()].sort((a, b) => a.start_ms - b.start_ms || a.id.localeCompare(b.id));
}

/** Float32 [-1,1] → Int16。 */
export function floatToInt16(input) {
  const out = new Int16Array(input.length);
  for (let i = 0; i < input.length; i += 1) {
    const value = Math.max(-1, Math.min(1, input[i]));
    out[i] = value < 0 ? value * 32768 : value * 32767;
  }
  return out;
}

/** 一小段 PCM 的峰值（0-1），用于电平指示。 */
export function peakOf(int16) {
  if (!int16 || int16.length === 0) return 0;
  let peak = 0;
  for (let i = 0; i < int16.length; i += 1) {
    const value = Math.abs(int16[i]);
    if (value > peak) peak = value;
  }
  return peak / 32768;
}
