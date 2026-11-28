//! 同一路径的文件变更队列。
//!
//! 与 ref `core/tools/file-mutation-queue.ts:14-54` 对齐：先用 realpath 统一键，
//! 不存在的新文件使用解析后的路径；不同文件仍可并行。注册表由本进程所有，条目使用
//! `Weak`，最后一次使用者离开后自动移除。注册表锁不会跨 await 持有。

use std::collections::HashMap;
use std::future::Future;
use std::path::{Path, PathBuf};
use std::sync::{Arc, OnceLock, Weak};

use thiserror::Error;
use tokio::sync::Mutex;

/// 解析文件变更队列键失败。
#[derive(Debug, Error)]
pub enum FileMutationQueueError {
    /// 目标路径无法解析，且不是“尚不存在”。
    #[error("could not resolve file mutation path {path}: {source}")]
    Resolve {
        /// 调用方给出的路径。
        path: PathBuf,
        /// 底层文件系统错误。
        #[source]
        source: std::io::Error,
    },
}

type Queue = Mutex<()>;
type Registry = HashMap<PathBuf, Weak<Queue>>;
static REGISTRY: OnceLock<Mutex<Registry>> = OnceLock::new();

/// 串行执行针对同一文件的变更；不同路径的操作保持并行。
///
/// 队列只在本进程内生效，与官方同进程扩展宿主的语义一致。取消由调用方的操作负责，
/// 队列锁直到操作 future 完成才释放，避免文件系统调用仍在进行时提前放开后续写者。
pub async fn with_file_mutation_queue<T, F, Fut>(
    path: &Path,
    operation: F,
) -> Result<T, FileMutationQueueError>
where
    F: FnOnce() -> Fut,
    Fut: Future<Output = T>,
{
    let key = match tokio::fs::canonicalize(path).await {
        Ok(path) => path,
        Err(error)
            if matches!(
                error.kind(),
                std::io::ErrorKind::NotFound | std::io::ErrorKind::NotADirectory
            ) =>
        {
            path.to_path_buf()
        }
        Err(source) => {
            return Err(FileMutationQueueError::Resolve {
                path: path.to_path_buf(),
                source,
            });
        }
    };

    let queue = {
        let mut registry = REGISTRY
            .get_or_init(|| Mutex::new(HashMap::new()))
            .lock()
            .await;
        if let Some(queue) = registry.get(&key).and_then(Weak::upgrade) {
            queue
        } else {
            let queue = Arc::new(Queue::new(()));
            registry.insert(key.clone(), Arc::downgrade(&queue));
            queue
        }
    };

    let guard = queue.lock().await;
    let result = operation().await;
    drop(guard);

    let mut registry = REGISTRY
        .get_or_init(|| Mutex::new(HashMap::new()))
        .lock()
        .await;
    if Arc::strong_count(&queue) == 1 {
        registry.remove(&key);
    }
    Ok(result)
}
