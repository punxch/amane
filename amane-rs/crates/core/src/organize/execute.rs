//! 整理执行: 按预览落点移动文件; 字幕随配, STRM 按配置生成, 冲突按预览时的选择处置.

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use thiserror::Error;

/// 回收目录名. 覆盖前的旧目标移到这里, 误删可恢复; 清理切片复用同一目录.
pub const TRASH_DIRNAME: &str = ".amane_trash";

/// 默认字幕扩展名. 带前导点; 与 Python 版默认集一致.
pub const DEFAULT_SUBTITLE_EXTENSIONS: &[&str] = &[".srt", ".ass", ".ssa", ".vtt", ".sub"];

/// 冲突处置. 预览标出冲突, 用户在预览阶段为每个冲突选一个处置.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ConflictAction {
    /// 不动这个文件, 记录原因.
    Skip,
    /// 覆盖目标; 旧文件先移入回收目录.
    Overwrite,
    /// 目标改名 (stem 追加 `-1`、`-2`…) 直到不冲突.
    Rename,
}

impl ConflictAction {
    pub fn parse(value: &str) -> Option<Self> {
        match value {
            "skip" => Some(Self::Skip),
            "overwrite" => Some(Self::Overwrite),
            "rename" => Some(Self::Rename),
            _ => None,
        }
    }
}

/// 一项执行请求: 预览算出的源与目标, 加上处置选择.
#[derive(Debug, Clone)]
pub struct ExecuteRequest {
    /// 相对库根的 POSIX 路径.
    pub source_rel: String,
    /// 预览渲染出的目标, 相对库根.
    pub target_rel: String,
    pub number: String,
    /// 无冲突时忽略此字段; 有冲突时缺省按 skip 处理.
    pub action: ConflictAction,
}

/// 单项执行结果.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ItemOutcome {
    /// 视频 (及随配字幕) 已移动到目标.
    Moved {
        target: String,
        subtitles: Vec<String>,
        strm: Option<String>,
    },
    /// 没动, 原因给人看.
    Skipped(String),
}

/// 一批执行的总结果.
#[derive(Debug, Default, PartialEq, Eq)]
pub struct ExecuteReport {
    pub moved: Vec<ItemOutcome>,
    pub plans: Vec<PlanRegistration>,
}

/// 执行成功后待登记的一条整理记录; 仓储写入由调用方完成.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PlanRegistration {
    pub path: String,
    pub number: String,
    pub source_path: String,
}

#[derive(Debug, Error)]
pub enum ExecuteError {
    #[error("移动 {from} 到 {to} 失败: {source}")]
    Move {
        from: String,
        to: String,
        source: std::io::Error,
    },
    #[error("创建目录 {0} 失败: {1}")]
    CreateDir(String, std::io::Error),
}

/// 执行一项: 移动视频、随配字幕、按需生成 STRM.
///
/// 落点与预览一致 —— 这里不再渲染模板, 只信任预览给出的目标; 执行期的状态变化
/// (源消失、目标新出现) 在本函数内判定并转成跳过, 不外溢成错误.
pub fn execute_item(
    root: &Path,
    request: &ExecuteRequest,
    subtitle_extensions: &[String],
    create_strm: bool,
) -> Result<ItemOutcome, ExecuteError> {
    let source = root.join(&request.source_rel);
    let mut target_rel = request.target_rel.clone();

    // 执行期状态变化 1: 源文件在预览后消失.
    if source.symlink_metadata().is_err() {
        return Ok(ItemOutcome::Skipped(format!(
            "源文件 {} 已不存在",
            request.source_rel
        )));
    }

    // 源与目标相同: 无需移动.
    if same_path(&request.source_rel, &target_rel) {
        let target = root.join(&target_rel);
        let strm = if create_strm {
            Some(write_strm(root, &target)?)
        } else {
            None
        };
        return Ok(ItemOutcome::Moved {
            target: target_rel,
            subtitles: Vec::new(),
            strm,
        });
    }

    let mut target = root.join(&target_rel);
    let mut target_exists = target.symlink_metadata().is_ok();
    if target_exists {
        match request.action {
            // 执行期状态变化 2: 预览时目标不存在, 执行时新出现且没有选择覆盖 → 保护性跳过.
            ConflictAction::Skip => {
                return Ok(ItemOutcome::Skipped(format!(
                    "目标 {} 已存在且未选择覆盖",
                    request.target_rel
                )));
            }
            // 改名: stem 追加序号直到不冲突, 占用者不动.
            ConflictAction::Rename => {
                target_rel = rename_candidate(root, &request.target_rel);
                target = root.join(&target_rel);
                target_exists = false;
            }
            ConflictAction::Overwrite => {}
        }
    }

    // 覆盖前旧目标进入回收目录, 保留相对结构, 误删可恢复.
    if target_exists {
        move_to_trash(root, &target)?;
    }

    if let Some(parent) = target.parent() {
        std::fs::create_dir_all(parent)
            .map_err(|source| ExecuteError::CreateDir(parent.display().to_string(), source))?;
    }
    std::fs::rename(&source, &target).map_err(|source| ExecuteError::Move {
        from: request.source_rel.clone(),
        to: target_rel.clone(),
        source,
    })?;

    let subtitles = move_subtitles(root, request, &target_rel, subtitle_extensions)?;
    let strm = if create_strm {
        Some(write_strm(root, &target)?)
    } else {
        None
    };

    Ok(ItemOutcome::Moved {
        target: target_rel,
        subtitles,
        strm,
    })
}

/// 同库内路径相同判定: 规范化为 POSIX 后比较.
fn same_path(left: &str, right: &str) -> bool {
    let normalize = |value: &str| value.replace('\\', "/").trim_matches('/').to_owned();
    normalize(left) == normalize(right) && !normalize(left).is_empty()
}

/// 旧目标移入 `<root>/.amane_trash/<原相对路径>`. 清理切片复用同一目录.
pub fn move_to_trash(root: &Path, target: &Path) -> Result<(), ExecuteError> {
    let relative = target
        .strip_prefix(root)
        .map(Path::to_path_buf)
        .unwrap_or_else(|_| target.to_path_buf());
    let destination = root.join(TRASH_DIRNAME).join(relative);
    if let Some(parent) = destination.parent() {
        std::fs::create_dir_all(parent)
            .map_err(|source| ExecuteError::CreateDir(parent.display().to_string(), source))?;
    }
    std::fs::rename(target, &destination).map_err(|source| ExecuteError::Move {
        from: target.display().to_string(),
        to: destination.display().to_string(),
        source,
    })?;
    Ok(())
}

/// 视频同目录 (不递归) 扩展名命中的字幕随视频移动, 重命名为 `<目标主干><字幕扩展名>`.
///
/// 只认扩展名; 目标字幕已存在时跳过该字幕并保留在原地, 不覆盖任何已有文件.
fn move_subtitles(
    root: &Path,
    request: &ExecuteRequest,
    target_rel: &str,
    subtitle_extensions: &[String],
) -> Result<Vec<String>, ExecuteError> {
    // 与配置校验用同一个归一函数: 校验补前导点而这里只小写, 配置写 `srt` 就永远匹配不上.
    let extensions: Vec<String> = subtitle_extensions
        .iter()
        .map(|value| crate::library::normalize_extension(value))
        .collect();
    if extensions.is_empty() {
        return Ok(Vec::new());
    }
    let source = root.join(&request.source_rel);
    let source_stem = source
        .file_stem()
        .map(std::ffi::OsStr::to_string_lossy)
        .unwrap_or_default()
        .into_owned();
    let source_dir = match source.parent() {
        Some(dir) if dir != root => dir.to_path_buf(),
        // 根目录下的视频没有可归属的子目录, 仍按其父目录 (即根) 扫描.
        _ => root.to_path_buf(),
    };
    let entries = match std::fs::read_dir(&source_dir) {
        Ok(entries) => entries,
        // 源目录已消失: 字幕无从谈起, 视频本体已在前面移动成功.
        Err(_) => return Ok(Vec::new()),
    };

    let target = root.join(target_rel);
    let target_stem = target
        .file_stem()
        .map(std::ffi::OsStr::to_string_lossy)
        .unwrap_or_default();
    let target_dir = target.parent().unwrap_or(root).to_path_buf();

    let mut moved = Vec::new();
    for entry in entries.filter_map(|entry| entry.ok()) {
        let path = entry.path();
        if path.is_dir() || path == source {
            continue;
        }
        let extension = path
            .extension()
            .map(|ext| format!(".{}", ext.to_string_lossy().to_lowercase()))
            .unwrap_or_default();
        if !extensions.contains(&extension) {
            continue;
        }

        // 同目录可能有多个视频: 只搬主干与本视频一致的, 语言后缀 (video.zh.srt) 一并认.
        let stem = path
            .file_stem()
            .map(std::ffi::OsStr::to_string_lossy)
            .unwrap_or_default()
            .into_owned();
        if stem != source_stem && !stem.starts_with(&format!("{source_stem}.")) {
            continue;
        }
        let suffix = &stem[source_stem.len()..];

        let destination = target_dir.join(format!("{target_stem}{suffix}{extension}"));
        if destination.symlink_metadata().is_ok() {
            continue;
        }
        std::fs::rename(&path, &destination).map_err(|source| ExecuteError::Move {
            from: path.display().to_string(),
            to: destination.display().to_string(),
            source,
        })?;
        let relative = destination
            .strip_prefix(root)
            .map(|path| path.to_string_lossy().replace('\\', "/"))
            .unwrap_or_else(|_| destination.to_string_lossy().to_string());
        moved.push(relative);
    }
    Ok(moved)
}

/// STRM 指针文件: 正文是视频的完整绝对路径, 原样写出 —— 不折叠段、不截断, 换行结尾.
fn write_strm(root: &Path, target: &Path) -> Result<String, ExecuteError> {
    let strm_path = target.with_extension("strm");
    // 绝对路径是纯文本, 不经过段规范化: 与路径模板的折叠/截断规则无关.
    let body = format!(
        "{}\n",
        target
            .canonicalize()
            .unwrap_or_else(|_| target.to_path_buf())
            .display()
    );
    std::fs::write(&strm_path, &body).map_err(|source| ExecuteError::Move {
        from: target.display().to_string(),
        to: strm_path.display().to_string(),
        source,
    })?;
    let relative = strm_path
        .strip_prefix(root)
        .map(|path| path.to_string_lossy().replace('\\', "/"))
        .unwrap_or_else(|_| strm_path.to_string_lossy().to_string());
    Ok(relative)
}

/// 目标改名: stem 追加 `-1`、`-2`… 直到磁盘上不存在.
pub fn rename_candidate(root: &Path, target_rel: &str) -> String {
    let target = root.join(target_rel);
    if target.symlink_metadata().is_err() {
        return target_rel.to_owned();
    }
    let stem = target
        .file_stem()
        .map(|stem| stem.to_string_lossy().into_owned())
        .unwrap_or_default();
    let extension = target
        .extension()
        .map(|ext| format!(".{}", ext.to_string_lossy()))
        .unwrap_or_default();
    let parent = target.parent().unwrap_or(root);

    for sequence in 1.. {
        let candidate = parent.join(format!("{stem}-{sequence}{extension}"));
        if candidate.symlink_metadata().is_err() {
            return candidate
                .strip_prefix(root)
                .map(|path| path.to_string_lossy().replace('\\', "/"))
                .unwrap_or_else(|_| candidate.to_string_lossy().to_string());
        }
    }
    unreachable!("循环内必返回")
}

/// 批量执行: 单项失败只跳过该项, 不中断其余.
///
/// 成功项给出待登记记录; 仓储写入由调用方在事务外逐条进行.
pub fn execute_batch(
    root: &Path,
    requests: &[ExecuteRequest],
    subtitle_extensions: &[String],
    create_strm: bool,
) -> (ExecuteReport, Vec<(String, String)>) {
    let mut report = ExecuteReport::default();
    let mut failures: Vec<(String, String)> = Vec::new();

    for request in requests {
        match execute_item(root, request, subtitle_extensions, create_strm) {
            Ok(ItemOutcome::Moved {
                target,
                subtitles,
                strm,
            }) => {
                report.plans.push(PlanRegistration {
                    path: target.clone(),
                    number: request.number.clone(),
                    source_path: request.source_rel.clone(),
                });
                report.moved.push(ItemOutcome::Moved {
                    target,
                    subtitles,
                    strm,
                });
            }
            Ok(ItemOutcome::Skipped(reason)) => failures.push((request.source_rel.clone(), reason)),
            Err(error) => failures.push((request.source_rel.clone(), error.to_string())),
        }
    }

    (report, failures)
}

/// 回收目录里一条记录的原路径; 清理切片据此恢复.
pub fn trash_destination(root: &Path, target_rel: &str) -> PathBuf {
    root.join(TRASH_DIRNAME).join(target_rel)
}

#[cfg(test)]
mod tests {
    use std::fs;

    use super::{ConflictAction, ExecuteRequest, TRASH_DIRNAME, execute_batch, execute_item};

    fn request(source: &str, target: &str, action: ConflictAction) -> ExecuteRequest {
        ExecuteRequest {
            source_rel: source.to_owned(),
            target_rel: target.to_owned(),
            number: "ABC-123".to_owned(),
            action,
        }
    }

    #[test]
    fn moves_to_preview_target() {
        let dir = tempfile::tempdir().expect("应创建临时目录");
        let root = dir.path();
        fs::create_dir_all(root.join("downloads")).expect("应创建目录");
        fs::write(root.join("downloads/ABC-123.mp4"), b"video").expect("应写入视频");

        let outcome = execute_item(
            root,
            &request(
                "downloads/ABC-123.mp4",
                "ABC-123/ABC-123.mp4",
                ConflictAction::Skip,
            ),
            &[],
            false,
        )
        .expect("应执行成功");

        let super::ItemOutcome::Moved {
            target,
            subtitles,
            strm,
        } = outcome
        else {
            panic!("应移动");
        };
        assert_eq!(target, "ABC-123/ABC-123.mp4");
        assert!(subtitles.is_empty());
        assert!(strm.is_none());
        assert!(
            fs::read(root.join("ABC-123/ABC-123.mp4")).expect("应读到视频") == b"video",
            "内容应一致"
        );
        assert!(!root.join("downloads/ABC-123.mp4").exists(), "源应消失");
    }

    #[test]
    fn subtitles_move_and_rename_keeping_extension() {
        let dir = tempfile::tempdir().expect("应创建临时目录");
        let root = dir.path();
        fs::create_dir_all(root.join("downloads")).expect("应创建目录");
        fs::write(root.join("downloads/ABC-123.mp4"), b"v").expect("应写入视频");
        fs::write(root.join("downloads/ABC-123.srt"), b"sub").expect("应写入字幕");
        fs::write(root.join("downloads/ABC-123.ass"), b"sub2").expect("应写入字幕");

        let outcome = execute_item(
            root,
            &request(
                "downloads/ABC-123.mp4",
                "ABC-123/ABC-123.mp4",
                ConflictAction::Skip,
            ),
            &super::DEFAULT_SUBTITLE_EXTENSIONS
                .iter()
                .map(|ext| (*ext).to_owned())
                .collect::<Vec<_>>(),
            false,
        )
        .expect("应执行成功");

        let super::ItemOutcome::Moved { subtitles, .. } = outcome else {
            panic!("应移动");
        };
        assert_eq!(subtitles.len(), 2, "两个字幕都应随配");
        assert!(
            root.join("ABC-123/ABC-123.srt").exists() && root.join("ABC-123/ABC-123.ass").exists(),
            "字幕重命名为主干 + 原扩展名"
        );
        assert!(!root.join("downloads/ABC-123.srt").exists(), "原字幕应消失");
    }

    #[test]
    fn subtitles_of_other_videos_are_left_alone() {
        let dir = tempfile::tempdir().expect("应创建临时目录");
        let root = dir.path();
        fs::create_dir_all(root.join("downloads")).expect("应创建目录");
        fs::write(root.join("downloads/ABC-123.mp4"), b"v").expect("应写入视频");
        fs::write(root.join("downloads/ABC-123.srt"), b"sub").expect("应写入字幕");
        fs::write(root.join("downloads/ABC-123.zh.srt"), b"sub").expect("应写入语言字幕");
        fs::write(root.join("downloads/ABC-456.srt"), b"other").expect("应写入他片字幕");

        let outcome = execute_item(
            root,
            &request(
                "downloads/ABC-123.mp4",
                "ABC-123/ABC-123.mp4",
                ConflictAction::Skip,
            ),
            &["srt".to_owned()],
            false,
        )
        .expect("应执行成功");

        let super::ItemOutcome::Moved { subtitles, .. } = outcome else {
            panic!("应移动");
        };
        assert_eq!(subtitles.len(), 2, "只搬本视频的两条字幕");
        assert!(
            root.join("ABC-123/ABC-123.srt").exists()
                && root.join("ABC-123/ABC-123.zh.srt").exists(),
            "语言后缀应保留"
        );
        assert!(
            root.join("downloads/ABC-456.srt").exists(),
            "同目录其它视频的字幕不能被搬走"
        );
    }

    #[test]
    fn subtitle_extensions_without_leading_dot_still_match() {
        let dir = tempfile::tempdir().expect("应创建临时目录");
        let root = dir.path();
        fs::create_dir_all(root.join("downloads")).expect("应创建目录");
        fs::write(root.join("downloads/ABC-123.mp4"), b"v").expect("应写入视频");
        fs::write(root.join("downloads/ABC-123.SRT"), b"sub").expect("应写入字幕");

        let outcome = execute_item(
            root,
            &request(
                "downloads/ABC-123.mp4",
                "ABC-123/ABC-123.mp4",
                ConflictAction::Skip,
            ),
            &["srt".to_owned()],
            false,
        )
        .expect("应执行成功");

        let super::ItemOutcome::Moved { subtitles, .. } = outcome else {
            panic!("应移动");
        };
        assert_eq!(
            subtitles.len(),
            1,
            "配置写 `srt` 时也要匹配: 校验会补前导点, 执行必须用同一套归一"
        );
    }

    #[test]
    fn strm_keeps_full_absolute_path_unclipped() {
        let dir = tempfile::tempdir().expect("应创建临时目录");
        let root = dir.path();
        fs::create_dir_all(root.join("a")).expect("应创建目录");
        fs::write(root.join("a/ABC-123.mp4"), b"v").expect("应写入视频");

        let outcome = execute_item(
            root,
            &request("a/ABC-123.mp4", "b/ABC-123.mp4", ConflictAction::Skip),
            &[],
            true,
        )
        .expect("应执行成功");

        let super::ItemOutcome::Moved { strm, .. } = outcome else {
            panic!("应移动");
        };
        let strm = strm.expect("应生成 STRM");
        let body = fs::read_to_string(root.join("b/ABC-123.strm")).expect("应读到 STRM");
        // canonicalize 会解析符号链接 (macOS 的 /var → /private/var), 正文以真实路径为准.
        let expected = root.join("b/ABC-123.mp4").canonicalize().expect("应解析");
        assert_eq!(
            body.trim_end(),
            expected.display().to_string(),
            "正文是完整绝对路径, 不折叠不截断"
        );
        assert!(body.ends_with('\n'), "正文以换行结尾");
        assert!(strm == "b/ABC-123.strm");
    }

    #[test]
    fn overwrite_moves_old_target_into_trash() {
        let dir = tempfile::tempdir().expect("应创建临时目录");
        let root = dir.path();
        fs::create_dir_all(root.join("downloads")).expect("应创建目录");
        fs::write(root.join("downloads/ABC-123.mp4"), b"new").expect("应写入新视频");
        fs::create_dir_all(root.join("ABC-123")).expect("应创建目录");
        fs::write(root.join("ABC-123/ABC-123.mp4"), b"old").expect("应写入旧视频");

        execute_item(
            root,
            &request(
                "downloads/ABC-123.mp4",
                "ABC-123/ABC-123.mp4",
                ConflictAction::Overwrite,
            ),
            &[],
            false,
        )
        .expect("应执行成功");

        assert_eq!(
            fs::read(root.join("ABC-123/ABC-123.mp4")).expect("应读到新视频"),
            b"new",
            "目标应被覆盖"
        );
        let recovered = root.join(TRASH_DIRNAME).join("ABC-123/ABC-123.mp4");
        assert_eq!(
            fs::read(&recovered).expect("旧文件应可从回收目录恢复"),
            b"old",
            "覆盖前旧文件应进入回收目录"
        );
    }

    #[test]
    fn rename_action_picks_free_suffix() {
        let dir = tempfile::tempdir().expect("应创建临时目录");
        let root = dir.path();
        fs::create_dir_all(root.join("ABC-123")).expect("应创建目录");
        fs::create_dir_all(root.join("downloads")).expect("应创建目录");
        fs::write(root.join("ABC-123/ABC-123.mp4"), b"old").expect("应写入占用者");
        fs::write(root.join("downloads/ABC-123.mp4"), b"new").expect("应写入视频");

        // Rename 处置在 execute_item 内自动挑空闲后缀, 占用者不动.
        let outcome = execute_item(
            root,
            &request(
                "downloads/ABC-123.mp4",
                "ABC-123/ABC-123.mp4",
                ConflictAction::Rename,
            ),
            &[],
            false,
        )
        .expect("应执行成功");
        let super::ItemOutcome::Moved { target, .. } = outcome else {
            panic!("应移动");
        };
        assert_eq!(target, "ABC-123/ABC-123-1.mp4");
        assert_eq!(
            fs::read(root.join("ABC-123/ABC-123.mp4")).expect("占用者应完好"),
            b"old"
        );
        assert_eq!(
            fs::read(root.join("ABC-123/ABC-123-1.mp4")).expect("应读到新文件"),
            b"new"
        );
    }

    #[test]
    fn vanished_source_skips_without_blocking_others() {
        let dir = tempfile::tempdir().expect("应创建临时目录");
        let root = dir.path();
        fs::create_dir_all(root.join("downloads")).expect("应创建目录");
        fs::write(root.join("downloads/XYZ-999.mp4"), b"v").expect("应写入视频");
        // ABC-123 的源文件在预览后被人删掉.

        let requests = [
            request(
                "downloads/ABC-123.mp4",
                "ABC-123/ABC-123.mp4",
                ConflictAction::Skip,
            ),
            request(
                "downloads/XYZ-999.mp4",
                "XYZ-999/XYZ-999.mp4",
                ConflictAction::Skip,
            ),
        ];
        let (report, failures) = execute_batch(root, &requests, &[], false);

        assert_eq!(report.plans.len(), 1, "其余文件照常整理");
        assert_eq!(report.plans[0].number, "ABC-123");
        assert_eq!(report.plans[0].path, "XYZ-999/XYZ-999.mp4");
        assert_eq!(failures.len(), 1, "消失的源只跳过并报告");
        assert!(
            failures[0].1.contains("已不存在"),
            "跳过原因应可读, 实际 {}",
            failures[0].1
        );
    }

    #[test]
    fn newly_appeared_target_protects_without_choice() {
        let dir = tempfile::tempdir().expect("应创建临时目录");
        let root = dir.path();
        fs::create_dir_all(root.join("downloads")).expect("应创建目录");
        fs::create_dir_all(root.join("ABC-123")).expect("应创建目录");
        fs::write(root.join("downloads/ABC-123.mp4"), b"v").expect("应写入视频");
        fs::write(root.join("ABC-123/ABC-123.mp4"), b"other").expect("应写入新占用者");

        let (report, failures) = execute_batch(
            root,
            &[request(
                "downloads/ABC-123.mp4",
                "ABC-123/ABC-123.mp4",
                ConflictAction::Skip,
            )],
            &[],
            false,
        );

        assert!(report.plans.is_empty(), "未选择覆盖时不得覆盖");
        assert!(failures[0].1.contains("未选择覆盖"));
        assert_eq!(
            fs::read(root.join("ABC-123/ABC-123.mp4")).expect("占用者应完好"),
            b"other"
        );
    }

    #[test]
    fn same_source_and_target_only_writes_strm() {
        let dir = tempfile::tempdir().expect("应创建临时目录");
        let root = dir.path();
        fs::create_dir_all(root.join("ABC-123")).expect("应创建目录");
        fs::write(root.join("ABC-123/ABC-123.mp4"), b"v").expect("应写入视频");

        let outcome = execute_item(
            root,
            &request(
                "ABC-123/ABC-123.mp4",
                "ABC-123/ABC-123.mp4",
                ConflictAction::Skip,
            ),
            &[],
            true,
        )
        .expect("应执行成功");

        let super::ItemOutcome::Moved { strm, .. } = outcome else {
            panic!("应成功");
        };
        assert!(strm.is_some());
        assert!(root.join("ABC-123/ABC-123.mp4").exists(), "同路径不移动");
    }
}
