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

export class ApiError extends Error {
  public readonly code: string;

  public constructor(code: string, message: string) {
    super(message);
    this.name = "ApiError";
    this.code = code;
  }
}

async function request<T>(path: string, init?: RequestInit): Promise<T> {
  const response = await fetch(path, {
    ...init,
    headers: { "content-type": "application/json", ...init?.headers },
  });

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

export const api = {
  list: (limit = 50, offset = 0): Promise<MetadataPage> =>
    request(`/api/metadata?limit=${limit}&offset=${offset}`),

  create: (payload: CreateMetadataRequest): Promise<MetadataDto> =>
    request("/api/metadata", {
      method: "POST",
      body: JSON.stringify(payload),
    }),

  extract: (text: string): Promise<ExtractedNumber> =>
    request(`/api/number/extract?text=${encodeURIComponent(text)}`),
};
