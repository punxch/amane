//! 整理登记的持久化: 预览判「同名不同番号」的依据, 执行切片负责写入.

use chrono::Utc;

use super::model::OrganizePlan;
use crate::db::Database;

/// 整理登记仓储错误.
#[derive(Debug, thiserror::Error)]
pub enum OrganizeError {
    #[error("数据库操作失败: {0}")]
    Database(#[from] sqlx::Error),
    #[error("路径 {0} 已被占用")]
    PathTaken(String),
}

/// 整理登记的仓储. 写入发生在执行切片; 预览只读.
#[derive(Debug, Clone)]
pub struct OrganizeRepository {
    database: Database,
}

impl OrganizeRepository {
    pub fn new(database: Database) -> Self {
        Self { database }
    }

    /// 登记一次整理的目标. 同一库内同一路径只允许一个番号占用.
    pub async fn register(
        &self,
        root_id: i64,
        path: &str,
        number: &str,
        source_path: &str,
    ) -> Result<OrganizePlan, OrganizeError> {
        let row = sqlx::query_as::<_, OrganizePlan>(
            "INSERT INTO organize_plan (root_id, path, number, source_path, created_at) \
             VALUES (?1, ?2, ?3, ?4, ?5) RETURNING *",
        )
        .bind(root_id)
        .bind(path)
        .bind(number)
        .bind(source_path)
        .bind(Utc::now())
        .fetch_one(self.database.write())
        .await
        .map_err(|error| match &error {
            sqlx::Error::Database(database) if database.is_unique_violation() => {
                OrganizeError::PathTaken(path.to_owned())
            }
            _ => OrganizeError::Database(error),
        })?;

        Ok(row)
    }

    /// 目标路径当前的占用者; 空表示可用.
    pub async fn find_by_path(
        &self,
        root_id: i64,
        path: &str,
    ) -> Result<Option<OrganizePlan>, OrganizeError> {
        let row = sqlx::query_as::<_, OrganizePlan>(
            "SELECT * FROM organize_plan WHERE root_id = ?1 AND path = ?2",
        )
        .bind(root_id)
        .bind(path)
        .fetch_optional(self.database.read())
        .await?;

        Ok(row)
    }

    /// 一个库的全部登记; 预览批量判定时一次取回, 避免逐条查询.
    pub async fn list_for_root(&self, root_id: i64) -> Result<Vec<OrganizePlan>, OrganizeError> {
        let rows = sqlx::query_as::<_, OrganizePlan>(
            "SELECT * FROM organize_plan WHERE root_id = ?1 ORDER BY id ASC",
        )
        .bind(root_id)
        .fetch_all(self.database.read())
        .await?;

        Ok(rows)
    }

    /// 移除登记; 执行切片在文件状态变化导致放弃时使用.
    pub async fn delete(&self, id: i64) -> Result<(), OrganizeError> {
        sqlx::query("DELETE FROM organize_plan WHERE id = ?1")
            .bind(id)
            .execute(self.database.write())
            .await?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::{OrganizeError, OrganizeRepository};
    use crate::db::Database;
    use crate::library::LibraryRepository;

    /// 两个真实的库根; organize_plan 外键指向 library_root, 登记必须有目标路径.
    async fn repository() -> (tempfile::TempDir, OrganizeRepository, i64, i64) {
        let dir = tempfile::tempdir().expect("应创建临时目录");
        let database = Database::open(&dir.path().join("amane.db"))
            .await
            .expect("应打开数据库");
        let libraries = LibraryRepository::new(database.clone());
        let first = libraries
            .create(std::path::Path::new("/tmp/amane-library-a"))
            .await
            .expect("应登记根目录");
        let second = libraries
            .create(std::path::Path::new("/tmp/amane-library-b"))
            .await
            .expect("应登记根目录");
        (dir, OrganizeRepository::new(database), first.id, second.id)
    }

    #[tokio::test]
    async fn registers_and_finds_by_path() {
        let (_dir, repository, root_id, _other) = repository().await;
        repository
            .register(
                root_id,
                "SOD/ABC-123/ABC-123.mp4",
                "ABC-123",
                "downloads/a.mp4",
            )
            .await
            .expect("应登记成功");

        let plan = repository
            .find_by_path(root_id, "SOD/ABC-123/ABC-123.mp4")
            .await
            .expect("应查询成功")
            .expect("应存在");
        assert_eq!(plan.number, "ABC-123");
        assert_eq!(plan.source_path, "downloads/a.mp4");

        assert!(
            repository
                .find_by_path(root_id, "elsewhere.mp4")
                .await
                .expect("应查询成功")
                .is_none()
        );
    }

    #[tokio::test]
    async fn same_path_in_same_root_is_rejected() {
        let (_dir, repository, root_id, _other) = repository().await;
        repository
            .register(root_id, "a/ABC-123.mp4", "ABC-123", "src.mp4")
            .await
            .expect("首次登记应成功");

        let error = repository
            .register(root_id, "a/ABC-123.mp4", "XYZ-999", "other.mp4")
            .await
            .expect_err("同库同路径应被拒绝");
        assert!(
            matches!(error, OrganizeError::PathTaken(_)),
            "应报路径被占用, 实际 {error}"
        );
    }

    #[tokio::test]
    async fn list_for_root_returns_only_that_root() {
        let (_dir, repository, root_id, other_id) = repository().await;
        repository
            .register(root_id, "a/1.mp4", "ABC-123", "s1.mp4")
            .await
            .expect("应登记成功");
        repository
            .register(other_id, "b/2.mp4", "XYZ-999", "s2.mp4")
            .await
            .expect("应登记成功");

        let plans = repository.list_for_root(root_id).await.expect("应查询成功");
        assert_eq!(plans.len(), 1);
        assert_eq!(plans[0].number, "ABC-123");
    }
}
