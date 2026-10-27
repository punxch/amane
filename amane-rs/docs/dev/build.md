# 构建与交付

> 记录构建依赖、命令入口与 CI 范围。具体命令在 `Justfile`。

## 命令入口

根目录 `just rs <recipe>` 委托到 `amane-rs/Justfile`; 在 `amane-rs/` 内直接 `just <recipe>`。

| 命令 | 依赖 Node | 内容 |
|------|-----------|------|
| `just rs check` | 否 | 格式、lint、后端测试 |
| `just rs check-ui` | 是 | 导出契约 → 生成类型 → 前端类型检查 |
| `just rs ci` | 是 | 上述两项依次执行 |

## 构建依赖

- Rust 下限 1.98, 工作区钉在 `rust-toolchain.toml` 指定的版本, CI 使用同一版本。
- `amane-net` 经 `wreq` 使用 BoringSSL (`btls-sys`), 构建需要 cmake 与 C/C++ 工具链; Windows 的 x86_64 目标另需 nasm。BoringSSL 只在本机编译一次, 之后进入增量缓存。
- 前端需要 pnpm。`ui/` 使用 pnpm 12 的 `allowBuilds` 放行 `esbuild` 的构建脚本; 低于 12 的版本会以「Ignored build scripts」中止安装。

## CI 范围

CI 只跑单平台 (Linux) 的 `just rs ci`: 格式、lint、后端测试, 以及基于当前契约的前端类型检查。桌面安装包在本地手工产出, 三平台产物矩阵等首个切片稳定后再加。

Android 交叉编译不进入 CI, 也不在本轮交付范围, 见 [`docs/adr/0004`](../../docs/adr/0004-no-android-client.md)。

## 契约生成

`just rs openapi` 由服务端二进制输出 `ui/openapi.json`; `just rs generate` 用 `openapi-typescript` 生成 `ui/src/api/schema.d.ts`; `ui/src/api/client.ts` 在此之上提供请求封装。三份产物都不入库。

契约漂移的判定方式是**重新生成后做类型检查**, 而不是比对文件差异: 接口改名或删字段后, 引用旧字段的前端代码无法通过 `vue-tsc`, 报错位置即需要同步的端点。生成类型缺失时 `ui/scripts/ensure-schema.mjs` 在 `dev` / `build` / `typecheck` 三个入口给出提示并中止, 不让错误退化成满屏的模块解析失败。

前端在浏览器开发时经 Vite 代理访问 `127.0.0.1:8787`; 桌面端由外壳分配端口后经环境变量注入。
