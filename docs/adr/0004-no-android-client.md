# 不提供 Android 客户端

---
Status: accepted
---

`amane-rs` 本轮只交付服务端与桌面端, 不实现 Android 客户端: 既不沿用 `androidapp/` 的 WebView 壳, 也不使用 Tauri 2 的移动端能力。

## 考虑过的方案

- **沿用 Kotlin WebView 壳指向新服务端**: 壳依赖旧实现的接口形状与页面结构, 前端重做后壳内的地址、鉴权与交互都要跟着改, 与重做一个客户端的工作量相当。
- **Tauri 2 mobile**: 把 BoringSSL、SQLite 与文件系统压进 Android 交叉编译链, 而手机侧没有媒体库, 刮削也不需要贴着文件运行。

## 后果

- 迁移期结束后, 移动端没有可用客户端; 旧壳只能配合 Python 服务端使用。
- Android 交叉编译不进入 CI, 因此 BoringSSL 的移动端构建问题不会被提前暴露。
