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

## 前端结构

`ui/` 的外观与信息架构沿用原项目 `web/`: 顶栏 (品牌、全局搜索、连接状态、主题) + 分组侧栏 (浏览 / 管理 / 运行, 设置固定在底部) + 页面壳。页面一律经 `components/common/PageShell.vue` 搭标题与工具条, 不各自复制外壳。

- 颜色、字体与断点集中在 `ui/src/style.css` 的 CSS 变量; 页面不写色值。深浅两套由根元素 `data-theme` 切换, 写入方是 `stores/ui.ts`。
- 列表态 (筛选、页码、视图) 属于 URL; 界面偏好 (主题、侧栏折叠、分页大小) 存本地。
- 影片与媒体库都拆成列表与详情两个路由, 详情只依赖番号或库编号, 可直接打开。

## 桌面壳

`crates/desktop` 是 Tauri 外壳, 与服务端共用同一份前端产物:

- 启动时生成本次进程的令牌, 数据目录取系统应用数据目录, 与 Python 实现的数据目录分开, 两者可同时安装。
- 监听地址固定 `127.0.0.1:0`, 端口由系统分配, 不会与已运行实例冲突; 实际地址在启动后写进窗口地址。
- 前端从地址参数捕获令牌并存入会话, 之后每个请求携带 `Authorization`; 未携带的本地请求被服务端拒绝。
- 关闭窗口即退出进程, 端口随之释放; 登记与任务已持久化, 重启后按基准时刻继续。
- 构建二进制: `cargo build -p amane-desktop`; 打包安装包需要 Tauri CLI (`cargo install tauri-cli` 或 pnpm 引入), 当前 bundle 配置为关闭。
