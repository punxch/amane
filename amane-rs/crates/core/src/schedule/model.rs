use chrono::{DateTime, Utc};
use chrono_tz::Tz;
use croner::Cron;
use serde::Serialize;
use thiserror::Error;

use crate::task::TaskKind;

/// 登记一条定时任务所需的内容.
#[derive(Debug, Clone)]
pub struct ScheduleSpec {
    pub kind: TaskKind,
    pub payload: serde_json::Value,
    pub cron: String,
    pub timezone: String,
}

/// 修改登记时允许改动的部分, 目前只有启停.
///
/// 类型、载荷与表达式创建后不变: 需要变更时删除后重新登记,
/// 否则一次不经意的修改会派生出没人预期的任务.
#[derive(Debug, Clone, Default)]
pub struct SchedulePatch {
    pub enabled: Option<bool>,
}

/// cron 与时区的组合, 负责把「下一个触发时刻」算出来.
#[derive(Debug, Clone)]
pub struct Pattern {
    cron: Cron,
    timezone: Tz,
}

impl Pattern {
    /// 解析五段式 cron 与 IANA 时区名. 解析失败的原因会原样带出.
    pub fn parse(cron: &str, timezone: &str) -> Result<Self, ScheduleError> {
        let cron = cron.trim();
        let timezone = timezone.trim();
        if cron.is_empty() {
            return Err(ScheduleError::InvalidCron("表达式不能为空".to_owned()));
        }
        let parsed = Cron::new(cron)
            .with_seconds_optional()
            .parse()
            .map_err(|error| ScheduleError::InvalidCron(error.to_string()))?;
        let zone = timezone.parse::<Tz>().map_err(|_| {
            ScheduleError::InvalidTimezone(if timezone.is_empty() {
                "时区不能为空".to_owned()
            } else {
                format!("不认识的时区名: {timezone}")
            })
        })?;

        Ok(Self {
            cron: parsed,
            timezone: zone,
        })
    }

    /// `after` 之后的下一次触发时刻 (UTC).
    pub fn next_after(&self, after: DateTime<Utc>) -> Result<DateTime<Utc>, ScheduleError> {
        let local = after.with_timezone(&self.timezone);
        self.cron
            .find_next_occurrence(&local, false)
            .map(|next| next.with_timezone(&Utc))
            .map_err(|error| ScheduleError::InvalidCron(error.to_string()))
    }
}

/// 一条定时任务.
#[derive(Debug, Clone, Serialize, sqlx::FromRow)]
pub struct Schedule {
    pub id: i64,
    pub kind: TaskKind,
    /// 派生任务时使用的载荷, JSON 文本.
    pub payload: String,
    pub cron: String,
    /// IANA 时区名, 例如 `Asia/Shanghai`. 触发时刻按它计算, 与进程时区无关.
    pub timezone: String,
    pub enabled: bool,
    /// 上次派发的时刻. 也是调度循环的基准时刻: 下一次触发从它之后起算.
    pub last_fired_at: Option<DateTime<Utc>>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

#[derive(Debug, Error)]
pub enum ScheduleError {
    #[error("cron 表达式不合法: {0}")]
    InvalidCron(String),
    #[error("时区不合法: {0}")]
    InvalidTimezone(String),
    #[error("任务载荷不合法: {0}")]
    InvalidPayload(String),
    #[error("定时任务 {0} 不存在")]
    NotFound(i64),
    #[error("派生任务失败: {0}")]
    Submit(String),
    #[error("数据库操作失败: {0}")]
    Database(#[from] sqlx::Error),
}
