# Issue 跟踪: 本地 markdown

本仓库的 issue 与 spec 以 markdown 文件保存在 `.scratch/`, 不使用 GitHub Issues.

## 约定

- 一个特性一个目录: `.scratch/<特性 slug>/`
- spec 为 `.scratch/<特性 slug>/spec.md`
- 实现用的 issue 每个文件一条, 位于 `.scratch/<特性 slug>/issues/<NN>-<slug>.md`, 编号从 `01` 起, 禁止合并为单个 tickets 文件
- `Status:` 行写在文件头部附近, 记录 triage 状态, 取值见 [triage-labels.md](triage-labels.md)
- 评论与往来记录追加至文件末尾 `## Comments` 标题下

## 技能要求「发布到 issue 跟踪器」时

在 `.scratch/<特性 slug>/` 下新建文件, 目录不存在时一并创建.

## 技能要求「获取对应 issue」时

读取指定路径的文件. 用户通常直接提供路径或编号.

## 寻路操作

供 `wayfinder` 使用. map 为单个文件, 其下每个子 issue 对应一个文件.

- **map**: `.scratch/<effort>/map.md`, 正文记录 Notes / Decisions-so-far / Fog
- **子 issue**: `.scratch/<effort>/issues/NN-<slug>.md`, 编号从 `01` 起, 正文写问题本身. `Type:` 行记录 `research`/`prototype`/`grilling`/`task`; `Status:` 行记录 `claimed`/`resolved`
- **阻塞**: 头部附近写 `Blocked by: NN, NN`. 所列文件全部为 `resolved` 时该 issue 可开始
- **可开始的 issue**: 扫描 `.scratch/<effort>/issues/`, 取未完成、未被阻塞、未被认领者, 编号最小者优先
- **认领**: 开始工作前写入 `Status: claimed`
- **解决**: 在 `## Answer` 标题下追加答案, 写入 `Status: resolved`, 并向 `map.md` 的 Decisions-so-far 追加上下文指针
