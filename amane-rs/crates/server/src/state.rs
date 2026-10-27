use std::sync::Arc;

use amane_core::config::Settings;
use amane_core::{
    Database, DbError, MetadataRepository, TaskHandler, TaskRepository, TaskRepositoryError,
    TaskService,
};
use amane_crawlers::Registry;
use amane_runtime::ScrapeHandler;

/// 启动失败的原因.
#[derive(Debug, thiserror::Error)]
pub enum BootError {
    #[error("创建数据目录失败: {0}")]
    Io(#[from] std::io::Error),
    #[error(transparent)]
    Database(#[from] DbError),
    #[error(transparent)]
    Task(#[from] TaskRepositoryError),
}

/// 一个进程内运行时的全部依赖.
///
/// 热配置变更时整体重建本结构并原子替换, 因此字段之间不存在半新半旧的组合.
pub struct Runtime {
    pub settings: Settings,
    pub database: Database,
    pub metadata: MetadataRepository,
    pub tasks: Arc<TaskService>,
}

impl Runtime {
    pub async fn boot(settings: Settings) -> Result<Self, BootError> {
        std::fs::create_dir_all(&settings.cold.data_dir)?;
        let database = Database::open(&settings.cold.data_dir.join("amane.db")).await?;
        let metadata = MetadataRepository::new(database.clone());

        let task_repository = TaskRepository::new(database.clone());
        // 上次进程遗留的运行中任务必须回到队列, 否则它们永远不会被领取.
        let recovered = task_repository.recover_running().await?;
        if recovered > 0 {
            tracing::warn!(count = recovered, "上次进程遗留的任务已放回队列");
        }

        let sources = Arc::new(Registry::builtin());
        let handlers: Vec<Arc<dyn TaskHandler>> =
            vec![ScrapeHandler::new(Arc::clone(&sources), metadata.clone())];
        let tasks = TaskService::new(
            task_repository,
            handlers,
            settings.hot.concurrency as usize,
            settings.cold.data_dir.join("tasks"),
        );

        Ok(Self {
            settings,
            database,
            metadata,
            tasks,
        })
    }
}

#[derive(Clone)]
pub struct AppState {
    pub runtime: Arc<Runtime>,
}

impl AppState {
    pub fn new(runtime: Runtime) -> Self {
        Self {
            runtime: Arc::new(runtime),
        }
    }
}
