use chrono::Utc;
use thiserror::Error;

use super::model::{Task, TaskSpec, TaskStatus};
use crate::db::Database;

#[derive(Debug, Error)]
pub enum TaskRepositoryError {
    #[error("数据库操作失败: {0}")]
    Database(#[from] sqlx::Error),
    #[error("任务 {0} 不存在")]
    NotFound(i64),
    #[error("任务 {0} 不在可重试状态; 只有失败与已取消的任务可以重试")]
    NotRetryable(i64),
    #[error("任务 {0} 不在运行状态, 不能写入终态")]
    NotRunning(i64),
}

/// 任务查询条件.
#[derive(Debug, Clone, Copy, Default)]
pub struct TaskFilter {
    pub status: Option<TaskStatus>,
}

/// 任务仓储. 写操作经由写池, 出队用带条件的更新语句, 不需要额外的互斥.
#[derive(Debug, Clone)]
pub struct TaskRepository {
    database: Database,
}

impl TaskRepository {
    pub fn new(database: Database) -> Self {
        Self { database }
    }

    pub async fn enqueue(&self, spec: &TaskSpec) -> Result<Task, TaskRepositoryError> {
        let payload = serde_json::to_string(&spec.payload).unwrap_or_else(|_| "null".to_owned());
        let row = sqlx::query_as::<_, Task>(
            r#"
            INSERT INTO task (kind, payload, status, priority, created_at)
            VALUES (?1, ?2, 'queued', ?3, ?4)
            RETURNING *
            "#,
        )
        .bind(spec.kind)
        .bind(payload)
        .bind(spec.priority)
        .bind(Utc::now())
        .fetch_one(self.database.write())
        .await?;

        Ok(row)
    }

    /// 领取一条排队中的任务. 没有可领取的任务时返回 `None`.
    pub async fn claim_next(&self) -> Result<Option<Task>, TaskRepositoryError> {
        let row = sqlx::query_as::<_, Task>(
            r#"
            UPDATE task
            SET status = 'running', started_at = ?1, attempts = attempts + 1
            WHERE id = (
                SELECT id FROM task WHERE status = 'queued'
                ORDER BY priority DESC, id ASC LIMIT 1
            )
            RETURNING *
            "#,
        )
        .bind(Utc::now())
        .fetch_optional(self.database.write())
        .await?;

        Ok(row)
    }

    /// 标记完成, 并在同一事务内写入后继任务.
    ///
    /// 「任务完成」与「后继任务入队」必须同时成立, 否则会出现完成但后继丢失的窗口.
    pub async fn complete(
        &self,
        id: i64,
        followups: &[TaskSpec],
    ) -> Result<Vec<Task>, TaskRepositoryError> {
        let mut transaction = self.database.write().begin().await?;

        let updated = sqlx::query(
            "UPDATE task SET status = 'done', finished_at = ?1 WHERE id = ?2 AND status = 'running'",
        )
        .bind(Utc::now())
        .bind(id)
        .execute(&mut *transaction)
        .await?
        .rows_affected();
        if updated != 1 {
            transaction.rollback().await?;
            return Err(TaskRepositoryError::NotRunning(id));
        }

        let mut children = Vec::with_capacity(followups.len());
        for spec in followups {
            let payload =
                serde_json::to_string(&spec.payload).unwrap_or_else(|_| "null".to_owned());
            let child = sqlx::query_as::<_, Task>(
                r#"
                INSERT INTO task (kind, payload, status, priority, created_at)
                VALUES (?1, ?2, 'queued', ?3, ?4)
                RETURNING *
                "#,
            )
            .bind(spec.kind)
            .bind(payload)
            .bind(spec.priority)
            .bind(Utc::now())
            .fetch_one(&mut *transaction)
            .await?;

            sqlx::query("INSERT INTO task_link (parent_id, child_id) VALUES (?1, ?2)")
                .bind(id)
                .bind(child.id)
                .execute(&mut *transaction)
                .await?;

            children.push(child);
        }

        transaction.commit().await?;
        Ok(children)
    }

    pub async fn fail(&self, id: i64, error: &str) -> Result<(), TaskRepositoryError> {
        let updated = sqlx::query(
            r#"
            UPDATE task SET status = 'failed', error = ?1, finished_at = ?2
            WHERE id = ?3 AND status = 'running'
            "#,
        )
        .bind(error)
        .bind(Utc::now())
        .bind(id)
        .execute(self.database.write())
        .await?
        .rows_affected();
        if updated != 1 {
            return Err(TaskRepositoryError::NotRunning(id));
        }

        Ok(())
    }

    pub async fn mark_cancelled(&self, id: i64) -> Result<(), TaskRepositoryError> {
        sqlx::query(
            "UPDATE task SET status = 'cancelled', finished_at = ?1 WHERE id = ?2 AND status IN ('queued', 'running')",
        )
        .bind(Utc::now())
        .bind(id)
        .execute(self.database.write())
        .await?;

        Ok(())
    }

    /// 排队中的任务直接标记取消; 返回 `true` 表示由本方法完成, `false` 表示任务正在运行.
    pub async fn cancel_queued(&self, id: i64) -> Result<bool, TaskRepositoryError> {
        let affected = sqlx::query(
            "UPDATE task SET status = 'cancelled', finished_at = ?1 WHERE id = ?2 AND status = 'queued'",
        )
        .bind(Utc::now())
        .bind(id)
        .execute(self.database.write())
        .await?
        .rows_affected();

        Ok(affected == 1)
    }

    /// 复制一条终态任务为新任务, 保留与原任务的关联.
    pub async fn retry(&self, id: i64) -> Result<Task, TaskRepositoryError> {
        let original = self
            .find(id)
            .await?
            .ok_or(TaskRepositoryError::NotFound(id))?;
        if !original.status.is_retryable() {
            return Err(TaskRepositoryError::NotRetryable(id));
        }

        let row = sqlx::query_as::<_, Task>(
            r#"
            INSERT INTO task (kind, payload, status, priority, retry_of, created_at)
            VALUES (?1, ?2, 'queued', ?3, ?4, ?5)
            RETURNING *
            "#,
        )
        .bind(original.kind)
        .bind(original.payload)
        .bind(original.priority)
        .bind(original.id)
        .bind(Utc::now())
        .fetch_one(self.database.write())
        .await?;

        Ok(row)
    }

    /// 后继任务. 顺序按入队先后.
    pub async fn children(&self, parent_id: i64) -> Result<Vec<Task>, TaskRepositoryError> {
        let rows = sqlx::query_as::<_, Task>(
            r#"
            SELECT task.* FROM task
            JOIN task_link ON task_link.child_id = task.id
            WHERE task_link.parent_id = ?1
            ORDER BY task.id ASC
            "#,
        )
        .bind(parent_id)
        .fetch_all(self.database.read())
        .await?;

        Ok(rows)
    }

    pub async fn find(&self, id: i64) -> Result<Option<Task>, TaskRepositoryError> {
        let row = sqlx::query_as::<_, Task>("SELECT * FROM task WHERE id = ?1")
            .bind(id)
            .fetch_optional(self.database.read())
            .await?;

        Ok(row)
    }

    pub async fn list(
        &self,
        filter: TaskFilter,
        limit: u32,
        offset: u32,
    ) -> Result<Vec<Task>, TaskRepositoryError> {
        // 参数按位置绑定, 带过滤与不带过滤的占位符编号必须各自成体系.
        let rows = match filter.status {
            Some(status) => {
                sqlx::query_as::<_, Task>(
                    "SELECT * FROM task WHERE status = ?1 ORDER BY id DESC LIMIT ?2 OFFSET ?3",
                )
                .bind(status)
                .bind(i64::from(limit))
                .bind(i64::from(offset))
                .fetch_all(self.database.read())
                .await?
            }
            None => {
                sqlx::query_as::<_, Task>("SELECT * FROM task ORDER BY id DESC LIMIT ?1 OFFSET ?2")
                    .bind(i64::from(limit))
                    .bind(i64::from(offset))
                    .fetch_all(self.database.read())
                    .await?
            }
        };

        Ok(rows)
    }

    pub async fn count(&self, filter: TaskFilter) -> Result<i64, TaskRepositoryError> {
        let total = match filter.status {
            Some(status) => {
                sqlx::query_scalar::<_, i64>("SELECT count(*) FROM task WHERE status = ?1")
                    .bind(status)
                    .fetch_one(self.database.read())
                    .await?
            }
            None => {
                sqlx::query_scalar::<_, i64>("SELECT count(*) FROM task")
                    .fetch_one(self.database.read())
                    .await?
            }
        };

        Ok(total)
    }

    /// 把上次进程遗留的运行中任务放回队列; 进程重启后由启动流程调用.
    pub async fn recover_running(&self) -> Result<u64, TaskRepositoryError> {
        let affected = sqlx::query(
            "UPDATE task SET status = 'queued', started_at = NULL WHERE status = 'running'",
        )
        .execute(self.database.write())
        .await?
        .rows_affected();

        Ok(affected)
    }
}

#[cfg(test)]
mod tests {
    use super::{TaskFilter, TaskRepository};
    use crate::db::Database;
    use crate::task::{TaskKind, TaskSpec, TaskStatus};

    async fn repository() -> (tempfile::TempDir, TaskRepository) {
        let dir = tempfile::tempdir().expect("应创建临时目录");
        let database = Database::open(&dir.path().join("amane.db"))
            .await
            .expect("应打开数据库");
        (dir, TaskRepository::new(database))
    }

    fn spec(number: &str) -> TaskSpec {
        TaskSpec::new(TaskKind::Scrape, serde_json::json!({ "number": number }))
    }

    #[tokio::test]
    async fn claim_takes_highest_priority_first() {
        let (_dir, repository) = repository().await;
        let low = repository.enqueue(&spec("LOW-001")).await.expect("应入队");
        let high = repository
            .enqueue(&TaskSpec {
                priority: 5,
                ..spec("HIGH-001")
            })
            .await
            .expect("应入队");

        let first = repository.claim_next().await.expect("应领取成功");
        let second = repository.claim_next().await.expect("应领取成功");
        let third = repository.claim_next().await.expect("应领取成功");

        assert_eq!(first.map(|task| task.id), Some(high.id));
        assert_eq!(second.map(|task| task.id), Some(low.id));
        assert!(third.is_none(), "队列空时应返回 None");
    }

    #[tokio::test]
    async fn claim_marks_running_and_counts_attempt() {
        let (_dir, repository) = repository().await;
        repository.enqueue(&spec("ABC-001")).await.expect("应入队");

        let task = repository
            .claim_next()
            .await
            .expect("应领取成功")
            .expect("应有任务");

        assert_eq!(task.status, TaskStatus::Running);
        assert_eq!(task.attempts, 1);
        assert!(task.started_at.is_some());
    }

    #[tokio::test]
    async fn complete_writes_followups_in_same_transaction() {
        let (_dir, repository) = repository().await;
        let parent = repository.enqueue(&spec("ABC-001")).await.expect("应入队");
        repository.claim_next().await.expect("应领取成功");

        let children = repository
            .complete(
                parent.id,
                &[TaskSpec::new(
                    TaskKind::Organize,
                    serde_json::json!({ "number": "ABC-001" }),
                )],
            )
            .await
            .expect("应完成");

        assert_eq!(children.len(), 1);
        assert_eq!(children[0].kind, TaskKind::Organize);
        assert_eq!(children[0].status, TaskStatus::Queued);
        let stored = repository
            .find(parent.id)
            .await
            .expect("应查询成功")
            .expect("应存在");
        assert_eq!(stored.status, TaskStatus::Done);
    }

    #[tokio::test]
    async fn retry_copies_payload_and_links_original() {
        let (_dir, repository) = repository().await;
        let original = repository.enqueue(&spec("ABC-001")).await.expect("应入队");
        repository
            .claim_next()
            .await
            .expect("应领取成功")
            .expect("应有任务");
        repository
            .fail(original.id, "上游超时")
            .await
            .expect("应失败");

        let retried = repository.retry(original.id).await.expect("应重试成功");

        assert_eq!(retried.retry_of, Some(original.id));
        assert_eq!(retried.payload, original.payload);
        assert_eq!(retried.status, TaskStatus::Queued);
        assert_eq!(retried.attempts, 0);
    }

    #[tokio::test]
    async fn retry_rejects_unfinished_task() {
        let (_dir, repository) = repository().await;
        let task = repository.enqueue(&spec("ABC-001")).await.expect("应入队");

        let result = repository.retry(task.id).await;

        assert!(
            matches!(result, Err(super::TaskRepositoryError::NotRetryable(id)) if id == task.id),
            "未终结的任务不能重试"
        );
    }

    #[tokio::test]
    async fn terminal_writes_require_running_state() {
        let (_dir, repository) = repository().await;
        let task = repository.enqueue(&spec("ABC-001")).await.expect("应入队");

        let completed = repository.complete(task.id, &[]).await;
        let failed = repository.fail(task.id, "不应写入").await;

        assert!(
            matches!(completed, Err(super::TaskRepositoryError::NotRunning(id)) if id == task.id)
        );
        assert!(matches!(failed, Err(super::TaskRepositoryError::NotRunning(id)) if id == task.id));
        let stored = repository
            .find(task.id)
            .await
            .expect("应查询成功")
            .expect("应存在");
        assert_eq!(
            stored.status,
            TaskStatus::Queued,
            "排队中的任务不应被写入终态"
        );
    }

    #[tokio::test]
    async fn retry_rejects_completed_task() {
        let (_dir, repository) = repository().await;
        let task = repository.enqueue(&spec("ABC-001")).await.expect("应入队");
        repository.claim_next().await.expect("应领取成功");
        repository.complete(task.id, &[]).await.expect("应完成");

        let result = repository.retry(task.id).await;

        assert!(
            matches!(result, Err(super::TaskRepositoryError::NotRetryable(id)) if id == task.id),
            "已完成的任务不能重试"
        );
    }

    #[tokio::test]
    async fn cancel_queued_only_affects_queued() {
        let (_dir, repository) = repository().await;
        let task = repository.enqueue(&spec("ABC-001")).await.expect("应入队");
        repository.claim_next().await.expect("应领取成功");

        let cancelled = repository.cancel_queued(task.id).await.expect("应取消成功");

        assert!(!cancelled, "运行中的任务不由排队取消处理");
    }

    #[tokio::test]
    async fn list_filters_by_status_and_counts() {
        let (_dir, repository) = repository().await;
        repository.enqueue(&spec("AAA-001")).await.expect("应入队");
        repository.enqueue(&spec("BBB-001")).await.expect("应入队");
        let claimed = repository
            .claim_next()
            .await
            .expect("应领取成功")
            .expect("应有任务");
        repository.fail(claimed.id, "失败").await.expect("应失败");

        let failed = repository
            .list(
                TaskFilter {
                    status: Some(TaskStatus::Failed),
                },
                10,
                0,
            )
            .await
            .expect("应列出");
        let all = repository
            .count(TaskFilter::default())
            .await
            .expect("应统计");

        assert_eq!(failed.len(), 1);
        assert_eq!(failed[0].id, claimed.id);
        assert_eq!(all, 2);
    }

    #[tokio::test]
    async fn lists_without_filter() {
        let (_dir, repository) = repository().await;
        repository.enqueue(&spec("AAA-001")).await.expect("应入队");
        repository.enqueue(&spec("BBB-001")).await.expect("应入队");

        let rows = repository
            .list(TaskFilter::default(), 10, 0)
            .await
            .expect("应列出");

        assert_eq!(rows.len(), 2, "不带过滤时应返回全部任务");
    }

    #[tokio::test]
    async fn recover_returns_running_tasks_to_queue() {
        let (_dir, repository) = repository().await;
        repository.enqueue(&spec("ABC-001")).await.expect("应入队");
        repository.claim_next().await.expect("应领取成功");

        let recovered = repository.recover_running().await.expect("应恢复");

        assert_eq!(recovered, 1);
        let task = repository
            .claim_next()
            .await
            .expect("应领取成功")
            .expect("应有任务");
        assert_eq!(task.attempts, 2, "恢复后再次领取会累加尝试次数");
    }
}
