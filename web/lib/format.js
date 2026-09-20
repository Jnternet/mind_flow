// 时间与文本格式化（纯函数，Node 里可测）。

/** 毫秒 → `mm:ss` / `h:mm:ss`。 */
export function clock(ms) {
  const total = Math.max(0, Math.floor((ms ?? 0) / 1000));
  const hours = Math.floor(total / 3600);
  const minutes = Math.floor((total % 3600) / 60);
  const seconds = total % 60;
  const pad = (value) => String(value).padStart(2, "0");
  return hours > 0 ? `${hours}:${pad(minutes)}:${pad(seconds)}` : `${pad(minutes)}:${pad(seconds)}`;
}

/** 下载进度百分比（0-100，total 未知时返回 null）。 */
export function percent(downloaded, total) {
  if (!total || total <= 0) return null;
  return Math.min(100, Math.round((downloaded / total) * 100));
}

/** 人类可读体积。 */
export function humanSize(bytes) {
  if (!bytes) return "0 B";
  const units = ["B", "KB", "MB", "GB"];
  let value = bytes;
  let unit = 0;
  while (value >= 1024 && unit < units.length - 1) {
    value /= 1024;
    unit += 1;
  }
  return `${value.toFixed(value >= 10 || unit === 0 ? 0 : 1)} ${units[unit]}`;
}

/** 段落占位文案。 */
export function pendingLabel(segmentId) {
  return `第 ${segmentId} 段识别中…`;
}
