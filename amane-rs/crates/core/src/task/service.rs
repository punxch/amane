use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use thiserror::Error;
use tokio::sync::{Notify, Semaphore};
use tokio::task::JoinHandle;
use tokio_util::sync::CancellationToken;

use super::handler::{TaskContext, TaskError, TaskHandler, TaskOutcome};
use super::logger::TaskLogger;
use super::model::{Task, TaskKind, TaskSpec};
use super::repository::{TaskRepository, TaskRepositoryError};

/// 队列空闲时的轮询间隔. 入队会主动唤醒, 这里只兜底处理重启遗留与时间推进.
const IDLE_POLL: Duration = Duration::from_millis(500);

#[derive(Debug, Error)]
pub enum TaskServiceError {
    #[error(transparent)]
    Repository(#[from] TaskRepositoryError),
    #[error("任务 {0} 不在可取消状态")]
    NotCancellable(i64),
}

/// 任务运行时: 出队、并发控制、取消与收尾.
pub struct TaskService {
    repository: TaskRepository,
    handlers: HashMap<TaskKind, Arc<dyn TaskHandler>>,
    /// 当前并发上限. 出队循环每次领取前读取, 因此可以就地调整.
    concurrency: AtomicUsize,
    /// 信号量凭证数即上限; 调小要回收空闲凭证, 调大要补发凭证.
    permits: Arc<Semaphore>,
    log_root: PathBuf,
    notify: Notify,
    running: Mutex<HashMap<i64, CancellationToken>>,
    shutdown: CancellationToken,
}

impl TaskService {
    pub fn new(
        repository: TaskRepository,
        handlers: Vec<Arc<dyn TaskHandler>>,
        concurrency: usize,
        log_root: PathBuf,
    ) -> Arc<Self> {
        let handlers = handlers
            .into_iter()
            .map(|handler| (handler.kind(), handler))
            .collect();

        let concurrency = concurrency.max(1);
        Arc::new(Self {
            repository,
            handlers,
            concurrency: AtomicUsize::new(concurrency),
            permits: Arc::new(Semaphore::new(concurrency)),
            log_root,
            notify: Notify::new(),
            running: Mutex::new(HashMap::new()),
            shutdown: CancellationToken::new(),
        })
    }

    pub fn repository(&self) -> &TaskRepository {
        &self.repository
    }

    pub fn log_root(&self) -> &std::path::Path {
        &self.log_root
    }

    pub async fn submit(&self, spec: TaskSpec) -> Result<Task, TaskServiceError> {
        let task = self.repository.enqueue(&spec).await?;
        self.notify.notify_one();
        Ok(task)
    }

    pub async fn retry(&self, id: i64) -> Result<Task, TaskServiceError> {
        let task = self.repository.retry(id).await?;
        self.notify.notify_one();
        Ok(task)
    }

    /// 取消任务: 排队中的直接置为终态, 运行中的经令牌通知处理函数.
    pub async fn cancel(&self, id: i64) -> Result<Task, TaskServiceError> {
        let running = self
            .running
            .lock()
            .ok()
            .and_then(|tasks| tasks.get(&id).cloned());

        if let Some(token) = running {
            token.cancel();
            // 处理函数在等待点响应, 通常几毫秒内落终态; 有界等待后仍返回当时的记录.
            for _ in 0..50 {
                let task = self
                    .repository
                    .find(id)
                    .await?
                    .ok_or(TaskServiceError::NotCancellable(id))?;
                if task.status.is_terminal() {
                    return Ok(task);
                }
                tokio::time::sleep(Duration::from_millis(20)).await;
            }
            return self
                .repository
                .find(id)
                .await?
                .ok_or(TaskServiceError::NotCancellable(id));
        }

        if self.repository.cancel_queued(id).await? {
            return self
                .repository
                .find(id)
                .await?
                .ok_or(TaskServiceError::NotCancellable(id));
        }

        Err(TaskServiceError::NotCancellable(id))
    }

    pub fn shutdown(&self) {
        self.shutdown.cancel();
    }

    /// 就地调整并发上限. 运行中的任务照原样跑完, 不影响它们的收尾.
    pub fn set_concurrency(&self, target: usize) {
        let target = target.max(1);
        let current = self.concurrency.swap(target, Ordering::SeqCst);
        if target > current {
            self.permits.add_permits(target - current);
        } else if current > target {
            // 只回收此刻空闲的凭证; 已被运行中的任务持有的不追回.
            self.permits.forget_permits(current - target);
        }
    }

    /// 启动出队循环. 返回句柄用于在停机时等待循环退出.
    pub fn start(self: &Arc<Self>) -> JoinHandle<()> {
        let service = Arc::clone(self);
        tokio::spawn(async move { service.dispatch_loop().await })
    }

    async fn dispatch_loop(self: Arc<Self>) {
        loop {
            tokio::select! {
                () = self.shutdown.cancelled() => break,
                () = self.notify.notified() => {}
                () = tokio::time::sleep(IDLE_POLL) => {}
            }
            self.dispatch_once(Arc::clone(&self.permits)).await;
        }
    }

    async fn dispatch_once(self: &Arc<Self>, permits: Arc<Semaphore>) {
        loop {
            let Ok(permit) = permits.clone().try_acquire_owned() else {
                break;
            };

            match self.repository.claim_next().await {
                Ok(Some(task)) => {
                    let service = Arc::clone(self);
                    tokio::spawn(async move {
                        service.run_task(task).await;
                        drop(permit);
                    });
                }
                Ok(None) => {
                    drop(permit);
                    break;
                }
                Err(error) => {
                    tracing::error!(%error, "领取任务失败");
                    drop(permit);
                    break;
                }
            }
        }
    }

    async fn run_task(self: Arc<Self>, task: Task) {
        let task_id = task.id;
        let token = CancellationToken::new();
        if let Ok(mut running) = self.running.lock() {
            running.insert(task_id, token.clone());
        }

        let outcome = self.execute(&task, token.clone()).await;

        if let Ok(mut running) = self.running.lock() {
            running.remove(&task_id);
        }

        // 取消与完成可能同时发生: 令牌已触发时以取消为准, 成功与失败两种终结都如此,
        // 否则处理器恰在取消瞬间报错会落成 failed.
        if token.is_cancelled() {
            self.settle_cancelled(task_id).await;
            return;
        }

        match outcome {
            Ok(outcome) => match self.repository.complete(task_id, &outcome.followups).await {
                Ok(_) => self.notify.notify_one(),
                Err(error) => tracing::error!(task = task_id, %error, "写入任务完成状态失败"),
            },
            Err(TaskError::Cancelled) => self.settle_cancelled(task_id).await,
            Err(TaskError::Failed(message)) => {
                if let Err(error) = self.repository.fail(task_id, &message).await {
                    tracing::error!(task = task_id, %error, "写入任务失败状态失败");
                }
            }
        }
    }

    async fn execute(
        &self,
        task: &Task,
        token: CancellationToken,
    ) -> Result<TaskOutcome, TaskError> {
        let logger = TaskLogger::open(&self.log_root, task.id)
            .await
            .map_err(|error| TaskError::Failed(format!("创建任务日志失败: {error}")))?;

        let Some(handler) = self.handlers.get(&task.kind).cloned() else {
            return Err(TaskError::Failed(format!(
                "任务类型 {} 没有注册处理函数",
                task.kind
            )));
        };

        let payload = serde_json::from_str(&task.payload)
            .map_err(|error| TaskError::Failed(format!("任务载荷不是合法 JSON: {error}")))?;
        let context = TaskContext {
            task_id: task.id,
            payload,
            logger,
            cancel: token,
        };

        handler.run(context).await
    }

    async fn settle_cancelled(&self, task_id: i64) {
        if let Err(error) = self.repository.mark_cancelled(task_id).await {
            tracing::error!(task = task_id, %error, "写入任务取消状态失败");
        }
    }
}

#[cfg(test)]
mod tests {
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::time::Duration;

    use async_trait::async_trait;
    use tokio_util::sync::CancellationToken;

    use super::{TaskService, TaskServiceError};
    use crate::db::Database;
    use crate::task::repository::TaskRepository;
    use crate::task::{
        Task, TaskContext, TaskError, TaskHandler, TaskKind, TaskOutcome, TaskSpec, TaskStatus,
    };

    type Run = Box<dyn Fn(TaskContext) -> BoxFuture + Send + Sync>;
    type BoxFuture =
        std::pin::Pin<Box<dyn std::future::Future<Output = Result<TaskOutcome, TaskError>> + Send>>;

    struct FnHandler {
        kind: TaskKind,
        run: Run,
    }

    impl FnHandler {
        fn new<F, Fut>(kind: TaskKind, run: F) -> std::sync::Arc<Self>
        where
            F: Fn(TaskContext) -> Fut + Send + Sync + 'static,
            Fut: std::future::Future<Output = Result<TaskOutcome, TaskError>> + Send + 'static,
        {
            std::sync::Arc::new(Self {
                kind,
                run: Box::new(move |context| Box::pin(run(context))),
            })
        }
    }

    #[async_trait]
    impl TaskHandler for FnHandler {
        fn kind(&self) -> TaskKind {
            self.kind
        }

        async fn run(&self, context: TaskContext) -> Result<TaskOutcome, TaskError> {
            (self.run)(context).await
        }
    }

    struct Harness {
        _dir: tempfile::TempDir,
        service: std::sync::Arc<TaskService>,
        repository: TaskRepository,
    }

    async fn harness(
        handlers: Vec<std::sync::Arc<dyn TaskHandler>>,
        concurrency: usize,
    ) -> Harness {
        let dir = tempfile::tempdir().expect("应创建临时目录");
        let database = Database::open(&dir.path().join("amane.db"))
            .await
            .expect("应打开数据库");
        let repository = TaskRepository::new(database);
        repository.recover_running().await.expect("应恢复遗留任务");
        let service = TaskService::new(
            repository.clone(),
            handlers,
            concurrency,
            dir.path().join("tasks"),
        );
        service.start();

        Harness {
            _dir: dir,
            service,
            repository,
        }
    }

    fn scrape(number: &str) -> TaskSpec {
        TaskSpec::new(TaskKind::Scrape, serde_json::json!({ "number": number }))
    }

    async fn wait_terminal(repository: &TaskRepository, id: i64) -> Task {
        for _ in 0..300 {
            let task = repository
                .find(id)
                .await
                .expect("应查询成功")
                .expect("应存在");
            if task.status.is_terminal() {
                return task;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
        panic!("任务 {id} 未在预期时间内进入终态");
    }

    #[tokio::test]
    async fn runs_handler_and_marks_done() {
        let handler = FnHandler::new(TaskKind::Scrape, |context| async move {
            context.logger.info("处理完成").await;
            Ok(TaskOutcome::done())
        });
        let harness = harness(vec![handler], 2).await;

        let task = harness
            .service
            .submit(scrape("ABC-001"))
            .await
            .expect("应入队");
        let settled = wait_terminal(&harness.repository, task.id).await;

        assert_eq!(settled.status, TaskStatus::Done);
        assert!(settled.finished_at.is_some());
    }

    #[tokio::test]
    async fn handler_failure_marks_failed_with_reason() {
        let handler = FnHandler::new(TaskKind::Scrape, |_context| async {
            Err(TaskError::Failed("来源未收录".to_owned()))
        });
        let harness = harness(vec![handler], 1).await;

        let task = harness
            .service
            .submit(scrape("ZZZ-999"))
            .await
            .expect("应入队");
        let settled = wait_terminal(&harness.repository, task.id).await;

        assert_eq!(settled.status, TaskStatus::Failed);
        assert_eq!(settled.error.as_deref(), Some("来源未收录"));
    }

    #[tokio::test]
    async fn unregistered_kind_fails_with_explicit_message() {
        let harness = harness(Vec::new(), 1).await;

        let task = harness
            .service
            .submit(scrape("ABC-001"))
            .await
            .expect("应入队");
        let settled = wait_terminal(&harness.repository, task.id).await;

        assert_eq!(settled.status, TaskStatus::Failed);
        assert!(
            settled
                .error
                .unwrap_or_default()
                .contains("没有注册处理函数"),
            "错误说明应指出缺少处理函数"
        );
    }

    #[tokio::test]
    async fn cancel_stops_running_task() {
        let handler = FnHandler::new(TaskKind::Scrape, |context| async move {
            let token: CancellationToken = context.cancel.clone();
            tokio::select! {
                () = token.cancelled() => Err(TaskError::Cancelled),
                () = tokio::time::sleep(Duration::from_secs(30)) => Ok(TaskOutcome::done()),
            }
        });
        let harness = harness(vec![handler], 1).await;

        let task = harness
            .service
            .submit(scrape("ABC-001"))
            .await
            .expect("应入队");
        tokio::time::sleep(Duration::from_millis(80)).await;
        harness.service.cancel(task.id).await.expect("应请求取消");
        let settled = wait_terminal(&harness.repository, task.id).await;

        assert_eq!(settled.status, TaskStatus::Cancelled);
    }

    #[tokio::test]
    async fn cancel_wins_over_failure() {
        // 处理器在取消生效后仍返回失败: 终态必须是已取消.
        let handler = FnHandler::new(TaskKind::Scrape, |context| async move {
            tokio::time::sleep(Duration::from_millis(60)).await;
            if context.cancel.is_cancelled() {
                return Err(TaskError::Failed("来源报错".to_owned()));
            }
            Ok(TaskOutcome::done())
        });
        let harness = harness(vec![handler], 1).await;

        let task = harness
            .service
            .submit(scrape("ABC-001"))
            .await
            .expect("应入队");
        tokio::time::sleep(Duration::from_millis(20)).await;
        harness.service.cancel(task.id).await.expect("应请求取消");
        let settled = wait_terminal(&harness.repository, task.id).await;

        assert_eq!(settled.status, TaskStatus::Cancelled);
    }

    #[tokio::test]
    async fn cancel_rejects_finished_task() {
        let handler = FnHandler::new(TaskKind::Scrape, |_context| async {
            Ok(TaskOutcome::done())
        });
        let harness = harness(vec![handler], 1).await;

        let task = harness
            .service
            .submit(scrape("ABC-001"))
            .await
            .expect("应入队");
        wait_terminal(&harness.repository, task.id).await;

        let result = harness.service.cancel(task.id).await;

        assert!(
            matches!(result, Err(TaskServiceError::NotCancellable(id)) if id == task.id),
            "终态任务不可取消"
        );
    }

    #[tokio::test]
    async fn concurrency_can_be_raised_in_place() {
        let active = std::sync::Arc::new(AtomicUsize::new(0));
        let peak = std::sync::Arc::new(AtomicUsize::new(0));
        let handler = FnHandler::new(TaskKind::Scrape, {
            let active = std::sync::Arc::clone(&active);
            let peak = std::sync::Arc::clone(&peak);
            move |_context| {
                let active = std::sync::Arc::clone(&active);
                let peak = std::sync::Arc::clone(&peak);
                async move {
                    let now = active.fetch_add(1, Ordering::SeqCst) + 1;
                    peak.fetch_max(now, Ordering::SeqCst);
                    tokio::time::sleep(Duration::from_millis(120)).await;
                    active.fetch_sub(1, Ordering::SeqCst);
                    Ok(TaskOutcome::done())
                }
            }
        });
        let harness = harness(vec![handler], 1).await;

        harness.service.set_concurrency(2);
        harness
            .service
            .submit(scrape("AAA-001"))
            .await
            .expect("应入队");
        harness
            .service
            .submit(scrape("BBB-002"))
            .await
            .expect("应入队");
        tokio::time::sleep(Duration::from_millis(400)).await;

        assert_eq!(
            peak.load(Ordering::SeqCst),
            2,
            "调大并发上限后, 两个任务应能同时运行"
        );
    }

    #[tokio::test]
    async fn concurrency_limit_serializes_tasks() {
        let active = std::sync::Arc::new(AtomicUsize::new(0));
        let peak = std::sync::Arc::new(AtomicUsize::new(0));
        let handler = FnHandler::new(TaskKind::Scrape, {
            let active = std::sync::Arc::clone(&active);
            let peak = std::sync::Arc::clone(&peak);
            move |_context| {
                let active = std::sync::Arc::clone(&active);
                let peak = std::sync::Arc::clone(&peak);
                async move {
                    let now = active.fetch_add(1, Ordering::SeqCst) + 1;
                    peak.fetch_max(now, Ordering::SeqCst);
                    tokio::time::sleep(Duration::from_millis(60)).await;
                    active.fetch_sub(1, Ordering::SeqCst);
                    Ok(TaskOutcome::done())
                }
            }
        });
        let harness = harness(vec![handler], 1).await;

        let first = harness
            .service
            .submit(scrape("AAA-001"))
            .await
            .expect("应入队");
        let second = harness
            .service
            .submit(scrape("BBB-001"))
            .await
            .expect("应入队");
        wait_terminal(&harness.repository, first.id).await;
        wait_terminal(&harness.repository, second.id).await;

        assert_eq!(
            peak.load(Ordering::SeqCst),
            1,
            "并发上限为 1 时不应并行执行"
        );
    }

    #[tokio::test]
    async fn followups_are_enqueued_on_completion() {
        let handler = FnHandler::new(TaskKind::Scrape, |_context| async {
            Ok(TaskOutcome::with_followups(vec![TaskSpec::new(
                TaskKind::Organize,
                serde_json::json!({ "number": "ABC-001" }),
            )]))
        });
        let harness = harness(vec![handler], 1).await;

        let parent = harness
            .service
            .submit(scrape("ABC-001"))
            .await
            .expect("应入队");
        wait_terminal(&harness.repository, parent.id).await;

        let children = harness
            .repository
            .children(parent.id)
            .await
            .expect("应查询成功");

        assert_eq!(children.len(), 1, "完成后应留下一条后继任务");
        assert_eq!(children[0].kind, TaskKind::Organize);
    }
}
