//! 清理清单与回收目录: 先看清单再执行, 删除进回收目录, 误删可恢复.

use std::path::Path;

use serde::{Deserialize, Serialize};
use thiserror::Error;

use super::execute::{ExecuteError, TRASH_DIRNAME, move_to_trash};
use crate::library::{LibraryFile, ScanKind};

/// 清单里的一项: 待删文件、大小与命中原因.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct CleanupItem {
    /// 相对库根的 POSIX 路径.
    pub path: String,
    /// 磁盘上的字节数; 读不到时为 0.
    pub size: u64,
    /// 命中的规则说明: 黑名单模式原文、大小阈值或系统产物名.
    pub reason: String,
}

/// 回收目录里的一项.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TrashItem {
    /// 相对回收目录的路径, 即文件的原相对路径.
    pub path: String,
    /// 磁盘上的字节数; 读不到时为 0.
    pub size: u64,
}

/// 清单构建错误.
#[derive(Debug, Error)]
pub enum CleanupError {
    #[error("移动 {path} 失败: {source}")]
    Move {
        path: String,
        source: std::io::Error,
    },
}

/// 由扫描索引构建清理清单: 无效文件与系统产物; 残留目录不属于清理范围.
///
/// 只读: 不触碰文件系统里的文件本体, 只为显示大小做 stat. 索引里没有
/// rule/detail 的无效文件给出生成时的规则名兜底.
pub fn build_cleanup_manifest(index: &[LibraryFile], root: &Path) -> Vec<CleanupItem> {
    let mut items = Vec::new();
    for file in index {
        let deletable = matches!(file.kind, ScanKind::Unwanted | ScanKind::System);
        if !deletable {
            continue;
        }
        let size = root
            .join(&file.path)
            .symlink_metadata()
            .map(|metadata| metadata.len())
            .unwrap_or(0);
        let reason = match (&file.rule, &file.detail) {
            (Some(rule), Some(detail)) => format!("{rule}: {detail}"),
            (Some(rule), None) => rule.clone(),
            (None, _) => "系统产物".to_owned(),
        };
        items.push(CleanupItem {
            path: file.path.clone(),
            size,
            reason,
        });
    }
    items
}

/// 执行清理: 清单为空时不动任何文件; 单项失败跳过并继续.
///
/// 文件按整理执行的同一规则进入回收目录 (保留相对结构), 原路径不再存在.
pub fn execute_cleanup(root: &Path, items: &[CleanupItem]) -> (Vec<String>, Vec<(String, String)>) {
    let mut moved = Vec::new();
    let mut failures: Vec<(String, String)> = Vec::new();
    for item in items {
        let target = root.join(&item.path);
        if target.symlink_metadata().is_err() {
            // 磁盘上已不存在: 视为完成, 索引照删.
            moved.push(item.path.clone());
            continue;
        }
        match move_to_trash(root, &target) {
            Ok(()) => moved.push(item.path.clone()),
            Err(ExecuteError::Move { source, .. }) => {
                failures.push((item.path.clone(), source.to_string()));
            }
            Err(ExecuteError::CreateDir(_, source)) => {
                failures.push((item.path.clone(), source.to_string()));
            }
        }
    }
    (moved, failures)
}

/// 列出回收目录的内容与占用. 不递归展开 `.amane_trash` 内的子目录结构 —
/// 恢复按原相对路径整文件进行, 目录只作为路径前缀出现.
pub fn list_trash(root: &Path) -> Vec<TrashItem> {
    let trash = root.join(TRASH_DIRNAME);
    let mut items = Vec::new();
    collect_files(&trash, &trash, &mut items);
    items.sort_by(|a, b| a.path.cmp(&b.path));
    items
}

fn collect_files(base: &Path, dir: &Path, items: &mut Vec<TrashItem>) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    let mut paths: Vec<_> = entries
        .filter_map(|entry| entry.ok())
        .map(|entry| entry.path())
        .collect();
    paths.sort();
    for path in paths {
        let Ok(metadata) = path.symlink_metadata() else {
            continue;
        };
        if metadata.is_dir() {
            collect_files(base, &path, items);
            continue;
        }
        let relative = path
            .strip_prefix(base)
            .map(|p| p.to_string_lossy().replace('\\', "/"))
            .unwrap_or_default();
        if !relative.is_empty() {
            items.push(TrashItem {
                path: relative,
                size: metadata.len(),
            });
        }
    }
}

/// 回收目录的总占用 (字节).
pub fn trash_total_size(items: &[TrashItem]) -> u64 {
    items.iter().map(|item| item.size).sum()
}

/// 恢复失败的原因.
#[derive(Debug, Error, PartialEq, Eq)]
pub enum RestoreError {
    #[error("回收目录中不存在 {0}")]
    NotFound(String),
    #[error("原路径 {0} 已存在文件, 恢复被拒绝; 先处理原路径的文件再恢复")]
    TargetExists(String),
    #[error("移动 {0} 失败: {1}")]
    Move(String, String),
}

/// 把回收目录里的一个文件恢复到原路径.
///
/// 目标已存在时拒绝恢复而不是覆盖; 回收目录里的文件保持原样, 用户可先处理原路径再重试.
pub fn restore_from_trash(root: &Path, relative: &str) -> Result<String, RestoreError> {
    let source = root.join(TRASH_DIRNAME).join(relative);
    if source.symlink_metadata().is_err() {
        return Err(RestoreError::NotFound(relative.to_owned()));
    }
    let target = root.join(relative);
    if target.symlink_metadata().is_ok() {
        return Err(RestoreError::TargetExists(relative.to_owned()));
    }
    if let Some(parent) = target.parent() {
        std::fs::create_dir_all(parent)
            .map_err(|error| RestoreError::Move(relative.to_owned(), error.to_string()))?;
    }
    std::fs::rename(&source, &target)
        .map_err(|error| RestoreError::Move(relative.to_owned(), error.to_string()))?;
    Ok(relative.to_owned())
}

#[cfg(test)]
mod tests {
    use std::fs;

    use super::{
        CleanupItem, build_cleanup_manifest, execute_cleanup, list_trash, restore_from_trash,
        trash_total_size,
    };
    use crate::library::{LibraryFile, ScanKind, UnwantedRule};

    fn index() -> Vec<LibraryFile> {
        vec![
            LibraryFile::unwanted("ads.mp4", UnwantedRule::Blacklist, "(?i)ads".to_owned()),
            LibraryFile::new(".DS_Store", ScanKind::System),
            LibraryFile::new("film.mp4", ScanKind::Media),
            LibraryFile::new("empty", ScanKind::LeftoverDir),
        ]
    }

    #[test]
    fn manifest_lists_unwanted_and_system_with_size_and_reason() {
        let dir = tempfile::tempdir().expect("应创建临时目录");
        let root = dir.path();
        fs::write(root.join("ads.mp4"), vec![0u8; 100]).expect("应写入无效文件");
        fs::write(root.join(".DS_Store"), vec![0u8; 4]).expect("应写入系统产物");

        let manifest = build_cleanup_manifest(&index(), root);

        assert_eq!(manifest.len(), 2, "媒体与残留目录不进清理清单");
        assert_eq!(manifest[0].path, "ads.mp4");
        assert_eq!(manifest[0].size, 100);
        assert_eq!(manifest[0].reason, "blacklist: (?i)ads");
        assert_eq!(manifest[1].path, ".DS_Store");
        assert_eq!(manifest[1].reason, "系统产物");
    }

    #[test]
    fn empty_manifest_touches_nothing() {
        let dir = tempfile::tempdir().expect("应创建临时目录");
        let root = dir.path();

        let (moved, failures) = execute_cleanup(root, &[]);

        assert!(moved.is_empty() && failures.is_empty());
        assert!(
            !root.join(super::super::TRASH_DIRNAME).exists(),
            "空清单不得创建回收目录"
        );
    }

    #[test]
    fn execution_moves_into_trash_and_removes_original() {
        let dir = tempfile::tempdir().expect("应创建临时目录");
        let root = dir.path();
        fs::write(root.join("ads.mp4"), b"x").expect("应写入无效文件");
        let items = vec![CleanupItem {
            path: "ads.mp4".to_owned(),
            size: 1,
            reason: "blacklist".to_owned(),
        }];

        let (moved, failures) = execute_cleanup(root, &items);

        assert!(failures.is_empty());
        assert_eq!(moved, ["ads.mp4"]);
        assert!(!root.join("ads.mp4").exists(), "原路径不再存在");
        assert!(
            root.join(super::super::TRASH_DIRNAME)
                .join("ads.mp4")
                .exists(),
            "文件应在回收目录里"
        );
    }

    #[test]
    fn vanished_file_counts_as_moved() {
        let dir = tempfile::tempdir().expect("应创建临时目录");
        let items = vec![CleanupItem {
            path: "gone.mp4".to_owned(),
            size: 0,
            reason: "undersized".to_owned(),
        }];

        let (moved, failures) = execute_cleanup(dir.path(), &items);

        assert_eq!(moved, ["gone.mp4"], "磁盘上已不存在的文件视为完成");
        assert!(failures.is_empty());
    }

    #[test]
    fn trash_listing_reports_occupation() {
        let dir = tempfile::tempdir().expect("应创建临时目录");
        let root = dir.path();
        fs::create_dir_all(root.join(super::super::TRASH_DIRNAME).join("ABC-123"))
            .expect("应创建回收结构");
        fs::write(
            root.join(super::super::TRASH_DIRNAME)
                .join("ABC-123/ads.mp4"),
            vec![0u8; 4096],
        )
        .expect("应写入文件");

        let items = list_trash(root);

        assert_eq!(items.len(), 1);
        assert_eq!(items[0].path, "ABC-123/ads.mp4");
        assert_eq!(items[0].size, 4096);
        assert_eq!(trash_total_size(&items), 4096, "占用应可统计");
    }

    #[test]
    fn restore_puts_file_back() {
        let dir = tempfile::tempdir().expect("应创建临时目录");
        let root = dir.path();
        let items = vec![CleanupItem {
            path: "ads.mp4".to_owned(),
            size: 1,
            reason: "blacklist".to_owned(),
        }];
        fs::write(root.join("ads.mp4"), b"x").expect("应写入文件");
        execute_cleanup(root, &items);

        restore_from_trash(root, "ads.mp4").expect("应恢复成功");

        assert!(root.join("ads.mp4").exists(), "文件回到原路径");
        assert!(
            !root
                .join(super::super::TRASH_DIRNAME)
                .join("ads.mp4")
                .exists(),
            "回收目录里不再保留"
        );
    }

    #[test]
    fn restore_refuses_when_target_exists() {
        let dir = tempfile::tempdir().expect("应创建临时目录");
        let root = dir.path();
        let items = vec![CleanupItem {
            path: "ads.mp4".to_owned(),
            size: 1,
            reason: "blacklist".to_owned(),
        }];
        fs::write(root.join("ads.mp4"), b"trash").expect("应写入文件");
        execute_cleanup(root, &items);
        // 原路径又出现了同名文件.
        fs::write(root.join("ads.mp4"), b"new").expect("应写入占用者");

        let error = restore_from_trash(root, "ads.mp4").expect_err("目标已存在应被拒绝");

        assert!(
            error.to_string().contains("已存在"),
            "应说明原因, 实际 {error}"
        );
        assert_eq!(
            fs::read(root.join("ads.mp4")).expect("占用者应完好"),
            b"new",
            "恢复不得覆盖"
        );
        assert!(
            root.join(super::super::TRASH_DIRNAME)
                .join("ads.mp4")
                .exists(),
            "回收目录里的文件保持原样, 用户可先处理原路径再重试"
        );
    }

    #[test]
    fn restore_reports_missing_entry() {
        let dir = tempfile::tempdir().expect("应创建临时目录");

        let error = restore_from_trash(dir.path(), "nowhere.mp4").expect_err("应报不存在");

        assert!(matches!(error, super::RestoreError::NotFound(_)));
    }
}
