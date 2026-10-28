use std::sync::Arc;
use std::time::Duration;

use arc_swap::ArcSwap;

use amane_core::config::{ConfigError, Settings};
use amane_core::{
    Database, DbError, LibraryRepository, MetadataRepository, OrganizeRepository,
    ResourceRepository, ScheduleRepository, Scheduler, TaskHandler, TaskRepository,
    TaskRepositoryError, TaskService,
};
use amane_crawlers::Registry;
use amane_net::{Rate, RateLimiters, WebClient};
use amane_runtime::{
    AssetsHandler, CleanupHandler, OrganizeHandler, ResourceStore, ScanHandler, ScrapeHandler,
};

/// 启动失败的原因.
#[derive(Debug, thiserror::Error)]
pub enum BootError {
    #[error("创建数据目录失败: {0}")]
    Io(#[from] std::io::Error),
    #[error(transparent)]
    Database(#[from] DbError),
    #[error(transparent)]
    Task(#[from] TaskRepositoryError),
    #[error("构造 HTTP 客户端失败: {0}")]
    Network(String),
}

/// 一个进程内运行时的全部依赖.
///
/// 热配置变更就地应用到各组件; 配置整体存于 `ArcSwap`, 读方拿到某一时刻的完整快照.
pub struct Runtime {
    pub database: Database,
    pub metadata: MetadataRepository,
    pub tasks: Arc<TaskService>,
    pub resources: Arc<ResourceStore>,
    /// 删除条目的入口: 数据库事务与文件清理都在它后面.
    pub removal: amane_core::MetadataRemoval,
    /// 整理预览的装配: HTTP 端点与整理任务共用.
    pub preview: amane_core::PreviewAssembler,
    /// 来源登记表. 处理函数用它获取, 筛选端点用它给出可选项.
    pub sources: Arc<Registry>,
    /// 定时任务登记与调度器. 循环由进程入口启动.
    pub schedules: ScheduleRepository,
    pub scheduler: Arc<Scheduler>,
    /// 串行化热配置的应用: 并发的保存请求基于同一份快照依次生效.
    apply_lock: tokio::sync::Mutex<()>,
    /// 出站客户端. 来源与资源下载共用同一份, 因此改超时会同时影响两侧.
    pub client: Arc<WebClient>,
    /// 当前配置. 热配置变更时整体替换, 冷配置只有重启后才可能变化.
    /// 任务处理器持有一份, 扫描等在开始时读取快照.
    settings: Arc<ArcSwap<Settings>>,
    /// 媒体库登记与扫描索引.
    pub libraries: LibraryRepository,
    /// 整理登记. 预览判「同名不同番号」的依据, 执行切片负责写入.
    pub organize: OrganizeRepository,
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

        // 限速器先于客户端构造: 客户端持有分桶状态, 反过来无法通过编译.
        let limiters = RateLimiters::new(Rate::per_second(2, 1.0))
            .with_override(amane_crawlers::dmm::HOST, Rate::per_second(2, 2.0));
        let client = WebClient::new(
            Duration::from_secs(u64::from(settings.hot.request_timeout_secs)),
            limiters,
        )
        .map_err(|error| BootError::Network(error.to_string()))?;

        let client = Arc::new(client);
        let resources = Arc::new(ResourceStore::new(
            settings.cold.data_dir.join("resources"),
            ResourceRepository::new(database.clone()),
            Arc::clone(&client),
        ));
        let registry = Registry::builtin(Arc::clone(&client));
        // 热配置里的来源顺序即聚合优先级; 为空表示保持内置顺序.
        registry.apply_enabled_order(&settings.hot.enabled_sources);
        let sources = Arc::new(registry);
        let libraries = LibraryRepository::new(database.clone());
        let organize = OrganizeRepository::new(database.clone());
        let settings = Arc::new(ArcSwap::from_pointee(settings));
        // 预览装配在 HTTP 端点与整理任务之间共用, 先于处理函数表构造.
        let preview = amane_core::PreviewAssembler::new(
            libraries.clone(),
            metadata.clone(),
            organize.clone(),
        );
        let handlers: Vec<Arc<dyn TaskHandler>> = vec![
            ScrapeHandler::new(Arc::clone(&sources), metadata.clone()),
            AssetsHandler::new(metadata.clone(), Arc::clone(&resources)),
            ScanHandler::new(libraries.clone(), Arc::clone(&settings)),
            OrganizeHandler::new(
                libraries.clone(),
                preview.clone(),
                organize.clone(),
                Arc::clone(&settings),
            ),
            CleanupHandler::new(libraries.clone()),
        ];
        let tasks = TaskService::new(
            task_repository,
            handlers,
            settings.load().hot.concurrency as usize,
            settings.load().cold.data_dir.join("tasks"),
        );
        let schedules = ScheduleRepository::new(database.clone());
        let scheduler = Arc::new(Scheduler::new(schedules.clone(), Arc::clone(&tasks)));
        let removal = amane_core::MetadataRemoval::new(
            metadata.clone(),
            Arc::clone(&resources) as Arc<dyn amane_core::ResourceFiles>,
        );

        Ok(Self {
            settings,
            database,
            metadata,
            tasks,
            resources,
            removal,
            preview,
            sources,
            schedules,
            scheduler,
            client,
            apply_lock: tokio::sync::Mutex::new(()),
            libraries,
            organize,
        })
    }

    /// 当前配置.
    pub fn settings(&self) -> Arc<Settings> {
        self.settings.load_full()
    }

    /// 冷配置中发生变化的字段名, 供界面提示重启.
    pub fn cold_diff(&self, incoming: &Settings) -> Vec<String> {
        let current = self.settings();
        let mut changed = Vec::new();

        if current.cold.data_dir != incoming.cold.data_dir {
            changed.push("data_dir".to_owned());
        }
        if current.cold.bind != incoming.cold.bind {
            changed.push("bind".to_owned());
        }
        if current.cold.api_token != incoming.cold.api_token {
            changed.push("api_token".to_owned());
        }

        changed
    }

    /// 应用热配置. 三项都就地生效: 正在运行的任务照原样运行完毕, 不受影响.
    ///
    /// 冷配置不在这里处理: 数据目录与监听地址等改动要重启进程才可能生效.
    pub async fn apply_hot(&self, incoming: &Settings) -> Result<Vec<String>, ConfigError> {
        // 并发的保存请求串行应用: 比较与生效都基于同一次取得的快照.
        let _guard = self.apply_lock.lock().await;
        // 白名单取全部登记来源: 取启用集的话, 停用后再启用会被判成未知来源.
        let known = self.sources.known_ids();
        incoming.hot.validate(&known)?;

        let mut applied = Vec::new();
        if incoming.hot.request_timeout_secs != self.settings().hot.request_timeout_secs {
            self.client
                .set_timeout(Duration::from_secs(u64::from(
                    incoming.hot.request_timeout_secs,
                )))
                .await
                .map_err(|error| ConfigError::Submit(error.to_string()))?;
            applied.push("request_timeout_secs".to_owned());
        }
        if incoming.hot.enabled_sources != self.settings().hot.enabled_sources {
            self.sources
                .apply_enabled_order(&incoming.hot.enabled_sources);
            applied.push("enabled_sources".to_owned());
        }
        if incoming.hot.concurrency != self.settings().hot.concurrency {
            self.tasks
                .set_concurrency(incoming.hot.concurrency as usize);
            applied.push("concurrency".to_owned());
        }
        if incoming.hot.scan != self.settings().hot.scan {
            // 扫描规则在每次扫描开始时读取快照, 存储即生效.
            applied.push("scan".to_owned());
        }
        if incoming.hot.organize != self.settings().hot.organize {
            // 模板在保存时已解析校验, 预览与执行读取快照即生效.
            applied.push("organize".to_owned());
        }

        // 只替换热层: 提交里的冷配置改动不在内存中生效, 要等重启进程.
        let mut stored = incoming.clone();
        stored.cold = self.settings().cold.clone();
        self.settings.store(Arc::new(stored));

        Ok(applied)
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
