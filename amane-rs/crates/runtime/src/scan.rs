//! 扫描任务: 遍历一个媒体库根目录, 归类文件并记录相对上次的增量.

use std::sync::Arc;

use amane_core::library::{KindCounts, ScanRules};
use amane_core::{
    LibraryError, LibraryFile, LibraryRepository, ScanError, TaskContext, TaskError, TaskHandler,
    TaskKind, TaskOutcome, TaskPayload,
};
use arc_swap::ArcSwap;
use async_trait::async_trait;

use amane_core::config::Settings;

/// 把扫描任务接到媒体库仓储与配置快照上.
///
/// 规则在每次扫描开始时从配置快照编译: 热配置改动不需要通知本处理器,
/// 下一次扫描自然使用新规则.
pub struct ScanHandler {
    libraries: LibraryRepository,
    settings: Arc<ArcSwap<Settings>>,
}

impl ScanHandler {
    pub fn new(libraries: LibraryRepository, settings: Arc<ArcSwap<Settings>>) -> Arc<Self> {
        Arc::new(Self {
            libraries,
            settings,
        })
    }

    /// 由配置快照编译扫描规则.
    fn rules(settings: &Settings) -> ScanRules {
        ScanRules::from_parts(
            settings.hot.scan.media_extensions.iter(),
            settings.hot.scan.min_file_size,
            settings
                .hot
                .scan
                .blacklist_patterns
                .iter()
                .filter_map(|pattern| regex::Regex::new(pattern).ok())
                .collect(),
        )
    }
}

#[async_trait]
impl TaskHandler for ScanHandler {
    fn kind(&self) -> TaskKind {
        TaskKind::Scan
    }

    async fn run(&self, context: TaskContext) -> Result<TaskOutcome, TaskError> {
        let TaskPayload::Scan { library_id } = context.payload else {
            return Err(TaskError::Failed("载荷类型与任务类型不匹配".to_owned()));
        };
        let root = self
            .libraries
            .require(library_id)
            .await
            .map_err(|error| TaskError::Failed(error.to_string()))?;

        let rules = Self::rules(&self.settings.load_full());
        context.logger.info(format!("开始扫描 {}", root.path)).await;

        // 遍历是纯阻塞 IO, 交给阻塞线程池, 不占异步工作线程.
        let root_path = root.path_buf();
        let outcome =
            tokio::task::spawn_blocking(move || amane_core::scan_directory(&root_path, &rules))
                .await
                .map_err(|error| TaskError::Failed(format!("扫描线程异常退出: {error}")))?
                .map_err(|error: ScanError| TaskError::Failed(error.to_string()))?;

        let diff = self
            .libraries
            .replace_files(root.id, &outcome.files, &outcome.leftover_dirs)
            .await
            .map_err(|error| match error {
                LibraryError::NotFound(id) => {
                    TaskError::Failed(format!("媒体库 {id} 已被移除, 放弃本次结果"))
                }
                other => TaskError::Failed(format!("写入扫描索引失败: {other}")),
            })?;

        let counts = KindCounts {
            media: count_of(&outcome.files, amane_core::ScanKind::Media),
            unwanted: count_of(&outcome.files, amane_core::ScanKind::Unwanted),
            system: count_of(&outcome.files, amane_core::ScanKind::System),
            unrecognized: count_of(&outcome.files, amane_core::ScanKind::Unrecognized),
            leftover_dir: outcome.leftover_dirs.len() as i64,
        };
        context
            .logger
            .info(format!(
                "扫描完成: 媒体 {}、无效 {}、系统产物 {}、未识别 {}、残留目录 {}",
                counts.media,
                counts.unwanted,
                counts.system,
                counts.unrecognized,
                counts.leftover_dir
            ))
            .await;
        context
            .logger
            .info(format!(
                "相对上次: 新增 {}、移除 {}",
                diff.added.len(),
                diff.removed.len()
            ))
            .await;
        for path in diff.added.iter().take(20) {
            context.logger.info(format!("新增 {path}")).await;
        }
        for path in diff.removed.iter().take(20) {
            context.logger.info(format!("移除 {path}")).await;
        }

        Ok(TaskOutcome::done())
    }
}

/// 按类别计数. 残留目录已在仓储层合并进同一张索引, 这里只计文件.
fn count_of(files: &[LibraryFile], kind: amane_core::ScanKind) -> i64 {
    i64::try_from(files.iter().filter(|file| file.kind == kind).count()).unwrap_or(i64::MAX)
}

#[cfg(test)]
mod tests {
    use std::fs;
    use std::path::Path;
    use std::sync::Arc;

    use amane_core::config::Settings;
    use amane_core::library::LibraryRepository;
    use amane_core::{
        Database, ScanKind, TaskContext, TaskError, TaskHandler, TaskKind, TaskLogger, TaskOutcome,
        TaskPayload,
    };
    use arc_swap::ArcSwap;
    use tokio_util::sync::CancellationToken;

    use super::ScanHandler;

    async fn fixture() -> (tempfile::TempDir, Arc<ScanHandler>, LibraryRepository) {
        let dir = tempfile::tempdir().expect("应创建临时目录");
        let database = Database::open(&dir.path().join("amane.db"))
            .await
            .expect("应打开数据库");
        let libraries = LibraryRepository::new(database.clone());
        let settings = Arc::new(ArcSwap::from_pointee(Settings::default()));
        let handler = ScanHandler::new(libraries.clone(), settings);

        (dir, handler, libraries)
    }

    async fn run(
        handler: &ScanHandler,
        dir: &Path,
        payload: TaskPayload,
    ) -> Result<TaskOutcome, TaskError> {
        let logger = TaskLogger::open(dir, 1).await.expect("应创建日志");
        handler
            .run(TaskContext {
                task_id: 1,
                payload,
                logger,
                cancel: CancellationToken::new(),
            })
            .await
    }

    #[tokio::test]
    async fn classifies_files_and_reports_diff() {
        let (dir, handler, libraries) = fixture().await;
        // 库目录与数据库分开放, 扫描只应看到库里的内容.
        let library = dir.path().join("library");
        fs::create_dir_all(&library).expect("应创建库目录");
        let root = libraries.insert(&library).await.expect("应登记成功");
        fs::write(library.join("film.mp4"), vec![0u8; 64]).expect("应写入媒体");
        fs::write(library.join("note.txt"), "说明").expect("应写入附属");
        fs::create_dir_all(library.join("empty")).expect("应创建空目录");

        run(
            &handler,
            dir.path(),
            TaskPayload::Scan {
                library_id: root.id,
            },
        )
        .await
        .expect("应完成");

        let (total, files) = libraries
            .files(root.id, None, 100, 0)
            .await
            .expect("应查询成功");
        assert_eq!(total, 3, "两个文件加一个残留目录");
        assert!(
            files
                .iter()
                .any(|file| file.path == "film.mp4" && file.kind == ScanKind::Media)
        );
        assert!(
            files
                .iter()
                .any(|file| file.path == "note.txt" && file.kind == ScanKind::Unrecognized)
        );
        assert!(
            files
                .iter()
                .any(|file| file.path == "empty" && file.kind == ScanKind::LeftoverDir)
        );
    }

    #[tokio::test]
    async fn second_scan_picks_up_add_and_remove() {
        let (dir, handler, libraries) = fixture().await;
        let library = dir.path().join("library");
        fs::create_dir_all(&library).expect("应创建库目录");
        let root = libraries.insert(&library).await.expect("应登记成功");
        fs::write(library.join("old.mp4"), vec![0u8; 64]).expect("应写入媒体");

        run(
            &handler,
            dir.path(),
            TaskPayload::Scan {
                library_id: root.id,
            },
        )
        .await
        .expect("首次扫描应完成");

        fs::remove_file(library.join("old.mp4")).expect("应删除媒体");
        fs::write(library.join("new.mkv"), vec![0u8; 64]).expect("应写入新媒体");

        run(
            &handler,
            dir.path(),
            TaskPayload::Scan {
                library_id: root.id,
            },
        )
        .await
        .expect("再次扫描应完成");

        let (_, files) = libraries
            .files(root.id, Some(ScanKind::Media), 100, 0)
            .await
            .expect("应查询成功");
        let paths: Vec<&str> = files.iter().map(|file| file.path.as_str()).collect();
        assert_eq!(paths, ["new.mkv"], "增量扫描应替换过期的媒体记录");
    }

    #[tokio::test]
    async fn applies_blacklist_from_settings() {
        let (dir, _handler, libraries) = fixture().await;
        let root = libraries.insert(dir.path()).await.expect("应登记成功");
        fs::write(dir.path().join("ads-promo.mp4"), vec![0u8; 2048]).expect("应写入媒体");

        // 未配置黑名单: 大于阈值的文件是媒体.
        run(
            &_handler,
            dir.path(),
            TaskPayload::Scan {
                library_id: root.id,
            },
        )
        .await
        .expect("应完成");
        let (_, files) = libraries
            .files(root.id, Some(ScanKind::Media), 10, 0)
            .await
            .expect("应查询成功");
        assert_eq!(files.len(), 1);

        // 配置黑名单与大小阈值后, 再扫描归类随之变化.
        let mut settings = Settings::default();
        settings.hot.scan.blacklist_patterns = vec!["(?i)ads".to_owned()];
        settings.hot.scan.min_file_size = 1024;
        let handler =
            ScanHandler::new(libraries.clone(), Arc::new(ArcSwap::from_pointee(settings)));

        run(
            &handler,
            dir.path(),
            TaskPayload::Scan {
                library_id: root.id,
            },
        )
        .await
        .expect("应完成");
        let (total, unwanted) = libraries
            .files(root.id, Some(ScanKind::Unwanted), 10, 0)
            .await
            .expect("应查询成功");
        assert_eq!(total, 1);
        assert_eq!(unwanted[0].rule.as_deref(), Some("blacklist"));
        assert_eq!(unwanted[0].detail.as_deref(), Some("(?i)ads"));
    }

    #[tokio::test]
    async fn missing_library_fails_with_reason() {
        let (dir, handler, _libraries) = fixture().await;

        let result = run(&handler, dir.path(), TaskPayload::Scan { library_id: 99 }).await;

        match result {
            Err(TaskError::Failed(message)) => {
                assert!(message.contains("不存在"), "应说明原因, 实际 {message}")
            }
            other => panic!("不存在的库必须失败, 实际 {other:?}"),
        }
    }

    #[tokio::test]
    async fn missing_directory_fails_with_reason() {
        let (dir, handler, libraries) = fixture().await;
        let gone = dir.path().join("gone");
        let root = libraries.insert(&gone).await.expect("应登记成功");

        let result = run(
            &handler,
            dir.path(),
            TaskPayload::Scan {
                library_id: root.id,
            },
        )
        .await;

        match result {
            Err(TaskError::Failed(message)) => {
                assert!(message.contains("读取目录"), "应说明原因, 实际 {message}")
            }
            other => panic!("目录消失必须失败, 实际 {other:?}"),
        }
    }

    #[tokio::test]
    async fn kind_is_scan() {
        let (_dir, handler, _libraries) = fixture().await;
        assert_eq!(handler.kind(), TaskKind::Scan);
    }
}
