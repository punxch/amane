# Triage 用词

`triage` 以五个状态角色与两个类别角色描述 issue. 本仓库把角色记录为 issue 文件头部的 `Status:` 与 `Kind:` 行, 取值沿用既有 GitHub 标签的用词 (去掉 `status:` / `kind:` 前缀), 迁移到 GitHub 时可直接对应回标签.

| 技能中的角色 | 本仓库取值 | 含义 | 对应 GitHub 标签 |
| --- | --- | --- | --- |
| `needs-triage` | `Status: untriaged` | 待维护者评估 | 无, 需新建 `status:untriaged` |
| `needs-info` | `Status: needs-info` | 等待提出者补充信息 | `status:needs-info` |
| `ready-for-agent` | `Status: ready-for-agent` | 已完备, 可交由 Agent 执行 | 无, 需新建 `status:ready-for-agent` |
| `ready-for-human` | `Status: triaged` | 已确认有效, 等待处理 | `status:triaged` |
| `wontfix` | `Status: wontfix` | 决定不处理 | `status:wontfix` |
| `bug` | `Kind: bug` | 缺陷 | `kind:bug` |
| `enhancement` | `Kind: enhancement` 或 `Kind: feature` | 现有功能改进 / 新功能需求 | `kind:enhancement` / `kind:feature` |

类别角色在本仓库分为三值: 新功能需求写 `feature`, 现有功能改进写 `enhancement`.
