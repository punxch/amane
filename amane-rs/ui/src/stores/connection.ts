/**
 * 后端连通状态.
 *
 * 只由 `/api/health` 的探测结果驱动, 供顶栏指示点与版本号使用; 与页面请求的失败无关 ——
 * 单个页面报错不该把整个顶栏标成断开.
 */
import { reactive } from "vue";

import { api } from "@/api/client";

export type ConnectionStatus = "checking" | "connected" | "unreachable";

export interface ConnectionState {
  status: ConnectionStatus;
  version: string | null;
}

/** 探测间隔. 只影响顶栏状态, 不阻塞任何页面. */
const HEALTH_POLL_MS = 15000;

export const connection = reactive<ConnectionState>({ status: "checking", version: null });

let timer: number | undefined;

async function probe(): Promise<void> {
  try {
    const health = await api.health();
    connection.version = health.version;
    connection.status = "connected";
  } catch {
    connection.status = "unreachable";
  }
}

/** 启动轮询; 重复调用不会叠加定时器. */
export function startHealthPolling(): void {
  if (timer !== undefined) {
    return;
  }
  void probe();
  timer = window.setInterval(() => void probe(), HEALTH_POLL_MS);
}

export function stopHealthPolling(): void {
  if (timer !== undefined) {
    window.clearInterval(timer);
    timer = undefined;
  }
}
