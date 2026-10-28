use super::model::{Pattern, Schedule, ScheduleError, SchedulePatch, ScheduleSpec};
use crate::db::Database;
use chrono::{DateTime, Utc};

/// 定时任务登记的仓储. 表达式与时区在写入前由 `Pattern` 验证.
#[derive(Clone)]
pub struct ScheduleRepository {
    database: Database,
}

impl ScheduleRepository {
    pub fn new(database: Database) -> Self {
        Self { database }
    }

    /// 登记一条定时任务. 基准时刻取当前时刻, 因此创建后从下一个触发点开始, 不回看历史.
    pub async fn create(&self, spec: &ScheduleSpec) -> Result<Schedule, ScheduleError> {
        Pattern::parse(&spec.cron, &spec.timezone)?;

        let now = Utc::now();
        let row = sqlx::query_as::<_, Schedule>(
            r#"
            INSERT INTO schedule (kind, payload, cron, timezone, enabled, last_fired_at, created_at, updated_at)
            VALUES (?1, ?2, ?3, ?4, 1, ?5, ?5, ?5)
            RETURNING *
            "#,
        )
        .bind(spec.kind)
        .bind(serde_json::to_string(&spec.payload).unwrap_or_else(|_| "null".to_owned()))
        .bind(spec.cron.trim())
        .bind(spec.timezone.trim())
        .bind(now)
        .fetch_one(self.database.write())
        .await?;

        Ok(row)
    }

    pub async fn list(&self) -> Result<Vec<Schedule>, ScheduleError> {
        let rows = sqlx::query_as::<_, Schedule>("SELECT * FROM schedule ORDER BY id ASC")
            .fetch_all(self.database.read())
            .await?;

        Ok(rows)
    }

    /// 启用的登记, 供调度循环使用.
    pub async fn list_enabled(&self) -> Result<Vec<Schedule>, ScheduleError> {
        let rows = sqlx::query_as::<_, Schedule>(
            "SELECT * FROM schedule WHERE enabled = 1 ORDER BY id ASC",
        )
        .fetch_all(self.database.read())
        .await?;

        Ok(rows)
    }

    pub async fn find(&self, id: i64) -> Result<Option<Schedule>, ScheduleError> {
        let row = sqlx::query_as::<_, Schedule>("SELECT * FROM schedule WHERE id = ?1")
            .bind(id)
            .fetch_optional(self.database.read())
            .await?;

        Ok(row)
    }

    /// 启用或停用. 启停都会把基准时刻设为当下, 因此停用期间错过的触发不会在重新启用后派发.
    pub async fn update(&self, id: i64, patch: &SchedulePatch) -> Result<Schedule, ScheduleError> {
        let now = Utc::now();
        sqlx::query_as::<_, Schedule>(
            r#"
            UPDATE schedule
            SET enabled = COALESCE(?1, enabled),
                last_fired_at = CASE WHEN ?1 IS NOT NULL THEN ?2 ELSE last_fired_at END,
                updated_at = ?2
            WHERE id = ?3
            RETURNING *
            "#,
        )
        .bind(patch.enabled)
        .bind(now)
        .bind(id)
        .fetch_optional(self.database.write())
        .await?
        .ok_or(ScheduleError::NotFound(id))
    }

    pub async fn delete(&self, id: i64) -> Result<(), ScheduleError> {
        let deleted = sqlx::query("DELETE FROM schedule WHERE id = ?1")
            .bind(id)
            .execute(self.database.write())
            .await?
            .rows_affected();
        if deleted == 0 {
            return Err(ScheduleError::NotFound(id));
        }

        Ok(())
    }

    /// 记录一次派发, 并把基准时刻推进到该时刻.
    pub async fn mark_fired(&self, id: i64, at: DateTime<Utc>) -> Result<(), ScheduleError> {
        sqlx::query("UPDATE schedule SET last_fired_at = ?1, updated_at = ?1 WHERE id = ?2")
            .bind(at)
            .bind(id)
            .execute(self.database.write())
            .await?;

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::ScheduleRepository;
    use crate::db::Database;
    use crate::schedule::model::{ScheduleError, ScheduleSpec};
    use crate::task::TaskKind;

    async fn repository() -> (tempfile::TempDir, ScheduleRepository) {
        let dir = tempfile::tempdir().expect("应创建临时目录");
        let database = Database::open(&dir.path().join("amane.db"))
            .await
            .expect("应打开数据库");

        (dir, ScheduleRepository::new(database))
    }

    fn spec(cron: &str, timezone: &str) -> ScheduleSpec {
        ScheduleSpec {
            kind: TaskKind::Scrape,
            payload: serde_json::json!({ "number": "ABC-123" }),
            cron: cron.to_owned(),
            timezone: timezone.to_owned(),
        }
    }

    #[tokio::test]
    async fn create_rejects_malformed_cron() {
        let (_dir, repository) = repository().await;

        for cron in ["", "   ", "99 * * * *", "* * *", "0 3 * * 99"] {
            let result = repository.create(&spec(cron, "Asia/Shanghai")).await;
            let message = match result {
                Err(ScheduleError::InvalidCron(message)) => message,
                other => panic!("表达式 {cron:?} 应被拒绝, 实际 {other:?}"),
            };
            assert!(!message.is_empty(), "拒绝原因应可读");
        }
    }

    #[tokio::test]
    async fn create_rejects_unknown_timezone() {
        let (_dir, repository) = repository().await;

        for timezone in ["Mars/Phobos", "", "GMT+8"] {
            let result = repository.create(&spec("0 3 * * *", timezone)).await;
            let message = match result {
                Err(ScheduleError::InvalidTimezone(message)) => message,
                other => panic!("时区 {timezone:?} 应被拒绝, 实际 {other:?}"),
            };
            assert!(!message.is_empty(), "拒绝原因应可读");
        }
    }

    #[tokio::test]
    async fn create_trims_and_anchors_at_now() {
        let (_dir, repository) = repository().await;

        let row = repository
            .create(&spec("  0 3 * * *  ", "  Asia/Shanghai  "))
            .await
            .expect("应创建成功");

        assert_eq!(row.cron, "0 3 * * *");
        assert_eq!(row.timezone, "Asia/Shanghai");
        assert!(row.enabled);
        assert!(row.last_fired_at.is_some(), "创建时刻就是基准时刻");
    }

    #[tokio::test]
    async fn disable_resets_reference_point() {
        let (_dir, repository) = repository().await;
        let row = repository
            .create(&spec("0 3 * * *", "Asia/Shanghai"))
            .await
            .expect("应创建成功");

        let updated = repository
            .update(
                row.id,
                &crate::schedule::model::SchedulePatch {
                    enabled: Some(false),
                },
            )
            .await
            .expect("应停用成功");

        assert!(!updated.enabled);
        assert_eq!(
            updated.last_fired_at,
            Some(updated.updated_at),
            "停用应把基准时刻设为当下"
        );
    }

    #[tokio::test]
    async fn update_reports_missing_row() {
        let (_dir, repository) = repository().await;

        let result = repository
            .update(
                999,
                &crate::schedule::model::SchedulePatch {
                    enabled: Some(false),
                },
            )
            .await;

        assert!(matches!(result, Err(ScheduleError::NotFound(999))));
    }

    #[tokio::test]
    async fn delete_reports_missing_row() {
        let (_dir, repository) = repository().await;

        let result = repository.delete(999).await;

        assert!(matches!(result, Err(ScheduleError::NotFound(999))));
    }
}
