# AGENTS.md

本文件为在 `amane-rs/` 内工作的 Coding Agent 提供引导.

## 用语规范

**最高优先级.** 注释、文档、提交说明、与维护者的对话遵守根仓库的 [`docs/dev/writing.md`](../docs/dev/writing.md) (遣词造句) 与 [`docs/dev/terms.md`](../docs/dev/terms.md) (项目用词). 与本文其它条款冲突时以那两份为准.

## 定位

Amane 的 Rust 实现, 逐步替代 `src/amane/` 的 Python 实现. 覆盖范围与推进顺序见 [`docs/adr/0001`](../docs/adr/0001-rust-rewrite-supersedes-python.md).

技术栈: Rust · axum · SQLite (sqlx) · Vue 3 + Vite + Tailwind · Tauri (未接入).

## 目录

| 路径 | 边界 |
|------|------|
| `crates/core/` | 业务内核: 番号、条目、配置、持久化. 不依赖传输层 |
| `crates/crawlers/` | 来源契约与登记表. 站点实现编译进二进制, 无插件宿主 |
| `crates/server/` | axum 适配: 路由、DTO、错误映射、OpenAPI |
| `ui/` | Vue 3 前端. 只经 HTTP 契约访问后端 |
| `migrations/` | sqlx 迁移 |

`crates/desktop/` 在接入 Tauri 的切片中加入, 加入前不进入工作区成员.

## 命令

对外入口是根目录的 `just rs <recipe>`; 在 `amane-rs/` 内可直接 `just <recipe>`.

```bash
just rs install     # 安装前端依赖
just rs dev         # 启动 API (:8787)
just rs ui          # 启动前端开发服务器
just rs check       # fmt + clippy + test
just rs fix         # 自动修复
just rs openapi     # 导出 OpenAPI 到 ui/openapi.json
just rs generate    # 由 OpenAPI 生成 TS 类型
just rs build       # 构建服务端与前端产物
```

## 规范

- **类型安全第一.** 所有函数与结构体具备完整类型标注; 跨层传递用结构体而不是 `serde_json::Value`.
- **禁止 `unwrap`/`expect`** 于请求路径上; 只有初始化期的不变量 (固定正则、schema 序列化) 允许.
- **禁止内部导入.** 导入一律放文件顶部.
- **禁止全局可变状态.** 依赖经 `Runtime` 构造后注入; 热配置变更整体重建 `Runtime`.
- **迁移必须用 sqlx.** `migrations/` 下只新增文件, 不修改已应用的迁移.
- **测试覆盖非法输入.** 解析与路由的表测试必须包含非法用例, 并断言错误信息的明确性.

## 文档

修改跨模块边界、顺序或契约后同步更新 `docs/dev/`; 归属与判据沿用根仓库 AGENTS.md 的「内容归属」.
