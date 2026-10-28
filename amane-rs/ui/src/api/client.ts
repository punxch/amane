/**
 * HTTP 客户端.
 *
 * 类型全部来自 `schema.d.ts`, 由服务端契约生成; 本文件只保留请求封装与错误类型.
 * 接口改动后重新运行 `just rs generate`, 前端类型检查会指出需要同步的位置.
 */
import type { components } from "@/api/schema";

type Schemas = components["schemas"];

export type MetadataDto = Schemas["MetadataDto"];
export type MetadataPage = Schemas["MetadataPage"];
export type ExtractedNumber = Schemas["ExtractedNumber"];
export type CreateMetadataRequest = Schemas["CreateMetadataRequest"];
export type ApiErrorBody = Schemas["ErrorBody"];
export type TaskDto = Schemas["TaskDto"];
export type TaskPage = Schemas["TaskPage"];
export type CreateTaskRequest = Schemas["CreateTaskRequest"];
export type TaskLogEntry = Schemas["TaskLogEntry"];
export type TaskLogPage = Schemas["TaskLogPage"];
export type FilterOptions = Schemas["FilterOptions"];
export type MetadataFieldDto = Schemas["MetadataFieldDto"];
export type UpdateMetadataRequest = Schemas["UpdateMetadataRequest"];
export type ScheduleDto = Schemas["ScheduleDto"];
export type SchedulePage = Schemas["SchedulePage"];
export type CreateScheduleRequest = Schemas["CreateScheduleRequest"];
export type UpdateScheduleRequest = Schemas["UpdateScheduleRequest"];
export type ConfigState = Schemas["ConfigState"];
export type SettingsDto = Schemas["SettingsDto"];
export type LibraryDto = Schemas["LibraryDto"];
export type LibraryPage = Schemas["LibraryPage"];
export type LibraryFilePage = Schemas["LibraryFilePage"];
export type CreateLibraryRequest = Schemas["CreateLibraryRequest"];
export type OrganizePreviewDto = Schemas["OrganizePreviewDto"];

/** 保存配置的请求体; 字段与 `Settings` 一致, 由后端拒绝未知取值. */
export interface ConfigBody {
  cold: { data_dir: string; bind: string; api_token: string | null };
  hot: { concurrency: number; request_timeout_secs: number; enabled_sources: string[] };
}

export class ApiError extends Error {
  public readonly code: string;

  public constructor(code: string, message: string) {
    super(message);
    this.name = "ApiError";
    this.code = code;
  }
}

const TOKEN_KEY = "amane_token";

/**
 * 捕获桌面外壳注入的本次进程令牌.
 *
 * 外壳以 `/?token=…` 打开界面; 令牌只存进本次会话, 不落盘.
 */
export function captureToken(): void {
  const token = new URLSearchParams(window.location.search).get("token");
  if (token !== null) {
    window.sessionStorage.setItem(TOKEN_KEY, token);
  }
}

function authHeaders(): Record<string, string> {
  const token = window.sessionStorage.getItem(TOKEN_KEY);
  return token === null ? {} : { authorization: `Bearer ${token}` };
}

async function request<T>(path: string, init?: RequestInit): Promise<T> {
  let response: Response;
  try {
    response = await fetch(path, {
      ...init,
      headers: { "content-type": "application/json", ...authHeaders(), ...init?.headers },
    });
  } catch {
    // 网络层失败没有响应体, 单独给一个错误码, 界面据此提示后端不可用.
    throw new ApiError("unreachable", "无法连接后端服务");
  }

  if (!response.ok) {
    const body: unknown = await response.json().catch(() => null);
    if (isApiErrorBody(body)) {
      throw new ApiError(body.code, body.message);
    }
    throw new ApiError("transport", `请求失败: ${response.status}`);
  }

  // 断言必要性: 响应体由服务端契约保证, 运行时无法在不引入校验库的前提下收窄,
  // 因此把收窄点集中在这一个函数里, 调用方拿到的类型始终来自生成的 schema.
  return (await response.json()) as T;
}

function isApiErrorBody(value: unknown): value is ApiErrorBody {
  if (typeof value !== "object" || value === null) {
    return false;
  }
  // 断言必要性: 入参是未知来源的 JSON, 先证明它是对象, 之后按键取值.
  const candidate = value as Record<string, unknown>;
  return typeof candidate.code === "string" && typeof candidate.message === "string";
}

/** 列表查询. 空字符串与 `null` 都表示不限制. */
export interface MetadataQuery {
  keyword?: string;
  contentType?: string | null;
  source?: string | null;
  limit?: number;
  offset?: number;
}

export const api = {
  list: (query: MetadataQuery = {}): Promise<MetadataPage> => {
    const params = new URLSearchParams({
      limit: String(query.limit ?? 50),
      offset: String(query.offset ?? 0),
    });
    const keyword = query.keyword?.trim();
    if (keyword !== undefined && keyword !== "") {
      params.set("q", keyword);
    }
    if (query.contentType !== null && query.contentType !== undefined && query.contentType !== "") {
      params.set("content_type", query.contentType);
    }
    if (query.source !== null && query.source !== undefined && query.source !== "") {
      params.set("source", query.source);
    }
    return request(`/api/metadata?${params.toString()}`);
  },

  filters: (): Promise<FilterOptions> => request("/api/filters"),

  metadata: (number: string): Promise<MetadataDto> =>
    request(`/api/metadata/${encodeURIComponent(number)}`),

  updateMetadata: (number: string, payload: UpdateMetadataRequest): Promise<MetadataDto> =>
    request(`/api/metadata/${encodeURIComponent(number)}`, {
      method: "PATCH",
      body: JSON.stringify(payload),
    }),

  setLock: (number: string, field: string, locked: boolean): Promise<MetadataDto> =>
    request(`/api/metadata/${encodeURIComponent(number)}/locks/${encodeURIComponent(field)}`, {
      method: "PUT",
      body: JSON.stringify({ locked }),
    }),

  addTag: (number: string, name: string): Promise<MetadataDto> =>
    request(`/api/metadata/${encodeURIComponent(number)}/tags`, {
      method: "POST",
      body: JSON.stringify({ name }),
    }),

  removeTag: (number: string, name: string): Promise<MetadataDto> =>
    request(
      `/api/metadata/${encodeURIComponent(number)}/tags/${encodeURIComponent(name)}`,
      { method: "DELETE" },
    ),

  create: (payload: CreateMetadataRequest): Promise<MetadataDto> =>
    request("/api/metadata", {
      method: "POST",
      body: JSON.stringify(payload),
    }),

  extract: (text: string): Promise<ExtractedNumber> =>
    request(`/api/number/extract?text=${encodeURIComponent(text)}`),

  listTasks: (status: string | null, limit = 50, offset = 0): Promise<TaskPage> => {
    const query = new URLSearchParams({ limit: String(limit), offset: String(offset) });
    if (status !== null) {
      query.set("status", status);
    }
    return request(`/api/tasks?${query.toString()}`);
  },

  createTask: (payload: CreateTaskRequest): Promise<TaskDto> =>
    request("/api/tasks", { method: "POST", body: JSON.stringify(payload) }),

  cancelTask: (id: number): Promise<TaskDto> =>
    request(`/api/tasks/${id}/cancel`, { method: "POST" }),

  retryTask: (id: number): Promise<TaskDto> =>
    request(`/api/tasks/${id}/retry`, { method: "POST" }),

  taskLog: (id: number): Promise<TaskLogPage> => request(`/api/tasks/${id}/log`),

  taskChildren: (id: number): Promise<TaskPage> => request(`/api/tasks/${id}/children`),

  readConfig: (): Promise<SettingsDto> => request("/api/config"),

  saveConfig: (body: ConfigBody): Promise<ConfigState> =>
    request("/api/config", { method: "PUT", body: JSON.stringify(body) }),

  configSchema: (): Promise<unknown> => request("/api/config/schema"),

  listSchedules: (): Promise<SchedulePage> => request("/api/schedules"),

  createSchedule: (payload: CreateScheduleRequest): Promise<ScheduleDto> =>
    request("/api/schedules", { method: "POST", body: JSON.stringify(payload) }),

  updateSchedule: (id: number, payload: UpdateScheduleRequest): Promise<ScheduleDto> =>
    request(`/api/schedules/${id}`, { method: "PATCH", body: JSON.stringify(payload) }),

  deleteSchedule: (id: number): Promise<void> =>
    request(`/api/schedules/${id}`, { method: "DELETE" }),

  listLibraries: (): Promise<LibraryPage> => request("/api/libraries"),

  createLibrary: (payload: CreateLibraryRequest): Promise<LibraryDto> =>
    request("/api/libraries", { method: "POST", body: JSON.stringify(payload) }),

  deleteLibrary: (id: number): Promise<void> =>
    request(`/api/libraries/${id}`, { method: "DELETE" }),

  scanLibrary: (id: number): Promise<TaskDto> =>
    request(`/api/libraries/${id}/scan`, { method: "POST" }),

  libraryFiles: (id: number, kind: string | null): Promise<LibraryFilePage> => {
    const query = new URLSearchParams();
    if (kind !== null) {
      query.set("kind", kind);
    }
    const suffix = query.size === 0 ? "" : `?${query.toString()}`;
    return request(`/api/libraries/${id}/files${suffix}`);
  },

  organizePreview: (id: number): Promise<OrganizePreviewDto> =>
    request(`/api/libraries/${id}/organize/preview`, { method: "POST" }),

  organizeExecute: (id: number, choices: Record<string, string>): Promise<TaskDto> =>
    request(`/api/libraries/${id}/organize/execute`, {
      method: "POST",
      body: JSON.stringify({ choices }),
    }),
};
