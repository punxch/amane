use std::sync::Arc;

use chrono::{DateTime, Utc};

use super::model::{Pattern, Schedule, ScheduleError};
use super::repository::ScheduleRepository;
use crate::task::{TaskService, TaskSpec};

/// 每次扫描的间隔. cron 的最小粒度是分钟, 这个间隔保证触发至多迟到十秒.
const TICK_INTERVAL: std::time::Duration = std::time::Duration::from_secs(10);

/// 调度循环: 扫描启用的登记, 到点派生任务.
///
/// 基准时刻取登记的 `last_fired_at`, 派发后推进到当下. 因此进程停机或暂停期间
/// 错过的触发只派发最近一次, 不会逐个重放.
pub struct Scheduler {
    repository: ScheduleRepository,
    tasks: Arc<TaskService>,
}

impl Scheduler {
    pub fn new(repository: ScheduleRepository, tasks: Arc<TaskService>) -> Self {
        Self { repository, tasks }
    }

    /// 扫描一轮, 返回本轮派生的任务. `now` 由调用方传入, 测试可以指定时刻.
    ///
    /// 单条登记出错只记日志并继续: 一条坏登记不能挡住排在它后面的其它登记.
    pub async fn tick(&self, now: DateTime<Utc>) -> Result<Vec<i64>, ScheduleError> {
        let mut dispatched = Vec::new();

        for schedule in self.repository.list_enabled().await? {
            let dispatched_task = self.dispatch(&schedule, now).await;
            if let Err(error) = dispatched_task {
                tracing::error!(schedule = schedule.id, %error, "定时任务派生失败");
                continue;
            }
            if let Some(task_id) = dispatched_task.expect("错误分支已返回") {
                dispatched.push(task_id);
            }
        }

        Ok(dispatched)
    }

    /// 到点则派生并推进基准时刻; 未到点返回 `None`.
    async fn dispatch(
        &self,
        schedule: &Schedule,
        now: DateTime<Utc>,
    ) -> Result<Option<i64>, ScheduleError> {
        let pattern = Pattern::parse(&schedule.cron, &schedule.timezone)?;
        let anchor = schedule.last_fired_at.unwrap_or(now);
        let next = pattern.next_after(anchor)?;
        if next > now {
            return Ok(None);
        }

        let payload: serde_json::Value = serde_json::from_str(&schedule.payload)
            .map_err(|error| ScheduleError::InvalidPayload(error.to_string()))?;
        let task = self
            .tasks
            .submit(TaskSpec {
                kind: schedule.kind,
                payload,
                priority: 0,
                schedule_id: Some(schedule.id),
            })
            .await
            .map_err(|error| ScheduleError::Submit(error.to_string()))?;
        self.repository.mark_fired(schedule.id, now).await?;

        tracing::info!(
            schedule = schedule.id,
            task = task.id,
            cron = %schedule.cron,
            "定时任务已派生"
        );

        Ok(Some(task.id))
    }

    /// 启动扫描循环. 进程停止时由调用方结束返回的任务.
    pub fn spawn(self: Arc<Self>) -> tokio::task::JoinHandle<()> {
        tokio::spawn(async move {
            loop {
                if let Err(error) = self.tick(Utc::now()).await {
                    tracing::error!(%error, "定时任务扫描失败");
                }
                tokio::time::sleep(TICK_INTERVAL).await;
            }
        })
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use chrono::{Duration, Utc};

    use super::Scheduler;
    use crate::db::Database;
    use crate::schedule::model::{SchedulePatch, ScheduleSpec};
    use crate::schedule::repository::ScheduleRepository;
    use crate::task::{TaskKind, TaskRepository, TaskService};

    async fn scheduler() -> (
        tempfile::TempDir,
        Scheduler,
        ScheduleRepository,
        TaskRepository,
    ) {
        let dir = tempfile::tempdir().expect("应创建临时目录");
        let database = Database::open(&dir.path().join("amane.db"))
            .await
            .expect("应打开数据库");
        let task_repository = TaskRepository::new(database.clone());
        let tasks = Arc::new(TaskService::new(
            task_repository.clone(),
            Vec::new(),
            1,
            dir.path().join("tasks"),
        ));
        let repository = ScheduleRepository::new(database);

        (
            dir,
            Scheduler::new(repository.clone(), Arc::clone(&tasks)),
            repository,
            task_repository,
        )
    }

    fn spec(cron: &str) -> ScheduleSpec {
        ScheduleSpec {
            kind: TaskKind::Scrape,
            payload: serde_json::json!({ "number": "ABC-123" }),
            cron: cron.to_owned(),
            timezone: "Asia/Shanghai".to_owned(),
        }
    }

    #[tokio::test]
    async fn fires_due_schedule_and_records_origin() {
        let (_dir, scheduler, repository, task_repository) = scheduler().await;
        let row = repository
            .create(&spec("* * * * *"))
            .await
            .expect("应创建成功");
        // 把基准时刻设为两分钟前, 让下一次触发落在过去.
        repository
            .mark_fired(row.id, Utc::now() - Duration::minutes(2))
            .await
            .expect("应推进基准时刻");
        let now = Utc::now();

        let dispatched = scheduler.tick(now).await.expect("扫描应成功");

        assert_eq!(dispatched.len(), 1, "到点的登记应派生一条任务");
        let task = task_repository
            .find(dispatched[0])
            .await
            .expect("应查询成功")
            .expect("任务应存在");
        assert_eq!(task.schedule_id, Some(row.id), "任务应保留触发来源");
        assert_eq!(task.kind, TaskKind::Scrape);

        // 基准时刻已推进到当下: 立刻再扫一轮不应重复派生.
        let again = scheduler.tick(now).await.expect("扫描应成功");
        assert!(again.is_empty(), "同一时刻不应触发两次");
    }

    #[tokio::test]
    async fn disabled_schedule_does_not_fire() {
        let (_dir, scheduler, repository, task_repository) = scheduler().await;
        let row = repository
            .create(&spec("* * * * *"))
            .await
            .expect("应创建成功");
        repository
            .update(
                row.id,
                &SchedulePatch {
                    enabled: Some(false),
                },
            )
            .await
            .expect("应停用成功");
        repository
            .mark_fired(row.id, Utc::now() - Duration::minutes(2))
            .await
            .expect("应推进基准时刻");

        let dispatched = scheduler.tick(Utc::now()).await.expect("扫描应成功");

        assert!(dispatched.is_empty(), "停用的登记不应派生任务");
        let count = task_repository
            .count(Default::default())
            .await
            .expect("应统计成功");
        assert_eq!(count, 0);
    }

    #[tokio::test]
    async fn missed_triggers_are_not_replayed() {
        let (_dir, scheduler, repository, task_repository) = scheduler().await;
        // 每小时一次, 基准时刻设为三小时前: 错过的三次不逐个重放.
        let row = repository
            .create(&spec("0 * * * *"))
            .await
            .expect("应创建成功");
        repository
            .mark_fired(row.id, Utc::now() - Duration::hours(3))
            .await
            .expect("应推进基准时刻");

        let dispatched = scheduler.tick(Utc::now()).await.expect("扫描应成功");

        assert_eq!(dispatched.len(), 1, "错过的多次触发只补最近一次");
        let total = task_repository
            .count(Default::default())
            .await
            .expect("应统计成功");
        assert_eq!(total, 1);
    }
}
