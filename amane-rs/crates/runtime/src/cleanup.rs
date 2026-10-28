//! 清理任务: 按清单把无效文件与系统产物移入回收目录; 清单为空时不动任何文件.

use std::sync::Arc;

use amane_core::{LibraryRepository, TaskContext, TaskError, TaskHandler, TaskKind, TaskOutcome};
use async_trait::async_trait;
use serde::Deserialize;

/// 清理任务的载荷.
#[derive(Debug, Deserialize)]
struct CleanupPayload {
    library_id: i64,
}

/// 把清理任务接到库登记上.
pub struct CleanupHandler {
    libraries: LibraryRepository,
}

impl CleanupHandler {
    pub fn new(libraries: LibraryRepository) -> Arc<Self> {
        Arc::new(Self { libraries })
    }
}

#[async_trait]
impl TaskHandler for CleanupHandler {
    fn kind(&self) -> TaskKind {
        TaskKind::Cleanup
    }

    async fn run(&self, context: TaskContext) -> Result<TaskOutcome, TaskError> {
        let payload: CleanupPayload = serde_json::from_value(context.payload.clone())
            .map_err(|error| TaskError::Failed(format!("清理载荷不合法: {error}")))?;
        let root = self
            .libraries
            .find(payload.library_id)
            .await
            .map_err(|error| TaskError::Failed(format!("查询媒体库失败: {error}")))?
            .ok_or_else(|| TaskError::Failed(format!("媒体库 {} 不存在", payload.library_id)))?;

        // 清单在执行时重建: 与界面上看到的来自同一张扫描索引, 不会删掉刚登记的内容.
        let index = self
            .libraries
            .all_files(payload.library_id, None)
            .await
            .map_err(|error| TaskError::Failed(format!("读取扫描索引失败: {error}")))?;
        let manifest = amane_core::build_cleanup_manifest(&index, &root.path_buf());

        if manifest.is_empty() {
            context.logger.info("清单为空, 没有可清理的文件").await;
            return Ok(TaskOutcome::done());
        }
        for item in &manifest {
            context
                .logger
                .info(format!(
                    "待清理 {} ({} 字节): {}",
                    item.path, item.size, item.reason
                ))
                .await;
        }

        // 移动是纯阻塞 IO, 交给阻塞线程池.
        // 批量移动一旦开始就不再中断: 半个库搬完比不搬更难收拾, 因此取消只在边界生效.
        if context.cancel.is_cancelled() {
            return Err(TaskError::Cancelled);
        }
        let root_path = root.path_buf();
        let items = manifest.clone();
        let (moved, failures) =
            tokio::task::spawn_blocking(move || amane_core::execute_cleanup(&root_path, &items))
                .await
                .map_err(|error| TaskError::Failed(format!("清理线程异常退出: {error}")))?;

        for path in &moved {
            context
                .logger
                .info(format!("已移入回收目录 → {path}"))
                .await;
        }
        for (path, reason) in &failures {
            context.logger.warn(format!("跳过 {path}: {reason}")).await;
        }

        // 索引与磁盘保持一致: 已移走的条目从索引删除.
        let removed = self
            .libraries
            .delete_files(root.id, &moved)
            .await
            .map_err(|error| TaskError::Failed(format!("更新扫描索引失败: {error}")))?;
        context
            .logger
            .info(format!(
                "清理完成: 移入回收 {}、跳过 {}、索引删除 {}",
                moved.len(),
                failures.len(),
                removed
            ))
            .await;

        Ok(TaskOutcome::done())
    }
}

#[cfg(test)]
mod tests {
    use std::fs;
    use std::sync::Arc;

    use amane_core::library::LibraryRepository;
    use amane_core::organize::TRASH_DIRNAME;
    use amane_core::{
        Database, TaskContext, TaskError, TaskHandler, TaskKind, TaskLogger, TaskOutcome,
    };
    use tokio_util::sync::CancellationToken;

    use super::CleanupHandler;

    async fn fixture() -> (tempfile::TempDir, Arc<CleanupHandler>) {
        let dir = tempfile::tempdir().expect("应创建临时目录");
        let database = Database::open(&dir.path().join("amane.db"))
            .await
            .expect("应打开数据库");
        let libraries = LibraryRepository::new(database);
        let handler = CleanupHandler::new(libraries);
        (dir, handler)
    }

    async fn run(
        handler: &CleanupHandler,
        dir: &std::path::Path,
        payload: serde_json::Value,
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
    async fn empty_manifest_untouches_disk() {
        let (dir, handler) = fixture().await;
        let library = dir.path().join("library");
        fs::create_dir_all(&library).expect("应创建库目录");
        let database = Database::open(&dir.path().join("amane.db"))
            .await
            .expect("应打开数据库");
        let libraries = LibraryRepository::new(database.clone());
        let root = libraries.create(&library).await.expect("应登记成功");
        libraries
            .replace_files(root.id, &[], &[])
            .await
            .expect("应写入空索引");

        run(
            &handler,
            dir.path(),
            serde_json::json!({"library_id": root.id}),
        )
        .await
        .expect("空清单应成功");

        assert!(
            !library.join(TRASH_DIRNAME).exists(),
            "空清单不得创建回收目录"
        );
    }

    #[tokio::test]
    async fn cleans_unwanted_files_and_updates_index() {
        let (dir, handler) = fixture().await;
        let library = dir.path().join("library");
        fs::create_dir_all(&library).expect("应创建库目录");
        let database = Database::open(&dir.path().join("amane.db"))
            .await
            .expect("应打开数据库");
        let libraries = LibraryRepository::new(database.clone());
        let root = libraries.create(&library).await.expect("应登记成功");
        fs::write(library.join("ads.mp4"), b"x").expect("应写入无效文件");
        fs::write(library.join("film.mp4"), b"v").expect("应写入媒体");
        libraries
            .replace_files(
                root.id,
                &[
                    amane_core::LibraryFile::unwanted(
                        "ads.mp4",
                        amane_core::UnwantedRule::Blacklist,
                        "(?i)ads".to_owned(),
                    ),
                    amane_core::LibraryFile::new("film.mp4", amane_core::ScanKind::Media),
                ],
                &[],
            )
            .await
            .expect("应写入索引");

        run(
            &handler,
            dir.path(),
            serde_json::json!({"library_id": root.id}),
        )
        .await
        .expect("应完成");

        assert!(!library.join("ads.mp4").exists(), "原路径不再存在");
        assert!(
            library.join(TRASH_DIRNAME).join("ads.mp4").exists(),
            "文件应进入回收目录"
        );
        assert!(library.join("film.mp4").exists(), "媒体不应被清理");

        // 索引与磁盘一致: unwanted 条目已删除, media 保留.
        let (_, files) = libraries
            .files(root.id, None, 100, 0)
            .await
            .expect("应查询成功");
        assert_eq!(files.len(), 1);
        assert_eq!(files[0].path, "film.mp4");
    }

    #[tokio::test]
    async fn missing_library_fails_with_reason() {
        let (dir, handler) = fixture().await;

        let result = run(&handler, dir.path(), serde_json::json!({"library_id": 99})).await;

        match result {
            Err(TaskError::Failed(message)) => {
                assert!(message.contains("不存在"), "实际 {message}")
            }
            other => panic!("不存在的库必须失败, 实际 {other:?}"),
        }
    }

    #[tokio::test]
    async fn kind_is_cleanup() {
        let (_dir, handler) = fixture().await;
        assert_eq!(handler.kind(), TaskKind::Cleanup);
    }
}
