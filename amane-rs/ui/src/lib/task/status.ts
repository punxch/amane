/**
 * 任务状态与类型的展示用取值.
 *
 * 后端是开放字符串: 未知取值原样显示, 不静默当成「完成」.
 */

export type BadgeTone = "brand" | "success" | "warning" | "danger" | "gray";

export interface TaskStatusDisplay {
  label: string;
  tone: BadgeTone;
}

const STATUS: Record<string, TaskStatusDisplay> = {
  queued: { label: "排队", tone: "gray" },
  running: { label: "运行中", tone: "warning" },
  done: { label: "完成", tone: "success" },
  failed: { label: "失败", tone: "danger" },
  cancelled: { label: "已取消", tone: "gray" },
};

export function taskStatus(status: string): TaskStatusDisplay {
  return STATUS[status] ?? { label: status, tone: "gray" };
}

/** 终态: 不再变化, 因此不再轮询日志. */
export function isTerminalStatus(status: string): boolean {
  return status === "done" || status === "failed" || status === "cancelled";
}

/** 只有失败与取消可以重试; 完成的任务重试等于重复一次副作用. */
export function isRetryableStatus(status: string): boolean {
  return status === "failed" || status === "cancelled";
}

/** 取值与 `TaskKind::as_str` 一致. */
const KIND_LABELS: Record<string, string> = {
  scrape: "刮削",
  assets: "资源下载",
  scan: "扫描",
  organize: "整理",
  refresh: "刷新",
  cleanup: "清理",
};

export function taskKind(kind: string): string {
  return KIND_LABELS[kind] ?? kind;
}

/** 状态筛选的可选项; 与后端 `TaskStatus` 的取值一致. */
export const TASK_STATUS_FILTERS: readonly { value: string; label: string }[] = [
  { value: "queued", label: "排队" },
  { value: "running", label: "运行中" },
  { value: "done", label: "完成" },
  { value: "failed", label: "失败" },
  { value: "cancelled", label: "已取消" },
];
