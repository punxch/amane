/**
 * 生成的前端类型缺失时给出明确提示.
 *
 * 类型由服务端契约生成, 不入库; 直接运行 vite 或 vue-tsc 会得到一串无法定位的模块错误,
 * 因此在入口先检查一次.
 */
import { existsSync } from "node:fs";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";

const target = join(dirname(fileURLToPath(import.meta.url)), "..", "src", "api", "schema.d.ts");

if (!existsSync(target)) {
  console.error(
    "缺少由 OpenAPI 生成的前端类型.\n在仓库根目录运行 `just rs generate`; 它先导出契约再生成类型.",
  );
  process.exit(1);
}
