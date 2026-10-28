use std::io;
use std::path::{Path, PathBuf};

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use tokio::fs::OpenOptions;
use tokio::io::AsyncWriteExt;

/// 任务日志的一行.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TaskLogLine {
    pub at: DateTime<Utc>,
    pub level: String,
    pub message: String,
}

/// 日志文件名. 同目录下的其它文件不得占用它.
const LOG_FILE: &str = "log.jsonl";

/// 单个任务的日志写入器.
///
/// 每个任务一个目录, 逐行追加 JSON, 进程中断后已写入的行仍然可读.
#[derive(Debug, Clone)]
pub struct TaskLogger {
    directory: PathBuf,
    path: PathBuf,
}

impl TaskLogger {
    /// 在 `root` 下创建 `task-{id}/log.jsonl` 并返回写入器.
    pub async fn open(root: &Path, task_id: i64) -> io::Result<Self> {
        let directory = root.join(format!("task-{task_id}"));
        tokio::fs::create_dir_all(&directory).await?;

        Ok(Self {
            path: directory.join(LOG_FILE),
            directory,
        })
    }

    /// 日志目录所在位置, 供读取端定位.
    pub fn directory(root: &Path, task_id: i64) -> PathBuf {
        root.join(format!("task-{task_id}"))
    }

    pub async fn info(&self, message: impl Into<String>) {
        self.write("info", message.into()).await;
    }

    pub async fn warn(&self, message: impl Into<String>) {
        self.write("warn", message.into()).await;
    }

    pub async fn error(&self, message: impl Into<String>) {
        self.write("error", message.into()).await;
    }

    /// 追加一行.
    ///
    /// 写入失败只上报给进程日志, 不让任务失败: 排查信息的缺失不应改变任务的终态.
    async fn write(&self, level: &str, message: String) {
        let line = TaskLogLine {
            at: Utc::now(),
            level: level.to_owned(),
            message,
        };
        let Ok(mut encoded) = serde_json::to_string(&line) else {
            tracing::warn!(path = %self.path.display(), "任务日志序列化失败");
            return;
        };
        encoded.push('\n');

        match OpenOptions::new()
            .create(true)
            .append(true)
            .open(&self.path)
            .await
        {
            Ok(mut file) => {
                if let Err(error) = file.write_all(encoded.as_bytes()).await {
                    tracing::warn!(path = %self.path.display(), %error, "写入任务日志失败");
                }
            }
            Err(error) => {
                tracing::warn!(path = %self.path.display(), %error, "打开任务日志失败");
            }
        }
    }

    /// 把一段原始内容写入该任务的日志目录.
    ///
    /// `name` 只能是单层文件名, 且不得占用日志本身: 原文来自外部站点,
    /// 文件名一旦包含路径分隔符, 写入位置就会超出任务目录.
    pub async fn write_file(&self, name: &str, content: &str) -> io::Result<()> {
        let plain = !name.is_empty()
            && name != "."
            && name != ".."
            && name != LOG_FILE
            && !name.contains('/')
            && !name.contains('\\');
        if !plain {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                format!("文件名不合法: {name}"),
            ));
        }

        tokio::fs::write(self.directory.join(name), content).await
    }

    /// 读取该任务已写入的全部日志行; 目录不存在时返回空.
    pub async fn read(root: &Path, task_id: i64) -> io::Result<Vec<TaskLogLine>> {
        let path = Self::directory(root, task_id).join(LOG_FILE);
        let content = match tokio::fs::read_to_string(&path).await {
            Ok(content) => content,
            Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(Vec::new()),
            Err(error) => return Err(error),
        };

        Ok(content
            .lines()
            .filter(|line| !line.trim().is_empty())
            .filter_map(|line| match serde_json::from_str::<TaskLogLine>(line) {
                Ok(entry) => Some(entry),
                Err(error) => {
                    tracing::warn!(path = %path.display(), %error, "跳过无法解析的任务日志行");
                    None
                }
            })
            .collect())
    }
}

#[cfg(test)]
mod tests {
    use super::TaskLogger;

    #[tokio::test]
    async fn appends_and_reads_lines_in_order() {
        let dir = tempfile::tempdir().expect("应创建临时目录");
        let logger = TaskLogger::open(dir.path(), 7).await.expect("应创建日志");

        logger.info("开始").await;
        logger.warn("来源超时").await;
        logger.error("结束").await;

        let lines = TaskLogger::read(dir.path(), 7).await.expect("应读取成功");
        let messages: Vec<&str> = lines.iter().map(|line| line.message.as_str()).collect();
        assert_eq!(messages, ["开始", "来源超时", "结束"]);
        assert_eq!(lines[1].level, "warn");
    }

    #[tokio::test]
    async fn writes_raw_beside_log() {
        let dir = tempfile::tempdir().expect("应创建临时目录");
        let logger = TaskLogger::open(dir.path(), 3).await.expect("应创建日志");

        logger
            .write_file("javdb.raw.html", "<html>原文</html>")
            .await
            .expect("应写入成功");

        let path = TaskLogger::directory(dir.path(), 3).join("javdb.raw.html");
        let content = tokio::fs::read_to_string(&path).await.expect("应读到原文");
        assert_eq!(content, "<html>原文</html>");
    }

    #[tokio::test]
    async fn rejects_names_that_escape_the_task_directory() {
        let dir = tempfile::tempdir().expect("应创建临时目录");
        let logger = TaskLogger::open(dir.path(), 4).await.expect("应创建日志");

        for name in ["../escape.txt", "nested/raw.txt", "", ".", "log.jsonl"] {
            assert!(
                logger.write_file(name, "x").await.is_err(),
                "{name} 不应被接受"
            );
        }
        assert!(
            !dir.path().join("escape.txt").exists(),
            "被拒绝的文件名不应出现在目录外"
        );
    }

    #[tokio::test]
    async fn reading_absent_task_returns_empty() {
        let dir = tempfile::tempdir().expect("应创建临时目录");

        let lines = TaskLogger::read(dir.path(), 99).await.expect("应读取成功");

        assert!(lines.is_empty());
    }
}
