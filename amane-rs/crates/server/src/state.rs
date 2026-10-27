use std::sync::Arc;

use amane_core::config::Settings;
use amane_core::{Database, MetadataRepository};

/// 一个进程内运行时的全部依赖.
///
/// 热配置变更时整体重建本结构并原子替换, 因此字段之间不存在"半新半旧"的组合.
pub struct Runtime {
    pub settings: Settings,
    pub database: Database,
    pub metadata: MetadataRepository,
}

impl Runtime {
    pub async fn boot(settings: Settings) -> Result<Self, amane_core::DbError> {
        std::fs::create_dir_all(&settings.cold.data_dir)?;
        let database = Database::open(&settings.cold.data_dir.join("amane.db")).await?;
        let metadata = MetadataRepository::new(database.clone());

        Ok(Self {
            settings,
            database,
            metadata,
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
