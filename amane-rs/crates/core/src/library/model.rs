//! 媒体库登记与扫描: 根目录登记、文件四类归类与两次扫描间的增量.

use std::path::PathBuf;

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

/// 一个已登记的媒体库根目录.
#[derive(Debug, Clone, Serialize, sqlx::FromRow)]
pub struct LibraryRoot {
    pub id: i64,
    /// 绝对路径, 登记时已规范化.
    pub path: String,
    pub created_at: DateTime<Utc>,
}

impl LibraryRoot {
    pub fn path_buf(&self) -> PathBuf {
        PathBuf::from(&self.path)
    }
}

/// 扫描后一个文件的归类. 前四类即规格中的四类呈现, 未识别单列一档防止静默消失.
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, sqlx::Type, Ord, PartialOrd,
)]
#[serde(rename_all = "snake_case")]
#[sqlx(rename_all = "snake_case")]
pub enum ScanKind {
    /// 媒体文件: 扩展名命中媒体规则且未被否决.
    Media,
    /// 无效文件: 命中黑名单或低于大小阈值, 进清理清单.
    Unwanted,
    /// 系统产物: 操作系统留下的附属文件.
    System,
    /// 未识别: 扩展名不匹配任何规则, 不静默消失.
    Unrecognized,
    /// 残留目录: 子树里没有任何媒体文件的目录.
    LeftoverDir,
}

impl ScanKind {
    /// 全部取值. `as_str` 是唯一的字符串表, 解析从这里反查.
    pub const ALL: [Self; 5] = [
        Self::Media,
        Self::Unwanted,
        Self::System,
        Self::Unrecognized,
        Self::LeftoverDir,
    ];

    pub fn parse(value: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|kind| kind.as_str() == value)
    }

    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Media => "media",
            Self::Unwanted => "unwanted",
            Self::System => "system",
            Self::Unrecognized => "unrecognized",
            Self::LeftoverDir => "leftover_dir",
        }
    }
}

/// 无效文件命中的规则名. `rule` 只给机器判别, 给人看的说明在 `detail`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum UnwantedRule {
    /// 命中文件黑名单正则; detail 存命中的模式原文.
    Blacklist,
    /// 低于最小视频大小; detail 存阈值与实际大小的说明.
    Undersized,
}

impl UnwantedRule {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Blacklist => "blacklist",
            Self::Undersized => "undersized",
        }
    }
}

/// 扫描索引里的一条记录. 目录路径与文件路径共用一张表, 以 kind 区分.
#[derive(Debug, Clone, PartialEq, Eq, sqlx::FromRow, Serialize)]
pub struct LibraryFile {
    /// 相对根目录的相对路径.
    pub path: String,
    pub kind: ScanKind,
    /// 无效文件命中的规则名; 其余类别为空.
    pub rule: Option<String>,
    /// 命中的模式原文或补充说明.
    pub detail: Option<String>,
}

impl LibraryFile {
    pub fn new(path: impl Into<String>, kind: ScanKind) -> Self {
        Self {
            path: path.into(),
            kind,
            rule: None,
            detail: None,
        }
    }

    pub fn unwanted(path: impl Into<String>, rule: UnwantedRule, detail: String) -> Self {
        Self {
            path: path.into(),
            kind: ScanKind::Unwanted,
            rule: Some(rule.as_str().to_owned()),
            detail: Some(detail),
        }
    }
}

/// 一次扫描相对上次的增量. 只报告路径, 历史归类以本次为准.
#[derive(Debug, Default, PartialEq, Eq)]
pub struct ScanDiff {
    pub added: Vec<String>,
    pub removed: Vec<String>,
}

/// 扫描错误: 根目录在两次访问之间消失等遍历故障.
#[derive(Debug, thiserror::Error)]
pub enum ScanError {
    #[error("读取目录 {path} 失败: {source}")]
    Walk {
        path: PathBuf,
        source: std::io::Error,
    },
}
