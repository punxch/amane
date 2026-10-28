//! 扫描归类规则: 哪些文件是媒体、哪些是无效文件、哪些根本不入清单.

use std::collections::BTreeSet;
use std::ffi::OsStr;
use std::path::{Path, PathBuf};

use regex::Regex;

use super::model::{LibraryFile, ScanError, ScanKind, UnwantedRule};

/// 操作系统留下的固定附属文件名. 大小写不敏感匹配.
const SYSTEM_ARTIFACTS: &[&str] = &[".ds_store", "thumbs.db", "desktop.ini"];

/// macOS 资源分支文件的前缀, 如 `._video.mp4`.
const APPLEDOUBLE_PREFIX: &str = "._";

/// 回收目录名. 路径任一层含此目录则整个子树不入清单, 交给清理切片处理.
pub const TRASH_DIRNAME: &str = ".amane_trash";

/// 默认媒体扩展名. `.strm` 是路径指针, 算媒体入口但大小过滤对它无意义.
pub const DEFAULT_MEDIA_EXTENSIONS: &[&str] = &[
    ".mp4", ".mkv", ".avi", ".wmv", ".flv", ".mov", ".ts", ".iso", ".strm",
];

/// 路径指针扩展名: 指向别处的字节, 文件本身的大小没有意义.
const POINTER_EXTENSIONS: &[&str] = &[".strm"];

/// 一次扫描使用的规则集. 由配置的扫描节编译而来.
#[derive(Debug, Clone, Default)]
pub struct ScanRules {
    /// 小写带前导点的扩展名, 如 `.mp4`.
    pub media_extensions: BTreeSet<String>,
    /// 字节阈值; 0 表示关闭大小过滤.
    pub min_file_size: u64,
    /// 与文件名 (含扩展名) 匹配的黑名单; 命中即无效文件.
    pub blacklist: Vec<Regex>,
}

impl ScanRules {
    /// 未编译进 [`ScanRules`] 前的原始配置, 校验逻辑在配置层.
    pub fn from_parts(
        media_extensions: impl IntoIterator<Item = impl AsRef<str>>,
        min_file_size: u64,
        blacklist: Vec<Regex>,
    ) -> Self {
        Self {
            media_extensions: media_extensions
                .into_iter()
                .map(|value| normalize_extension(value.as_ref()))
                .collect(),
            min_file_size,
            blacklist,
        }
    }

    /// 是否命中黑名单. 只匹配文件名, 不匹配目录段.
    fn blacklist_hit(&self, name: &str) -> Option<&Regex> {
        self.blacklist.iter().find(|rule| rule.is_match(name))
    }

    fn is_media_extension(&self, path: &Path) -> bool {
        path.extension()
            .and_then(OsStr::to_str)
            .is_some_and(|extension| {
                self.media_extensions
                    .contains(&format!(".{}", extension.to_lowercase()))
            })
    }

    /// 是否为低于阈值的媒体文件.
    ///
    /// 只对媒体扩展名判定; `.strm` 是路径指针不参与; 读不到大小的文件不判,
    /// 不能把读不到的正片当无效丢弃.
    fn is_undersized(&self, path: &Path, full: &Path) -> bool {
        if self.min_file_size == 0 {
            return false;
        }
        let extension = path
            .extension()
            .and_then(OsStr::to_str)
            .map(|value| format!(".{}", value.to_lowercase()))
            .unwrap_or_default();
        if POINTER_EXTENSIONS.contains(&extension.as_str()) {
            return false;
        }
        if !self.is_media_extension(path) {
            return false;
        }
        full.metadata()
            .map(|metadata| metadata.len() < self.min_file_size)
            .unwrap_or(false)
    }

    /// 归类单个文件. 顺序即优先级: 系统产物 → 黑名单 → 大小 → 扩展名.
    ///
    /// 黑名单先于大小判定: 大于阈值但命中黑名单的文件照样归入无效文件,
    /// 且必须带上命中的规则, 否则界面无法说明它为什么被挑出来.
    pub fn classify(&self, relative: &Path, full: &Path) -> LibraryFile {
        let name = relative
            .file_name()
            .and_then(OsStr::to_str)
            .unwrap_or_default();

        if name.to_lowercase() == TRASH_DIRNAME {
            // 单个文件不可能叫回收目录名, 这里只防御目录段; 目录段在遍历层剪枝.
            return LibraryFile::new(relative.to_string_lossy(), ScanKind::System);
        }
        if SYSTEM_ARTIFACTS.contains(&name.to_lowercase().as_str())
            || name.starts_with(APPLEDOUBLE_PREFIX)
        {
            return LibraryFile::new(relative.to_string_lossy(), ScanKind::System);
        }

        if let Some(pattern) = self.blacklist_hit(name) {
            return LibraryFile::unwanted(
                relative.to_string_lossy(),
                UnwantedRule::Blacklist,
                pattern.as_str().to_owned(),
            );
        }

        if !self.is_media_extension(relative) {
            return LibraryFile::new(relative.to_string_lossy(), ScanKind::Unrecognized);
        }

        if self.is_undersized(relative, full) {
            let actual = full
                .metadata()
                .map(|metadata| metadata.len())
                .unwrap_or_default();
            return LibraryFile::unwanted(
                relative.to_string_lossy(),
                UnwantedRule::Undersized,
                format!("小于 {} 字节 (实际 {} 字节)", self.min_file_size, actual),
            );
        }

        LibraryFile::new(relative.to_string_lossy(), ScanKind::Media)
    }
}

/// 规范化扩展名: 小写、补前导点. 形如 `mp4` 与 `.MP4` 都归一为 `.mp4`.
pub fn normalize_extension(value: &str) -> String {
    let trimmed = value.trim().to_lowercase();
    if trimmed.starts_with('.') {
        trimmed
    } else {
        format!(".{trimmed}")
    }
}

/// 遍历结果: 全部文件记录与残留目录.
#[derive(Debug, Default, PartialEq, Eq)]
pub struct ScanOutcome {
    pub files: Vec<LibraryFile>,
    pub leftover_dirs: Vec<PathBuf>,
}

/// 深度优先遍历根目录并归类.
///
/// 子树里没有任何媒体文件的目录记为残留目录 (根目录本身除外). 回收目录整个
/// 剪枝. 目录用排序后的条目, 同一棵树两次扫描给出同序结果.
pub fn scan_directory(root: &Path, rules: &ScanRules) -> Result<ScanOutcome, ScanError> {
    let mut outcome = ScanOutcome::default();
    visit_dir(root, root, rules, &mut outcome)?;
    Ok(outcome)
}

/// 访问一个目录; 返回其子树 (不含自身) 是否存在媒体文件.
fn visit_dir(
    root: &Path,
    dir: &Path,
    rules: &ScanRules,
    outcome: &mut ScanOutcome,
) -> Result<bool, ScanError> {
    let mut entries: Vec<PathBuf> = std::fs::read_dir(dir)
        .map_err(|source| ScanError::Walk {
            path: dir.to_path_buf(),
            source,
        })?
        .filter_map(|entry| entry.ok().map(|entry| entry.path()))
        .collect();
    entries.sort();

    let mut has_media = false;
    for path in entries {
        let Some(relative) = path.strip_prefix(root).ok().map(Path::to_path_buf) else {
            continue;
        };
        let Ok(file_type) = path.metadata().map(|metadata| metadata.is_dir()) else {
            // 读不到的条目 (悬空链接等) 归入未识别, 不静默消失.
            outcome.files.push(LibraryFile::new(
                relative.to_string_lossy(),
                ScanKind::Unrecognized,
            ));
            continue;
        };

        if file_type {
            if relative
                .file_name()
                .and_then(OsStr::to_str)
                .is_some_and(|name| name == TRASH_DIRNAME)
            {
                continue;
            }
            if visit_dir(root, &path, rules, outcome)? {
                has_media = true;
            }
            continue;
        }

        let file = rules.classify(&relative, &path);
        if file.kind == ScanKind::Media {
            has_media = true;
        }
        outcome.files.push(file);
    }

    if !has_media && dir != root {
        outcome.leftover_dirs.push(relative_path(root, dir));
    }

    Ok(has_media)
}

/// 相对路径文本; 调用处保证 `dir` 在 `root` 之下.
fn relative_path(root: &Path, dir: &Path) -> PathBuf {
    dir.strip_prefix(root)
        .map(Path::to_path_buf)
        .unwrap_or_else(|_| dir.to_path_buf())
}

#[cfg(test)]
mod tests {
    use regex::Regex;
    use std::fs;
    use std::path::{Path, PathBuf};

    use super::{DEFAULT_MEDIA_EXTENSIONS, ScanRules, normalize_extension, scan_directory};
    use crate::library::model::ScanKind;

    fn rules(blacklist: &[&str], min_file_size: u64) -> ScanRules {
        ScanRules::from_parts(
            DEFAULT_MEDIA_EXTENSIONS.iter().copied(),
            min_file_size,
            blacklist
                .iter()
                .map(|pattern| Regex::new(pattern).expect("测试正则应合法"))
                .collect(),
        )
    }

    fn touch(dir: &Path, name: &str, size: usize) -> PathBuf {
        let path = dir.join(name);
        fs::write(&path, vec![0u8; size]).expect("应写入测试文件");
        path
    }

    fn kinds_of(outcome: &super::ScanOutcome, kind: ScanKind) -> Vec<&str> {
        outcome
            .files
            .iter()
            .filter(|file| file.kind == kind)
            .map(|file| file.path.as_str())
            .collect()
    }

    #[test]
    fn extension_boundary_split_media_and_unrecognized() {
        let dir = tempfile::tempdir().expect("应创建临时目录");
        let root = dir.path();
        touch(root, "video.MP4", 1024);
        touch(root, "clip.mkv", 1024);
        touch(root, "notes.txt", 16);
        touch(root, "song.mp3", 16);
        // 无扩展名也不静默消失.
        touch(root, "README", 4);

        let outcome = scan_directory(root, &rules(&[], 0)).expect("应扫描成功");

        assert_eq!(
            kinds_of(&outcome, ScanKind::Media),
            ["clip.mkv", "video.MP4"]
        );
        assert_eq!(
            kinds_of(&outcome, ScanKind::Unrecognized),
            ["README", "notes.txt", "song.mp3"]
        );
        assert!(outcome.leftover_dirs.is_empty(), "根目录本身不算残留");
    }

    #[test]
    fn empty_root_yields_nothing() {
        let dir = tempfile::tempdir().expect("应创建临时目录");

        let outcome = scan_directory(dir.path(), &rules(&[], 0)).expect("应扫描成功");

        assert!(outcome.files.is_empty());
        assert!(outcome.leftover_dirs.is_empty(), "空根目录不应报残留");
    }

    #[test]
    fn companion_only_dir_is_leftover_with_unrecognized_files() {
        let dir = tempfile::tempdir().expect("应创建临时目录");
        let root = dir.path();
        fs::create_dir_all(root.join("extras")).expect("应创建子目录");
        touch(&root.join("extras"), "movie.srt", 64);
        touch(&root.join("extras"), "cover.jpg", 64);

        let outcome = scan_directory(root, &rules(&[], 0)).expect("应扫描成功");

        assert_eq!(
            kinds_of(&outcome, ScanKind::Unrecognized),
            ["extras/cover.jpg", "extras/movie.srt"],
            "附属文件应归入未识别, 不能静默消失"
        );
        assert_eq!(
            outcome
                .leftover_dirs
                .iter()
                .map(|path| path.to_string_lossy().to_string())
                .collect::<Vec<_>>(),
            ["extras"],
            "仅含附属文件的目录应记为残留"
        );
    }

    #[test]
    fn empty_subdir_and_media_ancestor_are_split() {
        let dir = tempfile::tempdir().expect("应创建临时目录");
        let root = dir.path();
        fs::create_dir_all(root.join("a/empty")).expect("应创建子目录");
        touch(&root.join("a"), "film.mp4", 2048);

        let outcome = scan_directory(root, &rules(&[], 0)).expect("应扫描成功");

        assert_eq!(kinds_of(&outcome, ScanKind::Media), ["a/film.mp4"]);
        assert_eq!(
            outcome
                .leftover_dirs
                .iter()
                .map(|path| path.to_string_lossy().to_string())
                .collect::<Vec<_>>(),
            ["a/empty"],
            "含媒体的祖先目录不算残留, 只报真正的空叶"
        );
    }

    #[test]
    fn undersized_media_becomes_unwanted_with_rule() {
        let dir = tempfile::tempdir().expect("应创建临时目录");
        let root = dir.path();
        touch(root, "clip.mp4", 64);
        touch(root, "pointer.strm", 4);
        touch(root, "big.mp4", 2048);

        let outcome = scan_directory(root, &rules(&[], 1024)).expect("应扫描成功");

        let unwanted: Vec<_> = outcome
            .files
            .iter()
            .filter(|file| file.kind == ScanKind::Unwanted)
            .collect();
        assert_eq!(unwanted.len(), 1, "只有低码率正片该进无效文件");
        assert_eq!(unwanted[0].path, "clip.mp4");
        assert_eq!(unwanted[0].rule.as_deref(), Some("undersized"));
        // 指针文件大小无意义, 不参与过滤.
        assert!(
            outcome
                .files
                .iter()
                .any(|file| file.path == "pointer.strm" && file.kind == ScanKind::Media)
        );
    }

    #[test]
    fn oversized_blacklist_hit_still_unwanted_with_pattern() {
        let dir = tempfile::tempdir().expect("应创建临时目录");
        let root = dir.path();
        touch(root, "ads-banner.mp4", 4096);
        touch(root, "clean.mp4", 4096);

        let outcome = scan_directory(root, &rules(&["(?i)ads"], 1024)).expect("应扫描成功");

        let unwanted: Vec<_> = outcome
            .files
            .iter()
            .filter(|file| file.kind == ScanKind::Unwanted)
            .collect();
        assert_eq!(unwanted.len(), 1);
        assert_eq!(unwanted[0].path, "ads-banner.mp4");
        assert_eq!(unwanted[0].rule.as_deref(), Some("blacklist"));
        assert_eq!(
            unwanted[0].detail.as_deref(),
            Some("(?i)ads"),
            "界面要能显示命中的规则原文"
        );
    }

    #[test]
    fn system_artifacts_and_trash_are_pruned() {
        let dir = tempfile::tempdir().expect("应创建临时目录");
        let root = dir.path();
        touch(root, ".DS_Store", 16);
        touch(root, "._video.mp4", 16);
        touch(root, "Thumbs.db", 16);
        fs::create_dir_all(root.join(".amane_trash/deleted")).expect("应创建回收目录");
        touch(&root.join(".amane_trash/deleted"), "old.mp4", 2048);
        touch(root, "real.mp4", 2048);

        let outcome = scan_directory(root, &rules(&[], 0)).expect("应扫描成功");

        let paths: Vec<&str> = outcome
            .files
            .iter()
            .map(|file| file.path.as_str())
            .collect();
        assert!(paths.contains(&".DS_Store"));
        assert!(paths.contains(&"._video.mp4"));
        assert!(paths.contains(&"Thumbs.db"));
        assert!(
            !paths.iter().any(|path| path.contains(".amane_trash")),
            "回收目录整个子树不入清单"
        );
        assert!(paths.contains(&"real.mp4"));
    }

    #[test]
    fn normalization_adds_leading_dot_and_lowercases() {
        assert_eq!(normalize_extension(" MP4 "), ".mp4");
        assert_eq!(normalize_extension(".ISO"), ".iso");
    }
}
