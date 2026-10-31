/**
 * 展示用格式化.
 *
 * 时间统一按 `zh-CN` 与 24 小时制输出; 界面上的时间都来自服务端的 UTC 时间戳, 由这里换算成本地时区.
 */

/** 完整时刻; 缺失时给破折号而不是空串, 否则表格里会留下看起来像加载失败的空白. */
export function formatTime(value: string | null | undefined): string {
  if (value === null || value === undefined || value === "") {
    return "—";
  }
  return new Date(value).toLocaleString("zh-CN", { hour12: false });
}

/** 只取时分秒, 用于任务日志的逐行时间. */
export function formatClock(value: string | null | undefined): string {
  if (value === null || value === undefined || value === "") {
    return "—";
  }
  return new Date(value).toLocaleTimeString("zh-CN", { hour12: false });
}

/** 日期; 缺失时同样给破折号. */
export function formatDate(value: string | null | undefined): string {
  if (value === null || value === undefined || value === "") {
    return "—";
  }
  return new Date(value).toLocaleDateString("zh-CN");
}

/** 文件大小. 保留两位有效小数, 超过 1 GB 后改用 GB. */
export function formatSize(bytes: number): string {
  if (bytes < 1024) {
    return `${bytes} B`;
  }
  if (bytes < 1024 * 1024) {
    return `${(bytes / 1024).toFixed(1)} KB`;
  }
  if (bytes < 1024 * 1024 * 1024) {
    return `${(bytes / 1024 / 1024).toFixed(1)} MB`;
  }
  return `${(bytes / 1024 / 1024 / 1024).toFixed(2)} GB`;
}

/** 分页页码: 服务端给总数与偏移, 页面自己换算当前页. */
export function pageCount(total: number, size: number): number {
  return Math.max(1, Math.ceil(total / size));
}
