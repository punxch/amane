# core / server / desktop 三层，前端只经由 HTTP 契约

---
Status: accepted
---

Rust 侧分为三个 crate：`amane-core` 承载业务，不依赖传输层；`amane-server` 以 axum 暴露 HTTP，服务 Docker 部署与浏览器访问；`amane-desktop` 是 Tauri 外壳，内嵌 axum 并监听 `127.0.0.1` 的随机端口。前端只经由 HTTP 契约访问后端，桌面端不另设 IPC 命令层。

## 考虑过的方案

- **纯 Tauri IPC**：每个功能需要 IPC 与 HTTP 两份适配层；Docker、浏览器访问、Android WebView 外壳与 CLI 同时失去宿主。
- **Tauri 仅作 WebView 外壳指向远端服务**：桌面端无法离线自持，与"逐步替代"的目标不一致。

## 后果

桌面端开放本地端口，鉴权沿用 API token；端口在启动时分配，并随前端引导信息下发。
API 契约由 Rust 侧生成 OpenAPI，前端 client 的生成链路与现有 `just generate` 保持同一形状。
