//! 删除番号条目: 库内事务与文件清理在同一个接口后面.

use std::sync::Arc;

use async_trait::async_trait;

use super::{MetadataRepository, RepositoryError};
use crate::number::MediaNumber;
use crate::resource::Resource;

/// 资源文件的清理口.
///
/// 文件系统不属于内核: 内核只声明「这里需要一个能删文件的东西」, 生产接资源库,
/// 测试接内存实现.
#[async_trait]
pub trait ResourceFiles: Send + Sync {
    async fn remove(&self, resource: &Resource) -> std::io::Result<()>;
}

/// 删除番号条目与它独占的资源.
pub struct MetadataRemoval {
    metadata: MetadataRepository,
    files: Arc<dyn ResourceFiles>,
}

impl MetadataRemoval {
    pub fn new(metadata: MetadataRepository, files: Arc<dyn ResourceFiles>) -> Self {
        Self { metadata, files }
    }

    /// 删除条目.
    ///
    /// 数据库先行, 文件随后: 反过来会让条目指向不存在的文件, 而这个顺序最坏只留下一个
    /// 无主文件. 文件删除失败只告警 —— 数据库已经一致, 无主文件不影响读取.
    pub async fn delete(&self, number: &MediaNumber) -> Result<(), RepositoryError> {
        let orphans = self.metadata.delete(number).await?;

        for resource in &orphans {
            if let Err(error) = self.files.remove(resource).await {
                tracing::warn!(resource = %resource.file_name, %error, "删除资源文件失败");
            }
        }

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use std::sync::{Arc, Mutex};

    use async_trait::async_trait;

    use super::{MetadataRemoval, ResourceFiles};
    use crate::db::Database;
    use crate::metadata::{MetadataInput, MetadataRepository, RepositoryError};
    use crate::number::MediaNumber;
    use crate::resource::{Resource, ResourceRepository};

    /// 内存实现: 只记下被要求删掉的文件名, 可以让删除失败.
    #[derive(Default)]
    struct FakeFiles {
        removed: Mutex<Vec<String>>,
        fail: bool,
    }

    #[async_trait]
    impl ResourceFiles for FakeFiles {
        async fn remove(&self, resource: &Resource) -> std::io::Result<()> {
            if self.fail {
                return Err(std::io::Error::other("磁盘只读"));
            }
            self.removed
                .lock()
                .expect("应加锁")
                .push(resource.file_name.clone());
            Ok(())
        }
    }

    async fn fixture(
        fail: bool,
    ) -> (
        tempfile::TempDir,
        MetadataRemoval,
        MetadataRepository,
        ResourceRepository,
        Arc<FakeFiles>,
    ) {
        let dir = tempfile::tempdir().expect("应创建临时目录");
        let database = Database::open(&dir.path().join("amane.db"))
            .await
            .expect("应打开数据库");
        let metadata = MetadataRepository::new(database.clone());
        let resources = ResourceRepository::new(database);
        let files = Arc::new(FakeFiles {
            removed: Mutex::new(Vec::new()),
            fail,
        });
        let removal = MetadataRemoval::new(metadata.clone(), Arc::clone(&files) as _);

        (dir, removal, metadata, resources, files)
    }

    #[tokio::test]
    async fn deletes_entry_and_its_resource_files() {
        let (_dir, removal, metadata, resources, files) = fixture(false).await;
        let number = MediaNumber::parse("ABC-001").expect("应解析成功");
        let row = metadata
            .upsert(&number, &MetadataInput::default())
            .await
            .expect("应写入成功");
        let poster = resources
            .reserve("https://example.invalid/p.jpg", "image/jpeg", 1)
            .await
            .expect("应登记成功");
        resources
            .link_poster(row.id, Some(poster.id))
            .await
            .expect("应关联成功");
        resources
            .set_file_name(poster.id, "1.jpg")
            .await
            .expect("应写入文件名");

        removal.delete(&number).await.expect("应删除成功");

        assert!(
            metadata.find(&number).await.expect("应查询成功").is_none(),
            "条目应已删除"
        );
        assert_eq!(
            *files.removed.lock().expect("应加锁"),
            vec!["1.jpg".to_owned()],
            "独占的资源文件应交给清理口"
        );
    }

    #[tokio::test]
    async fn file_failure_leaves_database_consistent() {
        let (_dir, removal, metadata, resources, _files) = fixture(true).await;
        let number = MediaNumber::parse("ABC-002").expect("应解析成功");
        let row = metadata
            .upsert(&number, &MetadataInput::default())
            .await
            .expect("应写入成功");
        let poster = resources
            .reserve("https://example.invalid/p.jpg", "image/jpeg", 1)
            .await
            .expect("应登记成功");
        resources
            .link_poster(row.id, Some(poster.id))
            .await
            .expect("应关联成功");

        removal
            .delete(&number)
            .await
            .expect("文件删不掉不该让删除失败");

        assert!(
            metadata.find(&number).await.expect("应查询成功").is_none(),
            "数据库仍应是一致状态"
        );
    }

    #[tokio::test]
    async fn absent_entry_is_not_found() {
        let (_dir, removal, _metadata, _resources, _files) = fixture(false).await;
        let number = MediaNumber::parse("ZZZ-999").expect("应解析成功");

        let result = removal.delete(&number).await;

        assert!(matches!(result, Err(RepositoryError::NotFound)));
    }
}
