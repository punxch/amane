//! SQLite 连接池.
//!
//! SQLite 同一时刻只允许一个写者, 因此写池固定为单连接, 读池按可用并行度开.
//! 两个池指向同一个库文件, 靠 WAL 让读不阻塞写.

use std::path::Path;
use std::str::FromStr;

use sqlx::SqlitePool;
use sqlx::sqlite::{SqliteConnectOptions, SqliteJournalMode, SqlitePoolOptions, SqliteSynchronous};
use thiserror::Error;

/// 读池连接数上限.
const READ_POOL_SIZE: u32 = 4;

#[derive(Debug, Error)]
pub enum DbError {
    #[error("创建数据目录失败: {0}")]
    Io(#[from] std::io::Error),
    #[error("打开数据库失败: {0}")]
    Open(#[from] sqlx::Error),
    #[error("执行迁移失败: {0}")]
    Migrate(#[from] sqlx::migrate::MigrateError),
}

/// 数据库句柄. 克隆只复制连接池引用.
#[derive(Debug, Clone)]
pub struct Database {
    write: SqlitePool,
    read: SqlitePool,
}

impl Database {
    /// 打开位于 `path` 的库文件, 必要时创建, 并执行未应用的迁移.
    pub async fn open(path: &Path) -> Result<Self, DbError> {
        let options = SqliteConnectOptions::from_str(&format!("sqlite://{}", path.display()))
            .map_err(DbError::Open)?
            .create_if_missing(true)
            .journal_mode(SqliteJournalMode::Wal)
            .synchronous(SqliteSynchronous::Normal)
            .foreign_keys(true);

        let write = SqlitePoolOptions::new()
            .max_connections(1)
            .min_connections(1)
            .connect_with(options.clone())
            .await?;
        let read = SqlitePoolOptions::new()
            .max_connections(READ_POOL_SIZE)
            .connect_with(options)
            .await?;

        let database = Self { write, read };
        database.migrate().await?;
        Ok(database)
    }

    async fn migrate(&self) -> Result<(), DbError> {
        // 重建表的迁移要先 DROP 旧表: 外键开着时 DROP 会连带删掉子表的行.
        // 只能在事务之外关 —— sqlx 的 SQLite 迁移总是包在事务里, 而事务内的
        // `PRAGMA foreign_keys` 是空操作. 写池固定单连接, 关掉的正是跑迁移的那条.
        sqlx::query("PRAGMA foreign_keys = OFF")
            .execute(&self.write)
            .await?;
        let outcome = sqlx::migrate!("../../migrations").run(&self.write).await;
        sqlx::query("PRAGMA foreign_keys = ON")
            .execute(&self.write)
            .await?;

        outcome?;
        Ok(())
    }

    /// 写连接池; 只有一个连接, 调用方不需要额外串行化.
    pub fn write(&self) -> &SqlitePool {
        &self.write
    }

    /// 读连接池.
    pub fn read(&self) -> &SqlitePool {
        &self.read
    }

    pub async fn close(&self) {
        self.read.close().await;
        self.write.close().await;
    }
}

#[cfg(test)]
mod tests {
    use super::Database;

    #[tokio::test]
    async fn migrations_apply_once() {
        let dir = tempfile::tempdir().expect("应创建临时目录");
        let path = dir.path().join("amane.db");

        let first = Database::open(&path).await.expect("首次打开应成功");
        let count: i64 = sqlx::query_scalar("SELECT count(*) FROM metadata")
            .fetch_one(first.read())
            .await
            .expect("迁移后应存在 metadata 表");
        assert_eq!(count, 0);
        first.close().await;

        let second = Database::open(&path).await.expect("再次打开应成功");
        let after: i64 = sqlx::query_scalar("SELECT count(*) FROM metadata")
            .fetch_one(second.read())
            .await
            .expect("重复迁移不应破坏表结构");
        assert_eq!(after, 0);
        second.close().await;
    }
}
