# 开发文档

> 本目录是**索引**: 只记录跨文件的边界、顺序、契约与取舍。字段、签名与单文件内部的原因去源码。
> 归属判据与根仓库一致, 见 [`../../AGENTS.md`](../../AGENTS.md)「内容归属」; 遣词造句见 [`../../docs/dev/writing.md`](../../docs/dev/writing.md), 项目用词见 [`../../docs/dev/terms.md`](../../docs/dev/terms.md)。

阅读顺序:

1. [architecture.md](architecture.md) — crate 边界、启动顺序、运行时替换
2. [parsing.md](parsing.md) — 番号解析已覆盖的范围与缺口
3. [build.md](build.md) — 命令、构建依赖、CI 范围

| 主题 | 文档 |
|------|------|
| 持久化、传输契约、错误映射 | [architecture.md](architecture.md) |
| 前端与契约生成 | [build.md](build.md) |
| 重写范围与决策 | [`../../docs/adr/`](../../docs/adr/) |

同一事实只出现在一个文档; 其它位置用相对链接。
